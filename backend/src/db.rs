//! Single-writer SQLite guarded by a tokio mutex.
//!
//! rosso's write rate is a poll tick's worth of inserts plus the odd read/star
//! toggle. One connection is plenty and avoids the lifetime headaches of a pool.
//! Reads go through the same mutex; every `with` closure blocks a runtime worker
//! thread for its duration, so keep the closures short and never await inside one.

use std::path::Path;
use std::sync::Arc;

use rusqlite::Connection;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Db {
    inner: Arc<Mutex<Connection>>,
}

/// Informational marker stamped into `user_version`. Migrations do **not** gate on
/// it: the schema below is declarative and idempotent, so it is applied on every
/// boot and converges to match the code no matter what the version says. (A watch
/// runner like bacon can restart mid-edit and advance the version before the
/// matching DDL is written; re-running the whole batch makes that harmless.)
/// It is read only for genuine one-shot data fixes.
const SCHEMA_VERSION: i64 = 1;

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // The runtime image is `scratch`: no /tmp, no /var/tmp, nothing SQLite's
        // unix VFS can use for a spill file. Left on the default it asks for one
        // as soon as a statement needs to sort or merge — an item insert big
        // enough to spill the FTS index is plenty — and fails the whole
        // transaction with SQLITE_IOERR_GETTEMPPATH (extended code 6410). That
        // never shows up in development, where /tmp always exists.
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        migrate(&conn)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn with<R>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<R>,
    ) -> rusqlite::Result<R> {
        let guard = self.inner.lock().await;
        f(&guard)
    }
}

fn migrate(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(SCHEMA)?;

    // `CREATE TABLE IF NOT EXISTS` does not alter a table that already exists,
    // so a column added after the baseline needs an explicit add-if-missing.
    // These calls stay here forever — they are what upgrades an old database.
    add_column_if_missing(conn, "items", "comments_url", "TEXT")?;
    add_column_if_missing(
        conn,
        "items",
        "extract_attempts",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(conn, "items", "extract_error", "TEXT")?;
    add_column_if_missing(
        conn,
        "items",
        "enrich_attempts",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(conn, "items", "enrich_error", "TEXT")?;
    add_column_if_missing(conn, "items", "scored_profile", "TEXT")?;

    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

/// Idempotent `ALTER TABLE ADD COLUMN` — checks `table_info` first, so it no-ops
/// when the column is already there. Identifiers are hardcoded literals, never
/// user input, so the inline format is safe.
fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    decl: &str,
) -> anyhow::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == column);
    drop(stmt);
    if !exists {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
            [],
        )?;
    }
    Ok(())
}

