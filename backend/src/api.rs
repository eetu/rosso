//! The `/api/*` surface. Every handler takes `_: Auth`.

use std::convert::Infallible;

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{sse, IntoResponse, Response, Sse};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_stream::wrappers::BroadcastStream;
// `tokio_stream`'s `filter_map` takes a plain closure; the `futures` one wants
// an async block for a decision that needs no awaiting.
use tokio_stream::{Stream, StreamExt};

use crate::auth::Auth;
use crate::error::{AppError, AppResult};
use crate::extract;
use crate::feed::{self, discover};
use crate::llm;
use crate::settings::{self, Settings, SettingsPatch};
use crate::store::{self, Feed, FeedPatch, Item, ItemDetail, ItemPatch, ItemQuery, Topic};
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/feeds", get(list_feeds).post(add_feed))
        .route("/api/feeds/{id}", patch(update_feed).delete(delete_feed))
        .route("/api/feeds/{id}/refresh", post(refresh_feed))
        .route("/api/opml/import", post(import_opml))
        .route("/api/opml/export", get(export_opml))
        .route("/api/items", get(list_items))
        .route("/api/items/{id}", get(get_item).patch(update_item))
        .route("/api/items/mark-read", post(mark_read))
        .route("/api/stream", get(stream))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/topics", get(list_topics))
}

#[derive(Serialize)]
struct TopicsResponse {
    topics: Vec<Topic>,
}

/// Topics worth grouping by, from the tags the model already assigns.
async fn list_topics(_: Auth, State(state): State<AppState>) -> AppResult<Json<TopicsResponse>> {
    Ok(Json(TopicsResponse {
        topics: store::list_topics(&state.db).await?,
    }))
}

#[derive(Serialize)]
struct SettingsResponse {
    #[serde(flatten)]
    settings: Settings,
    /// Models the host has installed, for the picker. Empty when it is asleep —
    /// the UI falls back to showing the configured name as free text.
    models: Vec<String>,
    llm_available: bool,
}

async fn get_settings(_: Auth, State(state): State<AppState>) -> AppResult<Json<SettingsResponse>> {
    let settings = settings::load(&state.db, &state.cfg).await?;
    let base = state.cfg.ollama_url.as_deref();
    let llm_available = state.llm_health.available(&state.http, base).await;

    let models = match (base, llm_available) {
        (Some(base), true) => llm::ollama::models(&state.http, base)
            .await
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    Ok(Json(SettingsResponse {
        settings,
        models,
        llm_available,
    }))
}

/// Saving an edited interest profile is all it takes to re-score the archive:
/// the fingerprint changes, and every item scored against the old one becomes a
/// candidate. No invalidation pass runs here.
async fn put_settings(
    _: Auth,
    State(state): State<AppState>,
    Json(patch): Json<SettingsPatch>,
) -> AppResult<Json<Settings>> {
    settings::save(&state.db, patch).await?;
    Ok(Json(settings::load(&state.db, &state.cfg).await?))
}

#[derive(Serialize)]
struct FeedsResponse {
    feeds: Vec<Feed>,
}

#[derive(Deserialize)]
struct AddFeedRequest {
    /// Whatever the user typed — a feed URL, a site URL, or a bare hostname.
    url: String,
    folder_id: Option<i64>,
}

#[derive(Serialize)]
struct ItemsResponse {
    items: Vec<Item>,
    /// Pass back as `?cursor=` for the next page; absent on the last page.
    next_cursor: Option<String>,
    /// Which search actually ran. A semantic search with the model host asleep
    /// answers from the text index instead of failing, and says so here rather
    /// than leaving the UI claiming something it did not do.
    mode: Option<&'static str>,
}

#[derive(Deserialize)]
struct MarkReadRequest {
    feed_id: Option<i64>,
}

async fn list_feeds(_: Auth, State(state): State<AppState>) -> AppResult<Json<FeedsResponse>> {
    Ok(Json(FeedsResponse {
        feeds: store::list_feeds(&state.db).await?,
    }))
}

/// Subscribe. Resolves the URL, stores the feed, and keeps the items from that
/// same fetch — so a new feed has content immediately instead of looking broken
/// until the next poll tick.
async fn add_feed(
    _: Auth,
    State(state): State<AppState>,
    Json(req): Json<AddFeedRequest>,
) -> AppResult<Json<Feed>> {
    let found = discover::discover(&state.http, &req.url, state.cfg.max_feed_bytes)
        .await
        .map_err(|e| AppError::BadRequest(format!("could not find a feed: {e}")))?;

    let id = store::insert_feed(
        &state.db,
        found.feed_url.clone(),
        found.parsed.title.clone(),
        found.parsed.site_url.clone(),
        found.parsed.icon.clone(),
        req.folder_id,
    )
    .await?
    .ok_or_else(|| AppError::Conflict(format!("already subscribed to {}", found.feed_url)))?;

    let interval = crate::feed::schedule::DEFAULT_INTERVAL_S;
    let (inserted, _) =
        store::record_success(&state.db, id, found.parsed, None, None, interval).await?;
    tracing::info!(feed_id = id, url = %found.feed_url, inserted, "subscribed");

    store::get_feed(&state.db, id)
        .await?
        .map(Json)
        .ok_or(AppError::NotFound)
}

async fn update_feed(
    _: Auth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(patch): Json<FeedPatch>,
) -> AppResult<Json<Feed>> {
    store::update_feed(&state.db, id, patch).await?;
    store::get_feed(&state.db, id)
        .await?
        .map(Json)
        .ok_or(AppError::NotFound)
}

async fn delete_feed(
    _: Auth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    if !store::delete_feed(&state.db, id).await? {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({ "deleted": id })))
}

