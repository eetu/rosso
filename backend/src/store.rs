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
    /// Topic tags the model assigned. Empty when it has not been enriched.
    pub tags: Vec<String>,
    pub read: bool,
    pub starred: bool,
    /// -1, 0 or 1.
    pub feedback: i64,
    /// How many items tell this story, this one included. `1` for everything
    /// that is not in a cluster, so the badge condition is `> 1` rather than a
    /// null check.
    pub cluster_size: i64,
}

/// One of the other reports of the same story.
#[derive(Debug, Serialize)]
pub struct Sibling {
    pub id: i64,
    pub title: String,
    pub url: Option<String>,
    pub feed_title: String,
}

#[derive(Debug, Serialize)]
pub struct ItemDetail {
    #[serde(flatten)]
    pub item: Item,
    pub content_html: Option<String>,
    /// The same story elsewhere. Empty unless this item heads a cluster — which
    /// is what keeps a collapsed list from losing anything: the rows it hid are
    /// listed on the one it kept.
    pub siblings: Vec<Sibling>,
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
    /// Narrow to one topic tag.
    pub tag: Option<String>,
    /// Free text. Searches the whole archive rather than the active view — you
    /// search for something you have already read as often as for something new.
    pub q: Option<String>,
    /// `text` (default) or `semantic`. Only meaningful alongside `q`.
    pub mode: Option<String>,
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
                (SELECT count(*) FROM items i WHERE i.feed_id = f.id AND i.read_at IS NULL \
                        AND (i.cluster_id IS NULL OR i.cluster_id = i.id)) AS unread \
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
                (SELECT count(*) FROM items i WHERE i.feed_id = f.id AND i.read_at IS NULL \
                        AND (i.cluster_id IS NULL OR i.cluster_id = i.id)) AS unread \
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
     i.score_reason, i.tags_json, i.read_at, i.starred_at, i.feedback, \
     CASE WHEN i.cluster_id IS NULL THEN 1 ELSE \
        (SELECT COUNT(*) FROM items s WHERE s.cluster_id = i.cluster_id) END AS cluster_size";

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
        // Stored as a JSON array; a row written before tags existed, or by a
        // model that answered without them, reads as none rather than an error.
        tags: row
            .get::<_, Option<String>>("tags_json")?
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default(),
        read: row.get::<_, Option<String>>("read_at")?.is_some(),
        starred: row.get::<_, Option<String>>("starred_at")?.is_some(),
        feedback: row.get("feedback")?,
        cluster_size: row.get("cluster_size")?,
    })
}

/// What the model wrote on one item, for the live event.
///
/// Read back rather than passed out of the worker: the summary path and the
/// rescore path write different subsets of these columns, and a struct assembled
/// at each write site would be four places to keep in step with the schema.
pub async fn enrichment_of(db: &Db, id: i64) -> rusqlite::Result<Option<crate::events::Event>> {
    db.with(move |c| {
        c.query_row(
            "SELECT summary, score, score_reason, tags_json FROM items WHERE id = ?1",
            params![id],
            |r| {
                Ok(crate::events::Event::ItemEnriched {
                    item_id: id,
                    summary: r.get("summary")?,
                    score: r.get("score")?,
                    score_reason: r.get("score_reason")?,
                    tags: r
                        .get::<_, Option<String>>("tags_json")?
                        .and_then(|raw| serde_json::from_str(&raw).ok())
                        .unwrap_or_default(),
                })
            },
        )
        .optional()
    })
    .await
}

