//! Every SQL statement in rosso lives here.
//!
//! The wire types are the same structs the SPA's `api.ts` mirrors by hand, so a
//! field rename here is a frontend change too.

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{named_params, params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::feed::parse::{ParsedFeed, ParsedItem};
use crate::feed::schedule::{next_interval, PollResult};

pub fn now_iso() -> String {
    iso(Utc::now())
}

pub fn iso(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

// ---------------------------------------------------------------- wire types

#[derive(Debug, Serialize)]
pub struct Feed {
    pub id: i64,
    pub url: String,
    pub site_url: Option<String>,
    /// The name to show: the user's override when set, else the feed's own.
    pub title: String,
    pub folder_id: Option<i64>,
    pub icon: Option<String>,
    pub unread: i64,
    pub last_fetch_at: Option<String>,
    pub next_fetch_at: String,
    pub last_error: Option<String>,
    pub disabled: bool,
}

#[derive(Debug, Serialize)]
pub struct Item {
    pub id: i64,
    pub feed_id: i64,
    pub feed_title: String,
    pub url: Option<String>,
    /// Present when the discussion lives somewhere other than the article, as on
    /// an aggregator feed.
    pub comments_url: Option<String>,
    pub title: String,
    pub author: Option<String>,
    pub published_at: Option<String>,
    /// LLM-derived and therefore null whenever the model host has not got to it.
    pub summary: Option<String>,
    pub score: Option<i64>,
    /// Why the model gave that score — shown on hover, not in the row.
    pub score_reason: Option<String>,
    pub read: bool,
    pub starred: bool,
    /// -1, 0 or 1.
    pub feedback: i64,
}

#[derive(Debug, Serialize)]
pub struct ItemDetail {
    #[serde(flatten)]
    pub item: Item,
    pub content_html: Option<String>,
}

/// The columns the poller needs, and nothing else.
#[derive(Debug)]
pub struct DueFeed {
    pub id: i64,
    pub url: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub interval_s: u64,
    pub failures: u32,
}

#[derive(Debug, Default, Deserialize)]
pub struct ItemQuery {
    /// `unread` (default), `starred`, `interesting`, `all`.
    pub view: Option<String>,
    pub feed_id: Option<i64>,
    pub limit: Option<u32>,
    /// `published_at|id` of the last row of the previous page.
    pub cursor: Option<String>,
    /// Score cutoff for the `interesting` view; comes from settings.
    #[serde(skip)]
    pub score_threshold: i64,
}

#[derive(Debug, Default, Deserialize)]
pub struct FeedPatch {
    pub custom_title: Option<String>,
    pub folder_id: Option<i64>,
    pub disabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ItemPatch {
    pub read: Option<bool>,
    pub starred: Option<bool>,
    /// -1, 0 or 1. Feeds back into the scoring prompt as calibration.
    pub feedback: Option<i64>,
}

// --------------------------------------------------------------------- feeds

const FEED_COLUMNS: &str = "f.id, f.url, f.site_url, \
     COALESCE(NULLIF(f.custom_title, ''), NULLIF(f.title, ''), f.url) AS shown_title, \
     f.folder_id, f.icon, f.last_fetch_at, f.next_fetch_at, f.last_error, f.disabled";

fn feed_from_row(row: &Row) -> rusqlite::Result<Feed> {
    Ok(Feed {
        id: row.get("id")?,
        url: row.get("url")?,
        site_url: row.get("site_url")?,
        title: row.get("shown_title")?,
        folder_id: row.get("folder_id")?,
        icon: row.get("icon")?,
        unread: row.get("unread")?,
        last_fetch_at: row.get("last_fetch_at")?,
        next_fetch_at: row.get("next_fetch_at")?,
        last_error: row.get("last_error")?,
        disabled: row.get::<_, i64>("disabled")? != 0,
    })
}

pub async fn list_feeds(db: &Db) -> rusqlite::Result<Vec<Feed>> {
    db.with(|c| {
        let sql = format!(
            "SELECT {FEED_COLUMNS}, \
                (SELECT count(*) FROM items i WHERE i.feed_id = f.id AND i.read_at IS NULL) AS unread \
             FROM feeds f ORDER BY shown_title COLLATE NOCASE"
        );
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map([], feed_from_row)?;
        rows.collect()
    })
    .await
}

pub async fn get_feed(db: &Db, id: i64) -> rusqlite::Result<Option<Feed>> {
    db.with(move |c| {
        let sql = format!(
            "SELECT {FEED_COLUMNS}, \
                (SELECT count(*) FROM items i WHERE i.feed_id = f.id AND i.read_at IS NULL) AS unread \
             FROM feeds f WHERE f.id = ?1"
        );
        c.query_row(&sql, params![id], feed_from_row).optional()
    })
    .await
}

/// Insert a feed, due immediately. Returns `None` when the URL is already
/// subscribed — a duplicate is a user mistake, not an error worth a 500.
pub async fn insert_feed(
    db: &Db,
    url: String,
    parsed_title: Option<String>,
    site_url: Option<String>,
    icon: Option<String>,
    folder_id: Option<i64>,
) -> rusqlite::Result<Option<i64>> {
    db.with(move |c| {
        let now = now_iso();
        let changed = c.execute(
            "INSERT OR IGNORE INTO feeds
                (url, title, site_url, icon, folder_id, next_fetch_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![
                url,
                parsed_title.unwrap_or_default(),
                site_url,
                icon,
                folder_id,
                now
            ],
        )?;
        Ok(if changed == 0 {
            None
        } else {
            Some(c.last_insert_rowid())
        })
    })
    .await
}

pub async fn update_feed(db: &Db, id: i64, patch: FeedPatch) -> rusqlite::Result<bool> {
    db.with(move |c| {
        let mut changed = 0;
        if let Some(title) = patch.custom_title {
            changed += c.execute(
                "UPDATE feeds SET custom_title = NULLIF(?2, '') WHERE id = ?1",
                params![id, title],
            )?;
        }
        if let Some(folder_id) = patch.folder_id {
            changed += c.execute(
                "UPDATE feeds SET folder_id = ?2 WHERE id = ?1",
                params![id, folder_id],
            )?;
        }
        if let Some(disabled) = patch.disabled {
            changed += c.execute(
                "UPDATE feeds SET disabled = ?2 WHERE id = ?1",
                params![id, i64::from(disabled)],
            )?;
        }
        Ok(changed > 0)
    })
    .await
}

pub async fn delete_feed(db: &Db, id: i64) -> rusqlite::Result<bool> {
    db.with(move |c| Ok(c.execute("DELETE FROM feeds WHERE id = ?1", params![id])? > 0))
        .await
}

pub async fn due_feeds(db: &Db, limit: u32) -> rusqlite::Result<Vec<DueFeed>> {
    db.with(move |c| {
        let mut stmt = c.prepare(
            "SELECT id, url, etag, last_modified, interval_s, failures
             FROM feeds
             WHERE disabled = 0 AND next_fetch_at <= ?1
             ORDER BY next_fetch_at
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![now_iso(), limit], |r| {
            Ok(DueFeed {
                id: r.get(0)?,
                url: r.get(1)?,
                etag: r.get(2)?,
                last_modified: r.get(3)?,
                interval_s: r.get::<_, i64>(4)?.max(0) as u64,
                failures: r.get::<_, i64>(5)?.max(0) as u32,
            })
        })?;
        rows.collect()
    })
    .await
}

pub async fn feed_by_id_for_poll(db: &Db, id: i64) -> rusqlite::Result<Option<DueFeed>> {
    db.with(move |c| {
        c.query_row(
            "SELECT id, url, etag, last_modified, interval_s, failures FROM feeds WHERE id = ?1",
            params![id],
            |r| {
                Ok(DueFeed {
                    id: r.get(0)?,
                    url: r.get(1)?,
                    etag: r.get(2)?,
                    last_modified: r.get(3)?,
                    interval_s: r.get::<_, i64>(4)?.max(0) as u64,
                    failures: r.get::<_, i64>(5)?.max(0) as u32,
                })
            },
        )
        .optional()
    })
    .await
}

/// Store a successful poll: feed metadata, validators, schedule, and any items
/// we had not seen. Returns how many items were new and the interval chosen.
///
/// One transaction, so a crash mid-insert cannot leave the feed marked fetched
/// with its items missing. The next interval is computed *inside* it because it
/// depends on whether the insert found anything new.
pub async fn record_success(
    db: &Db,
    feed_id: i64,
    parsed: ParsedFeed,
    etag: Option<String>,
    last_modified: Option<String>,
    current_interval_s: u64,
) -> rusqlite::Result<(usize, u64)> {
    db.with(move |c| {
        let tx = c.unchecked_transaction()?;
        let inserted = insert_items_tx(&tx, feed_id, &parsed.items)?;
        let result = if inserted > 0 {
            PollResult::NewItems
        } else {
            PollResult::Unchanged
        };
        let next_interval_s = next_interval(current_interval_s, result, 0, parsed.ttl_minutes);
        let next_fetch_at = schedule_at(next_interval_s);

        tx.execute(
            "UPDATE feeds SET
                title = COALESCE(NULLIF(?2, ''), title),
                site_url = COALESCE(?3, site_url),
                icon = COALESCE(?4, icon),
                etag = ?5,
                last_modified = ?6,
                last_fetch_at = ?7,
                next_fetch_at = ?8,
                interval_s = ?9,
                failures = 0,
                last_error = NULL
             WHERE id = ?1",
            params![
                feed_id,
                parsed.title.clone().unwrap_or_default(),
                parsed.site_url,
                parsed.icon,
                etag,
                last_modified,
                now_iso(),
                next_fetch_at,
                next_interval_s as i64,
            ],
        )?;
        tx.commit()?;
        Ok((inserted, next_interval_s))
    })
    .await
}

/// An ISO timestamp `secs` from now — what `next_fetch_at` is compared against.
pub fn schedule_at(secs: u64) -> String {
    iso(Utc::now() + chrono::Duration::seconds(secs as i64))
}

/// A poll that reached the server but had nothing new (a 304, or a 200 whose
/// entries we already had): reschedule, clear the error, touch nothing else.
pub async fn record_unchanged(db: &Db, feed_id: i64, next_interval_s: u64) -> rusqlite::Result<()> {
    db.with(move |c| {
        let next_fetch_at = schedule_at(next_interval_s);
        c.execute(
            "UPDATE feeds SET last_fetch_at = ?2, next_fetch_at = ?3, interval_s = ?4,
                              failures = 0, last_error = NULL
             WHERE id = ?1",
            params![feed_id, now_iso(), next_fetch_at, next_interval_s as i64],
        )?;
        Ok(())
    })
    .await
}

pub async fn record_failure(
    db: &Db,
    feed_id: i64,
    error: String,
    next_interval_s: u64,
) -> rusqlite::Result<()> {
    db.with(move |c| {
        let next_fetch_at = schedule_at(next_interval_s);
        c.execute(
            "UPDATE feeds SET failures = failures + 1, last_error = ?2, last_fetch_at = ?3,
                              next_fetch_at = ?4, interval_s = ?5
             WHERE id = ?1",
            params![
                feed_id,
                error,
                now_iso(),
                next_fetch_at,
                next_interval_s as i64
            ],
        )?;
        Ok(())
    })
    .await
}

fn insert_items_tx(
    tx: &rusqlite::Transaction<'_>,
    feed_id: i64,
    items: &[ParsedItem],
) -> rusqlite::Result<usize> {
    let mut stmt = tx.prepare(
        "INSERT OR IGNORE INTO items
            (feed_id, guid, url, comments_url, title, author, published_at, fetched_at,
             content_html, content_text, truncated)
         VALUES (:feed_id, :guid, :url, :comments_url, :title, :author, :published_at,
                 :fetched_at, :content_html, :content_text, :truncated)",
    )?;
    let fetched_at = now_iso();
    let mut inserted = 0;
    for item in items {
        inserted += stmt.execute(named_params! {
            ":feed_id": feed_id,
            ":guid": item.guid,
            ":url": item.url,
            ":comments_url": item.comments_url,
            ":title": item.title,
            ":author": item.author,
            ":published_at": item.published_at.map(iso),
            ":fetched_at": fetched_at,
            ":content_html": item.content_html,
            ":content_text": item.content_text,
            ":truncated": i64::from(item.truncated),
        })?;
    }
    Ok(inserted)
}

// --------------------------------------------------------------------- items

const ITEM_COLUMNS: &str = "i.id, i.feed_id, \
     COALESCE(NULLIF(f.custom_title, ''), NULLIF(f.title, ''), f.url) AS feed_title, \
     i.url, i.comments_url, i.title, i.author, i.published_at, i.summary, i.score, \
     i.score_reason, i.read_at, i.starred_at, i.feedback";

fn item_from_row(row: &Row) -> rusqlite::Result<Item> {
    Ok(Item {
        id: row.get("id")?,
        feed_id: row.get("feed_id")?,
        feed_title: row.get("feed_title")?,
        url: row.get("url")?,
        comments_url: row.get("comments_url")?,
        title: row.get("title")?,
        author: row.get("author")?,
        published_at: row.get("published_at")?,
        summary: row.get("summary")?,
        score: row.get("score")?,
        score_reason: row.get("score_reason")?,
        read: row.get::<_, Option<String>>("read_at")?.is_some(),
        starred: row.get::<_, Option<String>>("starred_at")?.is_some(),
        feedback: row.get("feedback")?,
    })
}

pub async fn list_items(db: &Db, query: ItemQuery) -> rusqlite::Result<Vec<Item>> {
    db.with(move |c| {
        let limit = query.limit.unwrap_or(50).clamp(1, 200);
        let view_clause = match query.view.as_deref() {
            Some("starred") => "i.starred_at IS NOT NULL",
            Some("all") => "1",
            // Unread and above the bar. An "interesting" view that kept showing
            // things already read would just be the unread list with extra steps.
            Some("interesting") => "i.read_at IS NULL AND i.score >= :threshold",
            // Unread is the default because it is the one people live in.
            _ => "i.read_at IS NULL",
        };
        // Best first in the interesting view, newest first everywhere else —
        // sorting the unread list by score would shuffle a feed out of order.
        let order = if query.view.as_deref() == Some("interesting") {
            "i.score DESC, COALESCE(i.published_at, i.fetched_at) DESC, i.id DESC"
        } else {
            "COALESCE(i.published_at, i.fetched_at) DESC, i.id DESC"
        };
        // Keyset pagination, never OFFSET: `items` is the table that grows
        // without bound, and OFFSET re-walks every skipped row. The cursor
        // encodes the *time* ordering, so it cannot page a score-ordered view —
        // the interesting view is a shortlist by construction and returns one
        // page with no cursor rather than paging by the wrong key.
        let paged = supports_cursor(query.view.as_deref());
        let (cursor_ts, cursor_id) = if paged {
            split_cursor(query.cursor.as_deref())
        } else {
            (None, None)
        };
        let cursor_clause = if paged {
            "AND (:cursor_ts IS NULL
                  OR (COALESCE(i.published_at, i.fetched_at), i.id) < (:cursor_ts, :cursor_id))"
        } else {
            ""
        };

        let sql = format!(
            "SELECT {ITEM_COLUMNS}
             FROM items i JOIN feeds f ON f.id = i.feed_id
             WHERE {view_clause}
               AND (:feed_id IS NULL OR i.feed_id = :feed_id)
               {cursor_clause}
             ORDER BY {order}
             LIMIT :limit"
        );

        // rusqlite rejects a bound parameter the statement does not mention, so
        // the binds are assembled to match the clauses actually included — the
        // alternative, a no-op `:x IS NULL OR 1` for every unused parameter,
        // puts dead SQL in front of the next reader.
        let mut binds: Vec<(&str, &dyn rusqlite::ToSql)> =
            vec![(":feed_id", &query.feed_id), (":limit", &limit)];
        if paged {
            binds.push((":cursor_ts", &cursor_ts));
            binds.push((":cursor_id", &cursor_id));
        }
        if view_clause.contains(":threshold") {
            binds.push((":threshold", &query.score_threshold));
        }

        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map(&binds[..], item_from_row)?;
        rows.collect()
    })
    .await
}

pub async fn get_item(db: &Db, id: i64) -> rusqlite::Result<Option<ItemDetail>> {
    db.with(move |c| {
        let sql = format!(
            "SELECT {ITEM_COLUMNS}, i.content_html
             FROM items i JOIN feeds f ON f.id = i.feed_id
             WHERE i.id = ?1"
        );
        c.query_row(&sql, params![id], |r| {
            Ok(ItemDetail {
                item: item_from_row(r)?,
                content_html: r.get("content_html")?,
            })
        })
        .optional()
    })
    .await
}

pub async fn update_item(db: &Db, id: i64, patch: ItemPatch) -> rusqlite::Result<bool> {
    db.with(move |c| {
        let now = now_iso();
        let mut changed = 0;
        if let Some(read) = patch.read {
            changed += c.execute(
                "UPDATE items SET read_at = CASE WHEN ?2 = 1 THEN COALESCE(read_at, ?3) END
                 WHERE id = ?1",
                params![id, i64::from(read), now],
            )?;
        }
        if let Some(starred) = patch.starred {
            changed += c.execute(
                "UPDATE items SET starred_at = CASE WHEN ?2 = 1 THEN COALESCE(starred_at, ?3) END
                 WHERE id = ?1",
                params![id, i64::from(starred), now],
            )?;
        }
        if let Some(feedback) = patch.feedback {
            changed += c.execute(
                "UPDATE items SET feedback = ?2 WHERE id = ?1",
                params![id, feedback.clamp(-1, 1)],
            )?;
        }
        Ok(changed > 0)
    })
    .await
}

/// Mark everything read, optionally narrowed to one feed.
pub async fn mark_read(db: &Db, feed_id: Option<i64>) -> rusqlite::Result<usize> {
    db.with(move |c| {
        c.execute(
            "UPDATE items SET read_at = ?1
             WHERE read_at IS NULL AND (?2 IS NULL OR feed_id = ?2)",
            params![now_iso(), feed_id],
        )
    })
    .await
}

// ---------------------------------------------------------------- extraction

/// An item whose feed gave a teaser (or nothing) and that has a page to fetch.
#[derive(Debug)]
pub struct PendingExtraction {
    pub id: i64,
    pub url: String,
}

const PENDING_EXTRACTION_WHERE: &str = "i.url IS NOT NULL \
     AND i.extracted = 0 \
     AND i.truncated = 1 \
     AND i.extract_attempts < :max_attempts";

pub async fn due_for_extraction(db: &Db, limit: u32) -> rusqlite::Result<Vec<PendingExtraction>> {
    db.with(move |c| {
        // Newest first: the backlog that matters is what the reader is about to
        // look at, not what fell off the bottom of the list weeks ago.
        let sql = format!(
            "SELECT i.id, i.url FROM items i
             WHERE {PENDING_EXTRACTION_WHERE}
             ORDER BY COALESCE(i.published_at, i.fetched_at) DESC
             LIMIT :limit"
        );
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map(
            named_params! {
                ":max_attempts": crate::extract::MAX_ATTEMPTS,
                ":limit": limit,
            },
            |r| {
                Ok(PendingExtraction {
                    id: r.get(0)?,
                    url: r.get(1)?,
                })
            },
        )?;
        rows.collect()
    })
    .await
}

/// The same predicate for one item, so the reader can extract on demand what it
/// is about to show.
pub async fn pending_extraction(db: &Db, id: i64) -> rusqlite::Result<Option<PendingExtraction>> {
    db.with(move |c| {
        let sql = format!(
            "SELECT i.id, i.url FROM items i WHERE i.id = :id AND {PENDING_EXTRACTION_WHERE}"
        );
        c.query_row(
            &sql,
            named_params! { ":id": id, ":max_attempts": crate::extract::MAX_ATTEMPTS },
            |r| {
                Ok(PendingExtraction {
                    id: r.get(0)?,
                    url: r.get(1)?,
                })
            },
        )
        .optional()
    })
    .await
}

pub async fn record_extraction(
    db: &Db,
    id: i64,
    html: String,
    text: String,
) -> rusqlite::Result<()> {
    db.with(move |c| {
        c.execute(
            "UPDATE items SET content_html = ?2, content_text = ?3, extracted = 1,
                              extract_error = NULL, extract_attempts = 0
             WHERE id = ?1",
            params![id, html, text],
        )?;
        Ok(())
    })
    .await
}

pub async fn record_extraction_failure(db: &Db, id: i64, error: String) -> rusqlite::Result<()> {
    db.with(move |c| {
        c.execute(
            "UPDATE items SET extract_attempts = extract_attempts + 1, extract_error = ?2
             WHERE id = ?1",
            params![id, error],
        )?;
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------- enrichment

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrichKind {
    /// Never summarized: read the article.
    Full,
    /// Already summarized, but the profile it was scored against has changed.
    /// Only the score needs redoing, from the stored summary.
    Rescore,
}

#[derive(Debug)]
pub struct EnrichCandidate {
    pub id: i64,
    pub kind: EnrichKind,
    pub title: String,
    pub feed_title: String,
    pub content_text: Option<String>,
    pub summary: Option<String>,
}

/// A thumb the reader gave, shown back to the model as calibration.
#[derive(Debug)]
pub struct FeedbackExample {
    pub title: String,
    pub feedback: i64,
}

pub async fn due_for_enrichment(
    db: &Db,
    profile_fingerprint: &str,
    limit: u32,
) -> rusqlite::Result<Vec<EnrichCandidate>> {
    let profile = profile_fingerprint.to_string();
    db.with(move |c| {
        // `IS NOT` rather than `!=` so a NULL `scored_profile` counts as a
        // mismatch; `!=` against NULL is NULL, which would quietly exclude every
        // item that has never been scored.
        //
        // An item with no text is skipped entirely: summarizing a bare headline
        // produces a sentence that says nothing the title did not.
        //
        // The `still_extracting` clause is the ordering rule between the two
        // workers. A truncated item holds the *teaser* until extraction replaces
        // it, so enriching first would summarize the teaser and then never look
        // again — `enriched_at` would already be set. Waiting until extraction
        // has either succeeded or exhausted its attempts is what keeps the two
        // in step without either knowing about the other.
        let mut stmt = c.prepare(
            "SELECT i.id,
                    i.enriched_at IS NULL AS needs_summary,
                    i.title,
                    COALESCE(NULLIF(f.custom_title, ''), NULLIF(f.title, ''), f.url) AS feed_title,
                    i.content_text,
                    i.summary
             FROM items i JOIN feeds f ON f.id = i.feed_id
             WHERE i.enrich_attempts < :max_attempts
               AND (i.enriched_at IS NULL OR i.scored_profile IS NOT :profile)
               AND (i.enriched_at IS NOT NULL
                    OR (i.content_text IS NOT NULL AND length(i.content_text) > 0))
               AND NOT (i.enriched_at IS NULL
                        AND i.truncated = 1
                        AND i.extracted = 0
                        AND i.url IS NOT NULL
                        AND i.extract_attempts < :max_extract)
             ORDER BY COALESCE(i.published_at, i.fetched_at) DESC
             LIMIT :limit",
        )?;
        let rows = stmt.query_map(
            named_params! {
                ":max_attempts": crate::llm::enrich::MAX_ATTEMPTS,
                ":max_extract": crate::extract::MAX_ATTEMPTS,
                ":profile": profile,
                ":limit": limit,
            },
            |r| {
                Ok(EnrichCandidate {
                    id: r.get("id")?,
                    kind: if r.get::<_, i64>("needs_summary")? != 0 {
                        EnrichKind::Full
                    } else {
                        EnrichKind::Rescore
                    },
                    title: r.get("title")?,
                    feed_title: r.get("feed_title")?,
                    content_text: r.get("content_text")?,
                    summary: r.get("summary")?,
                })
            },
        )?;
        rows.collect()
    })
    .await
}

pub async fn feedback_examples(db: &Db, limit: u32) -> rusqlite::Result<Vec<FeedbackExample>> {
    db.with(move |c| {
        let mut stmt = c.prepare(
            "SELECT title, feedback FROM items
             WHERE feedback != 0 AND title != ''
             ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |r| {
            Ok(FeedbackExample {
                title: r.get(0)?,
                feedback: r.get(1)?,
            })
        })?;
        rows.collect()
    })
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn record_enrichment(
    db: &Db,
    id: i64,
    summary: String,
    score: Option<i64>,
    reason: Option<String>,
    tags_json: Option<String>,
    model: String,
    profile_fingerprint: String,
) -> rusqlite::Result<()> {
    db.with(move |c| {
        c.execute(
            // Attempts reset on success: the counter exists to stop retrying
            // something permanently broken, and a success proves it is not. Left
            // cumulative, three unlucky failures over a month would retire an
            // item that works fine.
            "UPDATE items SET summary = ?2, score = ?3, score_reason = ?4, tags_json = ?5,
                              enrich_model = ?6, scored_profile = ?7, enriched_at = ?8,
                              enrich_error = NULL, enrich_attempts = 0
             WHERE id = ?1",
            params![
                id,
                summary,
                score,
                reason,
                tags_json,
                model,
                profile_fingerprint,
                now_iso()
            ],
        )?;
        Ok(())
    })
    .await
}

pub async fn record_rescore(
    db: &Db,
    id: i64,
    score: Option<i64>,
    reason: Option<String>,
    profile_fingerprint: String,
) -> rusqlite::Result<()> {
    db.with(move |c| {
        c.execute(
            "UPDATE items SET score = ?2, score_reason = ?3, scored_profile = ?4,
                              enrich_error = NULL, enrich_attempts = 0
             WHERE id = ?1",
            params![id, score, reason, profile_fingerprint],
        )?;
        Ok(())
    })
    .await
}

pub async fn record_enrich_failure(db: &Db, id: i64, error: String) -> rusqlite::Result<()> {
    db.with(move |c| {
        c.execute(
            "UPDATE items SET enrich_attempts = enrich_attempts + 1, enrich_error = ?2
             WHERE id = ?1",
            params![id, error],
        )?;
        Ok(())
    })
    .await
}

/// Can this view be paged by the time cursor? Only views ordered by time.
pub fn supports_cursor(view: Option<&str>) -> bool {
    view != Some("interesting")
}

/// `"<timestamp>|<id>"`, the sort key of the last row the client saw.
fn split_cursor(cursor: Option<&str>) -> (Option<String>, Option<i64>) {
    let Some((ts, id)) = cursor.and_then(|c| c.split_once('|')) else {
        return (None, None);
    };
    match id.parse::<i64>() {
        Ok(id) => (Some(ts.to_string()), Some(id)),
        Err(_) => (None, None),
    }
}

pub fn cursor_for(item: &Item) -> String {
    format!(
        "{}|{}",
        item.published_at.as_deref().unwrap_or_default(),
        item.id
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_round_trip_and_junk_is_ignored() {
        assert_eq!(
            split_cursor(Some("2026-09-01T10:00:00Z|42")),
            (Some("2026-09-01T10:00:00Z".into()), Some(42))
        );
        // A malformed cursor must degrade to "first page", not to an error or a
        // silently empty result.
        assert_eq!(split_cursor(Some("garbage")), (None, None));
        assert_eq!(split_cursor(Some("2026|notanumber")), (None, None));
        assert_eq!(split_cursor(None), (None, None));
    }
}