/// Poll one feed now, ignoring its schedule.
async fn refresh_feed(
    _: Auth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Feed>> {
    let due = store::feed_by_id_for_poll(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    feed::poll_feed(&state, due).await;
    store::get_feed(&state.db, id)
        .await?
        .map(Json)
        .ok_or(AppError::NotFound)
}

#[derive(Serialize)]
struct ImportResponse {
    added: usize,
    /// Already subscribed. Re-importing the same file is how people check a
    /// migration worked, so this is an ordinary outcome rather than a failure.
    skipped: usize,
}

/// Subscribe to everything in an OPML file.
///
/// The URLs are taken as given — no discovery pass. A file holds hundreds of
/// them, and resolving each one would mean hundreds of outbound requests before
/// the response, against hosts that did nothing to deserve a burst. A URL that
/// turns out not to be a feed simply fails its first poll and says so on the feed
/// row, which is the same path a feed that dies next week takes.
async fn import_opml(
    _: Auth,
    State(state): State<AppState>,
    body: String,
) -> AppResult<Json<ImportResponse>> {
    let outlines = feed::opml::parse(&body);
    if outlines.is_empty() {
        return Err(AppError::BadRequest("no subscriptions in that file".into()));
    }

    let mut added = 0;
    let mut skipped = 0;
    for outline in outlines {
        let inserted = store::insert_feed(
            &state.db,
            outline.xml_url,
            outline.title,
            outline.site_url,
            None,
            None,
        )
        .await?;
        if inserted.is_some() {
            added += 1;
        } else {
            skipped += 1;
        }
    }
    tracing::info!(added, skipped, "opml import");
    Ok(Json(ImportResponse { added, skipped }))
}

/// The subscription list, as a file a browser downloads.
async fn export_opml(_: Auth, State(state): State<AppState>) -> AppResult<Response> {
    let feeds = store::list_feeds(&state.db).await?;
    let xml = feed::opml::render(&feeds, &chrono::Utc::now().to_rfc3339());
    Ok((
        [
            (header::CONTENT_TYPE, "text/x-opml+xml; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"rosso.opml\"",
            ),
        ],
        xml,
    )
        .into_response())
}

/// What the background loops are doing, as server-sent events.
///
/// The stream is a nicety on top of a reader that is already correct without it:
/// every event it carries is something a reload would show anyway. So a client
/// that falls behind is dropped forward to the newest event rather than being
/// waited for, and a dropped connection is the browser's problem — `EventSource`
/// reconnects on its own.
async fn stream(
    _: Auth,
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<sse::Event, Infallible>>> {
    let events = BroadcastStream::new(state.events.subscribe()).filter_map(|event| {
        // `Err` here is `Lagged`: this client missed some. Skipping is right —
        // the events that follow carry the current state, and the counts they
        // move are recomputed server-side rather than accumulated in the tab.
        let event = event.ok()?;
        Some(Ok(sse::Event::default().json_data(event).ok()?))
    });
    // A reader is idle for hours at a time, and every hop in front of this —
    // Traefik, oauth2-proxy — will close a connection that says nothing.
    Sse::new(events).keep_alive(sse::KeepAlive::default())
}

async fn list_items(
    _: Auth,
    State(state): State<AppState>,
    Query(mut query): Query<ItemQuery>,
) -> AppResult<Json<ItemsResponse>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200) as usize;

    // Search by meaning, when asked and when the model host can answer. It falls
    // through to the text index otherwise rather than failing: the reader never
    // depends on the mini being awake, and a search box that stops working
    // because a LAN machine is off would be exactly that dependency.
    let wants_semantic = query.mode.as_deref() == Some("semantic");
    if let (true, Some(text)) = (wants_semantic, query.q.as_deref()) {
        if let Some(vector) = llm::embed::embed_query(&state, text).await {
            let items = store::semantic_search(
                &state.db,
                &state.cfg.embed_model,
                vector,
                query.feed_id,
                limit,
            )
            .await?;
            // Ranked, so there is no cursor — a shortlist, like the interesting
            // view.
            return Ok(Json(ItemsResponse {
                items,
                next_cursor: None,
                mode: Some("semantic"),
            }));
        }
    }

    // A search spans the archive, so it pages the way `all` does whatever view
    // the sidebar happens to have selected.
    let paged = query.q.is_some() || store::supports_cursor(query.view.as_deref());
    // The cutoff is a setting, not something the client gets to choose — two
    // clients disagreeing about what counts as interesting would be worse than
    // one opinion held server-side.
    query.score_threshold = settings::load(&state.db, &state.cfg).await?.score_threshold;
    let searching = query.q.is_some();

    let items = store::list_items(&state.db, query).await?;
    // A short page is the last page; a full one might not be.
    let next_cursor = (paged && items.len() == limit)
        .then(|| store::cursor_for(items.last().expect("non-empty page")));
    Ok(Json(ItemsResponse {
        items,
        next_cursor,
        mode: searching.then_some("text"),
    }))
}

/// Opening an item extracts it if the feed shipped only a teaser and the worker
/// has not reached it yet. Without this the reader shows an empty pane for the
/// item you just clicked while the backlog is drained in published order.
async fn get_item(
    _: Auth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<ItemDetail>> {
    if state.cfg.extract_enabled {
        if let Some(pending) = store::pending_extraction(&state.db, id).await? {
            extract::run_inline(&state, &pending).await;
        }
    }
    store::get_item(&state.db, id)
        .await?
        .map(Json)
        .ok_or(AppError::NotFound)
}

async fn update_item(
    _: Auth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(patch): Json<ItemPatch>,
) -> AppResult<Json<ItemDetail>> {
    store::update_item(&state.db, id, patch).await?;
    store::get_item(&state.db, id)
        .await?
        .map(Json)
        .ok_or(AppError::NotFound)
}

async fn mark_read(
    _: Auth,
    State(state): State<AppState>,
    Json(req): Json<MarkReadRequest>,
) -> AppResult<Json<Value>> {
    let marked = store::mark_read(&state.db, req.feed_id).await?;
    Ok(Json(json!({ "marked": marked })))
}