pub async fn list_items(db: &Db, query: ItemQuery) -> rusqlite::Result<Vec<Item>> {
    // A query the tokenizer empties — `?q=***` — is a search that matches
    // nothing, not an absent filter. Deciding that here keeps the SQL below from
    // having to distinguish "no search" from "a search with no terms".
    let search = match query.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        Some(raw) => match fts_query(raw) {
            Some(expr) => Some(expr),
            None => return Ok(Vec::new()),
        },
        None => None,
    };

    db.with(move |c| {
        let limit = query.limit.unwrap_or(50).clamp(1, 200);
        // Searching spans the archive: the view selector picks what is *new* to
        // you, which is the wrong axis once you are looking for a specific thing.
        let view = if search.is_some() {
            Some("all")
        } else {
            query.view.as_deref()
        };
        let view_clause = match view {
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
        let order = if view == Some("interesting") {
            "i.score DESC, COALESCE(i.published_at, i.fetched_at) DESC, i.id DESC"
        } else {
            "COALESCE(i.published_at, i.fetched_at) DESC, i.id DESC"
        };
        // Keyset pagination, never OFFSET: `items` is the table that grows
        // without bound, and OFFSET re-walks every skipped row. The cursor
        // encodes the *time* ordering, so it cannot page a score-ordered view —
        // the interesting view is a shortlist by construction and returns one
        // page with no cursor rather than paging by the wrong key.
        let tag_clause = if query.tag.is_some() {
            "AND EXISTS (SELECT 1 FROM json_each(i.tags_json) WHERE json_each.value = :tag)"
        } else {
            ""
        };
        // The index stores no copy of the text — the subquery returns rowids,
        // and the join back to `items` is what the row is built from.
        let search_clause = if search.is_some() {
            "AND i.id IN (SELECT rowid FROM items_fts WHERE items_fts MATCH :q)"
        } else {
            ""
        };
        // One row per story. A cluster's head carries `cluster_id = id`, so this
        // is a column comparison rather than a correlated subquery per row.
        //
        // Not applied to a search: searching is how you go looking for a
        // specific thing, and a result set that quietly omitted the report you
        // were after because a different outlet ran it first would be a bug you
        // could not see. The list collapses; the search does not.
        let cluster_clause = if search.is_some() {
            ""
        } else {
            "AND (i.cluster_id IS NULL OR i.cluster_id = i.id)"
        };
        let paged = supports_cursor(view);
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
               {tag_clause}
               {search_clause}
               {cluster_clause}
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
        if query.tag.is_some() {
            binds.push((":tag", &query.tag));
        }
        if search.is_some() {
            binds.push((":q", &search));
        }

        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map(&binds[..], item_from_row)?;
        rows.collect()
    })
    .await
}

pub async fn get_item(db: &Db, id: i64) -> rusqlite::Result<Option<ItemDetail>> {
    let Some((item, content_html)) = db
        .with(move |c| {
            let sql = format!(
                "SELECT {ITEM_COLUMNS}, i.content_html
                 FROM items i JOIN feeds f ON f.id = i.feed_id
                 WHERE i.id = ?1"
            );
            c.query_row(&sql, params![id], |r| {
                Ok((
                    item_from_row(r)?,
                    r.get::<_, Option<String>>("content_html")?,
                ))
            })
            .optional()
        })
        .await?
    else {
        return Ok(None);
    };

    // Only ask when there is a cluster to ask about.
    let siblings = if item.cluster_size > 1 {
        cluster_siblings(db, id).await?
    } else {
        Vec::new()
    };
    Ok(Some(ItemDetail {
        item,
        content_html,
        siblings,
    }))
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
/// What the list is showing, for mark-read to act on.
#[derive(Debug, Default, Deserialize)]
pub struct MarkReadScope {
    pub feed_id: Option<i64>,
    pub tag: Option<String>,
    pub q: Option<String>,
}

impl MarkReadScope {
    /// Nothing narrows it: this clears the whole archive. The UI asks first.
    pub fn is_everything(&self) -> bool {
        self.feed_id.is_none()
            && self.tag.as_deref().is_none_or(str::is_empty)
            && self.q.as_deref().is_none_or(str::is_empty)
    }
}

/// Mark read exactly what the list is showing.
///
/// It used to take only `feed_id`, so pressing the button while looking at one
/// topic cleared every unread item in the archive — a destructive action on a
/// selection it could not see. The narrowing now comes from the same three
/// filters the list itself uses.
///
/// Clusters go whole. A story dismissed is dismissed, and leaving the collapsed
/// members unread would be invisible right up until the dedupe threshold changed
/// and they all came back.
pub async fn mark_read(db: &Db, scope: MarkReadScope) -> rusqlite::Result<usize> {
    let search = match scope.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        Some(raw) => match fts_query(raw) {
            Some(expr) => Some(expr),
            // A query matching nothing marks nothing — emphatically not
            // everything, which is what dropping the clause would do.
            None => return Ok(0),
        },
        None => None,
    };

    db.with(move |c| {
        let tag_clause = if scope.tag.is_some() {
            "AND EXISTS (SELECT 1 FROM json_each(i.tags_json) WHERE json_each.value = :tag)"
        } else {
            ""
        };
        let search_clause = if search.is_some() {
            "AND i.id IN (SELECT rowid FROM items_fts WHERE items_fts MATCH :q)"
        } else {
            ""
        };
        let sql = format!(
            "WITH matched AS (
                 SELECT i.id, i.cluster_id FROM items i
                 WHERE i.read_at IS NULL
                   AND (:feed_id IS NULL OR i.feed_id = :feed_id)
                   {tag_clause}
                   {search_clause}
             )
             UPDATE items SET read_at = :now
             WHERE read_at IS NULL
               AND (id IN (SELECT id FROM matched)
                    OR cluster_id IN
                       (SELECT cluster_id FROM matched WHERE cluster_id IS NOT NULL))"
        );

        let now = now_iso();
        let mut binds: Vec<(&str, &dyn rusqlite::ToSql)> =
            vec![(":feed_id", &scope.feed_id), (":now", &now)];
        if scope.tag.is_some() {
            binds.push((":tag", &scope.tag));
        }
        if search.is_some() {
            binds.push((":q", &search));
        }
        c.execute(&sql, &binds[..])
    })
    .await
}