/// Every statement is idempotent, so the whole batch runs on every boot.
///
/// Timestamps are ISO 8601 TEXT throughout. Never let a column mix storage
/// classes — SQLite's `ORDER BY` sorts INTEGER before TEXT, so one stray integer
/// silently reorders a whole feed.
///
/// The LLM-derived columns on `items` are all nullable and stay NULL when the mini
/// is unreachable. Nothing in the reader path reads them without a fallback.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS folders (
    id         INTEGER PRIMARY KEY,
    title      TEXT NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS feeds (
    id            INTEGER PRIMARY KEY,
    url           TEXT NOT NULL UNIQUE,
    site_url      TEXT,
    title         TEXT NOT NULL DEFAULT '',
    custom_title  TEXT,
    folder_id     INTEGER REFERENCES folders(id) ON DELETE SET NULL,
    icon          TEXT,

    etag          TEXT,
    last_modified TEXT,
    last_fetch_at TEXT,
    next_fetch_at TEXT NOT NULL,
    interval_s    INTEGER NOT NULL DEFAULT 1800,
    failures      INTEGER NOT NULL DEFAULT 0,
    last_error    TEXT,
    disabled      INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_feeds_due ON feeds(next_fetch_at) WHERE disabled = 0;

CREATE TABLE IF NOT EXISTS items (
    id            INTEGER PRIMARY KEY,
    feed_id       INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
    guid          TEXT NOT NULL,
    url           TEXT,
    comments_url  TEXT,
    title         TEXT NOT NULL DEFAULT '',
    author        TEXT,
    published_at  TEXT,
    fetched_at    TEXT NOT NULL,

    content_html  TEXT,
    content_text  TEXT,
    -- `truncated` records what the *feed* shipped (a teaser, or nothing) and
    -- never changes. `extracted` records whether we have since fetched the
    -- article itself. The extraction worker keys off both.
    truncated     INTEGER NOT NULL DEFAULT 0,
    extracted     INTEGER NOT NULL DEFAULT 0,
    extract_attempts INTEGER NOT NULL DEFAULT 0,
    extract_error TEXT,

    summary       TEXT,
    score         INTEGER,
    score_reason  TEXT,
    tags_json     TEXT,
    enriched_at   TEXT,
    enrich_model  TEXT,
    enrich_attempts INTEGER NOT NULL DEFAULT 0,
    enrich_error  TEXT,
    -- Fingerprint of the interest profile this item's score was produced
    -- against. Editing the profile changes the fingerprint, which is what makes
    -- every scored item a rescore candidate without an invalidation pass.
    scored_profile TEXT,
    cluster_id    INTEGER,

    read_at       TEXT,
    starred_at    TEXT,
    feedback      INTEGER NOT NULL DEFAULT 0,

    UNIQUE(feed_id, guid)
);
CREATE INDEX IF NOT EXISTS idx_items_feed_time ON items(feed_id, published_at DESC);
CREATE INDEX IF NOT EXISTS idx_items_unread ON items(read_at, published_at DESC);
CREATE INDEX IF NOT EXISTS idx_items_starred ON items(starred_at DESC) WHERE starred_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_items_score ON items(score DESC, published_at DESC);
CREATE INDEX IF NOT EXISTS idx_items_cluster ON items(cluster_id) WHERE cluster_id IS NOT NULL;

-- External-content FTS: the index stores no copy of the text, it points back at
-- `items` by rowid. The triggers below are what keep it in step.
CREATE VIRTUAL TABLE IF NOT EXISTS items_fts USING fts5(
    title,
    content_text,
    content='items',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS items_fts_insert AFTER INSERT ON items BEGIN
    INSERT INTO items_fts(rowid, title, content_text)
    VALUES (new.id, new.title, new.content_text);
END;

CREATE TRIGGER IF NOT EXISTS items_fts_delete AFTER DELETE ON items BEGIN
    INSERT INTO items_fts(items_fts, rowid, title, content_text)
    VALUES ('delete', old.id, old.title, old.content_text);
END;

CREATE TRIGGER IF NOT EXISTS items_fts_update AFTER UPDATE ON items BEGIN
    INSERT INTO items_fts(items_fts, rowid, title, content_text)
    VALUES ('delete', old.id, old.title, old.content_text);
    INSERT INTO items_fts(rowid, title, content_text)
    VALUES (new.id, new.title, new.content_text);
END;

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn temp_files_are_never_asked_for() {
        // The scratch image has nowhere to put one, so this pragma is what keeps
        // a spilling statement from failing the transaction it is part of.
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("rosso.db")).unwrap();
        let mode: i64 = db
            .with(|c| c.query_row("PRAGMA temp_store", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(mode, 2, "temp_store is not MEMORY");
    }

    #[tokio::test]
    async fn migration_is_idempotent_and_fts_tracks_items() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rosso.db");

        let db = Db::open(&path).unwrap();
        // Re-opening runs the whole batch again; a non-idempotent statement would
        // error here, which is the point of the check.
        drop(db);
        let db = Db::open(&path).unwrap();

        db.with(|c| {
            c.execute(
                "INSERT INTO feeds (url, next_fetch_at, created_at) VALUES (?1, ?2, ?2)",
                ("https://example.com/feed.xml", "2026-01-01T00:00:00Z"),
            )?;
            c.execute(
                "INSERT INTO items (feed_id, guid, title, content_text, fetched_at)
                 VALUES (1, 'g1', 'Hedgehog census', 'counting hedgehogs', '2026-01-01T00:00:00Z')",
                [],
            )
        })
        .await
        .unwrap();

        let hits: i64 = db
            .with(|c| {
                c.query_row(
                    "SELECT count(*) FROM items_fts WHERE items_fts MATCH 'hedgehog'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(hits, 1);

        // The update trigger has to delete the old index row first, or the stale
        // term keeps matching.
        db.with(|c| {
            c.execute(
                "UPDATE items SET title = 'Badger census', content_text = 'counting badgers' WHERE id = 1",
                [],
            )
        })
        .await
        .unwrap();

        let stale: i64 = db
            .with(|c| {
                c.query_row(
                    "SELECT count(*) FROM items_fts WHERE items_fts MATCH 'hedgehog'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(stale, 0);
    }
}
