//! The live channel: what the background loops did, pushed to an open tab.
//!
//! rosso fetches whether or not a browser is open, which is the whole point of
//! it — but it also means a tab left open all morning shows a morning-old list.
//! This is how that tab finds out, without polling.
//!
//! **Fire-and-forget by design.** A send with no listeners is not an error and
//! nothing here is persisted: an event missed while the tab was closed is
//! recovered by the reload that opening it performs anyway. So no caller checks
//! the result of `emit`, and no path waits on delivery.

use serde::Serialize;
use tokio::sync::broadcast;

/// What the SPA is told. The payload carries the changed values rather than an
/// id to go and fetch: the alternative is a request per enriched item, arriving
/// in a burst exactly when the model host has just finished a backlog.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Event {
    /// A poll inserted something. Counts moved; the list may want reloading.
    ItemsNew { feed_id: i64, count: usize },
    /// A poll finished with nothing new, or failed. Only the feed row moved.
    FeedUpdated { feed_id: i64 },
    /// The model got to an item. Everything needed to patch the row in place.
    ItemEnriched {
        item_id: i64,
        summary: Option<String>,
        score: Option<i64>,
        score_reason: Option<String>,
        tags: Vec<String>,
    },
}

/// The fan-out. Cheap to clone; one lives on `AppState`.
#[derive(Clone)]
pub struct Events(broadcast::Sender<Event>);

impl Default for Events {
    fn default() -> Self {
        // Deep enough to absorb a poll tick that lands on a dozen feeds at once.
        // A client slower than this is dropped to the newest event rather than
        // being waited for — see `subscribe`.
        Self(broadcast::channel(256).0)
    }
}

impl Events {
    pub fn emit(&self, event: Event) {
        // `Err` means nobody is listening, which is the normal state of a reader
        // whose tab is closed.
        let _ = self.0.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.0.subscribe()
    }
}