// ------------------------------------------------------------------- digests

/// A candidate for the day's digest, as the model sees it.
#[derive(Debug)]
pub struct DigestCandidate {
    pub id: i64,
    pub title: String,
    pub feed_title: String,
    pub summary: Option<String>,
    pub score: Option<i64>,
}

/// One stored digest, JSON still unparsed — the API parses it, so a digest
/// written by an older shape degrades to an error on one day rather than
/// failing the boot.
#[derive(Debug)]
pub struct StoredDigest {
    pub day: String,
    pub content: String,
    pub item_count: i64,
    pub created_at: String,
}

/// What a digest is built from: the day's best, one per story.
///
/// Read items count. A digest is a record of the day, not a second inbox — and
/// by the time it is generated you have read some of what is in it.
pub async fn digest_candidates(
    db: &Db,
    day: &str,
    limit: u32,
) -> rusqlite::Result<Vec<DigestCandidate>> {
    let day = day.to_string();
    db.with(move |c| {
        let mut stmt = c.prepare(
            "SELECT i.id, i.title,
                    COALESCE(NULLIF(f.custom_title, ''), NULLIF(f.title, ''), f.url) AS feed_title,
                    i.summary, i.score
             FROM items i JOIN feeds f ON f.id = i.feed_id
             WHERE date(COALESCE(i.published_at, i.fetched_at)) = :day
               AND (i.cluster_id IS NULL OR i.cluster_id = i.id)
             ORDER BY i.score DESC NULLS LAST,
                      COALESCE(i.published_at, i.fetched_at) DESC
             LIMIT :limit",
        )?;
        let rows = stmt.query_map(named_params! { ":day": day, ":limit": limit }, |r| {
            Ok(DigestCandidate {
                id: r.get("id")?,
                title: r.get("title")?,
                feed_title: r.get("feed_title")?,
                summary: r.get("summary")?,
                score: r.get("score")?,
            })
        })?;
        rows.collect()
    })
    .await
}

pub async fn record_digest(
    db: &Db,
    day: String,
    content: String,
    model: String,
    item_count: usize,
) -> rusqlite::Result<()> {
    db.with(move |c| {
        c.execute(
            "INSERT INTO digests (day, content, model, item_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(day) DO UPDATE SET
                 content = excluded.content,
                 model = excluded.model,
                 item_count = excluded.item_count,
                 created_at = excluded.created_at",
            params![day, content, model, item_count as i64, now_iso()],
        )?;
        Ok(())
    })
    .await
}

pub async fn get_digest(db: &Db, day: &str) -> rusqlite::Result<Option<StoredDigest>> {
    let day = day.to_string();
    db.with(move |c| {
        c.query_row(
            "SELECT day, content, item_count, created_at FROM digests WHERE day = ?1",
            params![day],
            |r| {
                Ok(StoredDigest {
                    day: r.get("day")?,
                    content: r.get("content")?,
                    item_count: r.get("item_count")?,
                    created_at: r.get("created_at")?,
                })
            },
        )
        .optional()
    })
    .await
}

/// The days that have one, newest first.
pub async fn list_digest_days(db: &Db, limit: u32) -> rusqlite::Result<Vec<String>> {
    db.with(move |c| {
        let mut stmt = c.prepare("SELECT day FROM digests ORDER BY day DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit], |r| r.get(0))?;
        rows.collect()
    })
    .await
}

/// The items a digest refers to, for the reader to link.
pub async fn digest_items(db: &Db, ids: &[i64]) -> rusqlite::Result<Vec<Item>> {
    items_by_id(db, ids).await
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

// -------------------------------------------------------------------- topics

#[derive(Debug, Serialize)]
pub struct Topic {
    pub tag: String,
    pub unread: i64,
}

/// A tag has to appear this many times before it is a topic.
///
/// The tags are free text from the model, so near-duplicates are inevitable —
/// `rust`, `rustlang`, `rust-lang`. A floor is most of the cure without a
/// controlled vocabulary: a one-off variant never reaches it, while a term the
/// corpus really is about does so quickly.
pub const MIN_TOPIC_COUNT: i64 = 3;

/// How many topics the sidebar will show.
const MAX_TOPICS: u32 = 12;

/// Topics across unread items, most waiting first.
///
/// Counts are of *unread* items so the number means the same thing as the one
/// beside each feed — how much is waiting — and drains the same way as it is
/// read.
pub async fn list_topics(db: &Db) -> rusqlite::Result<Vec<Topic>> {
    db.with(|c| {
        let mut stmt = c.prepare(
            "SELECT json_each.value AS tag, count(*) AS unread
             FROM items i, json_each(i.tags_json)
             WHERE i.read_at IS NULL AND i.tags_json IS NOT NULL
               AND (i.cluster_id IS NULL OR i.cluster_id = i.id)
             GROUP BY tag
             HAVING unread >= ?1
             ORDER BY unread DESC, tag
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![MIN_TOPIC_COUNT, MAX_TOPICS], |r| {
            Ok(Topic {
                tag: r.get("tag")?,
                unread: r.get("unread")?,
            })
        })?;
        rows.collect()
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

#[derive(Debug)]
pub struct EmbedCandidate {
    pub id: i64,
    pub title: String,
    pub content_text: Option<String>,
}

/// One stored vector, with the cluster its item already belongs to.
#[derive(Debug)]
pub struct Neighbour {
    pub item_id: i64,
    pub cluster_id: Option<i64>,
    pub embedding: Vec<u8>,
}

/// Items with no usable vector: never embedded, or embedded by a different model.
///
/// Waits on extraction for the same reason enrichment does — a truncated item
/// holds the teaser until the article replaces it, and a vector built from a
/// teaser would cluster on the publisher's boilerplate rather than the story.
pub async fn due_for_embedding(
    db: &Db,
    model: &str,
    limit: u32,
) -> rusqlite::Result<Vec<EmbedCandidate>> {
    let model = model.to_string();
    db.with(move |c| {
        let mut stmt = c.prepare(
            "SELECT i.id, i.title, i.content_text
             FROM items i
             LEFT JOIN item_embeddings e ON e.item_id = i.id
             WHERE i.embed_attempts < :max_attempts
               AND (e.item_id IS NULL OR e.model IS NOT :model)
               AND NOT (i.truncated = 1
                        AND i.extracted = 0
                        AND i.url IS NOT NULL
                        AND i.extract_attempts < :max_extract)
             ORDER BY COALESCE(i.published_at, i.fetched_at) DESC
             LIMIT :limit",
        )?;
        let rows = stmt.query_map(
            named_params! {
                ":max_attempts": crate::llm::embed::MAX_ATTEMPTS,
                ":max_extract": crate::extract::MAX_ATTEMPTS,
                ":model": model,
                ":limit": limit,
            },
            |r| {
                Ok(EmbedCandidate {
                    id: r.get("id")?,
                    title: r.get("title")?,
                    content_text: r.get("content_text")?,
                })
            },
        )?;
        rows.collect()
    })
    .await
}

pub async fn record_embedding(
    db: &Db,
    item_id: i64,
    model: String,
    dims: usize,
    embedding: Vec<u8>,
) -> rusqlite::Result<()> {
    db.with(move |c| {
        c.execute(
            "INSERT INTO item_embeddings (item_id, model, dims, embedding, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(item_id) DO UPDATE SET
                 model = excluded.model,
                 dims = excluded.dims,
                 embedding = excluded.embedding,
                 created_at = excluded.created_at",
            params![item_id, model, dims as i64, embedding, now_iso()],
        )?;
        // Success retires the counter, or three unlucky failures spread over a
        // month would retire a healthy item.
        c.execute(
            "UPDATE items SET embed_attempts = 0, embed_error = NULL WHERE id = ?1",
            params![item_id],
        )?;
        Ok(())
    })
    .await
}

pub async fn record_embed_failure(db: &Db, item_id: i64, error: String) -> rusqlite::Result<()> {
    db.with(move |c| {
        c.execute(
            "UPDATE items SET embed_attempts = embed_attempts + 1, embed_error = ?2 WHERE id = ?1",
            params![item_id, error],
        )?;
        Ok(())
    })
    .await
}

/// Items ranked by how close their vector is to `query`, best first.
///
/// A brute-force scan, which is what this size of archive wants: no index to
/// keep current, no approximate recall, and the whole thing is one sequential
/// read. Memory stays flat because the vectors are compared and dropped one row
/// at a time — only an `(f32, i64)` per item survives the scan, so a 20,000-item
/// archive costs a few hundred kilobytes rather than the sixty megabytes the
/// vectors themselves would.
///
/// Ranked, so there is no cursor: this is a shortlist by construction, the same
/// way the interesting view is.
pub async fn semantic_search(
    db: &Db,
    model: &str,
    query: Vec<f32>,
    feed_id: Option<i64>,
    limit: usize,
) -> rusqlite::Result<Vec<Item>> {
    let model = model.to_string();
    let ranked: Vec<i64> = db
        .with(move |c| {
            let mut stmt = c.prepare(
                "SELECT e.item_id, e.embedding
                 FROM item_embeddings e JOIN items i ON i.id = e.item_id
                 WHERE e.model = :model AND (:feed_id IS NULL OR i.feed_id = :feed_id)",
            )?;
            let mut rows = stmt.query(named_params! { ":model": model, ":feed_id": feed_id })?;

            let mut scored: Vec<(f32, i64)> = Vec::new();
            while let Some(row) = rows.next()? {
                let id: i64 = row.get(0)?;
                let blob: Vec<u8> = row.get(1)?;
                let score = crate::llm::embed::similarity_blob(&query, &blob);
                // Everything is somewhat similar to everything; a floor keeps a
                // search for "sqlite" from also returning the whole archive in
                // descending order of irrelevance.
                if score >= SEMANTIC_FLOOR {
                    scored.push((score, id));
                }
            }
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            Ok(scored.into_iter().take(limit).map(|(_, id)| id).collect())
        })
        .await?;

    items_by_id(db, &ranked).await
}

/// Below this, a match is the vector space being dense rather than the item
/// being relevant. Tuned to be forgiving — a search by meaning that returns
/// nothing looks broken, and the ranking already puts the good ones first.
const SEMANTIC_FLOOR: f32 = 0.55;

/// Fetch items by id, preserving the order of `ids`.
///
/// SQLite returns rows in whatever order it likes, and the order here *is* the
/// answer — losing it would turn a ranking back into a list.
async fn items_by_id(db: &Db, ids: &[i64]) -> rusqlite::Result<Vec<Item>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids = ids.to_vec();
    db.with(move |c| {
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!(
            "SELECT {ITEM_COLUMNS}
             FROM items i JOIN feeds f ON f.id = i.feed_id
             WHERE i.id IN ({placeholders})"
        );
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(ids.iter()), item_from_row)?;
        let mut found: Vec<Item> = rows.collect::<rusqlite::Result<_>>()?;
        found.sort_by_key(|item| {
            ids.iter()
                .position(|id| *id == item.id)
                .unwrap_or(usize::MAX)
        });
        Ok(found)
    })
    .await
}

pub async fn embedding_of(db: &Db, item_id: i64) -> rusqlite::Result<Option<Vec<u8>>> {
    db.with(move |c| {
        c.query_row(
            "SELECT embedding FROM item_embeddings WHERE item_id = ?1",
            params![item_id],
            |r| r.get(0),
        )
        .optional()
    })
    .await
}

/// Vectors an item could be a duplicate of: same model, published recently, not
/// the item itself.
///
/// Bounded by time rather than by count because that is what the question is —
/// the same story reported by five outlets arrives within hours, and a match
/// against something from March is a topic, not a duplicate.
pub async fn neighbours_within(
    db: &Db,
    model: &str,
    hours: i64,
    exclude: i64,
) -> rusqlite::Result<Vec<Neighbour>> {
    let model = model.to_string();
    db.with(move |c| {
        let mut stmt = c.prepare(
            "SELECT e.item_id, i.cluster_id, e.embedding
             FROM item_embeddings e JOIN items i ON i.id = e.item_id
             WHERE e.model = :model
               AND e.item_id != :exclude
               AND COALESCE(i.published_at, i.fetched_at) >= :since",
        )?;
        let since = (chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();
        let rows = stmt.query_map(
            named_params! { ":model": model, ":exclude": exclude, ":since": since },
            |r| {
                Ok(Neighbour {
                    item_id: r.get("item_id")?,
                    cluster_id: r.get("cluster_id")?,
                    embedding: r.get("embedding")?,
                })
            },
        )?;
        rows.collect()
    })
    .await
}

/// Put `item_id` in `head_id`'s cluster.
///
/// A cluster is identified by its head's own id, so the head carries
/// `cluster_id = id`. That is what lets the list filter to one row per cluster
/// with a column comparison instead of a correlated subquery per row.
pub async fn join_cluster(db: &Db, item_id: i64, head_id: i64) -> rusqlite::Result<()> {
    db.with(move |c| {
        let tx = c.unchecked_transaction()?;
        // The head may not have been in a cluster before this second item made
        // it one.
        tx.execute(
            "UPDATE items SET cluster_id = id WHERE id = ?1 AND cluster_id IS NULL",
            params![head_id],
        )?;
        tx.execute(
            "UPDATE items SET cluster_id = ?2 WHERE id = ?1",
            params![item_id, head_id],
        )?;
        tx.commit()
    })
    .await
}

/// The other items telling the same story, oldest first — the reader's
/// "also covered by".
pub async fn cluster_siblings(db: &Db, item_id: i64) -> rusqlite::Result<Vec<Sibling>> {
    db.with(move |c| {
        let mut stmt = c.prepare(
            "SELECT s.id, s.title, s.url,
                    COALESCE(NULLIF(f.custom_title, ''), NULLIF(f.title, ''), f.url) AS feed_title
             FROM items s JOIN feeds f ON f.id = s.feed_id
             WHERE s.cluster_id = (SELECT cluster_id FROM items WHERE id = ?1)
               AND s.id != ?1
             ORDER BY COALESCE(s.published_at, s.fetched_at) ASC",
        )?;
        let rows = stmt.query_map(params![item_id], |r| {
            Ok(Sibling {
                id: r.get("id")?,
                title: r.get("title")?,
                url: r.get("url")?,
                feed_title: r.get("feed_title")?,
            })
        })?;
        rows.collect()
    })
    .await
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

/// What the user typed, as an FTS5 `MATCH` expression.
///
/// Raw input cannot go into `MATCH`. An unbalanced quote, a leading `-`, or the
/// bare word `NEAR` is a *syntax error*, not a fruitless search, and a search box
/// that 500s on an apostrophe is worse than one that finds nothing. So the string
/// is tokenized here and every token re-emitted quoted, which demotes FTS5's
/// operators to ordinary words. The last token also takes a `*`, so the list
/// narrows while you are still typing rather than only once you stop.
///
/// Returns `None` when nothing survives — the caller treats that as a search that
/// matched nothing rather than as no search at all.
fn fts_query(raw: &str) -> Option<String> {
    let tokens: Vec<&str> = raw
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let (last, rest) = tokens.split_last()?;
    let mut expr = String::new();
    for token in rest {
        expr.push_str(&format!("\"{token}\" "));
    }
    expr.push_str(&format!("\"{last}\"*"));
    Some(expr)
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

    #[test]
    fn search_terms_are_quoted_and_the_last_one_is_a_prefix() {
        assert_eq!(fts_query("rust async").unwrap(), "\"rust\" \"async\"*");
        assert_eq!(fts_query("borrow").unwrap(), "\"borrow\"*");
        // Punctuation is a separator, not syntax — none of it reaches FTS5.
        assert_eq!(fts_query("o'brien").unwrap(), "\"o\" \"brien\"*");
        assert_eq!(fts_query("  spaced   out ").unwrap(), "\"spaced\" \"out\"*");
        // FTS5 operators are demoted to words by the quoting.
        assert_eq!(fts_query("NEAR OR -x").unwrap(), "\"NEAR\" \"OR\" \"x\"*");
        assert_eq!(fts_query("***"), None);
        assert_eq!(fts_query(""), None);
    }

    /// The query has to survive FTS5's parser, which unit-testing the string
    /// cannot show — only SQLite can say whether it is valid syntax.
    #[tokio::test]
    async fn punctuation_does_not_break_the_match() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Db::open(&dir.path().join("rosso.db")).unwrap();
        for raw in ["o'brien", "NEAR OR -x", "c++ \"quoted", "rust async"] {
            let expr = fts_query(raw).unwrap();
            db.with(move |c| {
                c.query_row(
                    "SELECT count(*) FROM items_fts WHERE items_fts MATCH ?1",
                    params![expr],
                    |r| r.get::<_, i64>(0),
                )
            })
            .await
            .unwrap_or_else(|e| panic!("{raw:?} is not a valid MATCH expression: {e}"));
        }
    }
}
