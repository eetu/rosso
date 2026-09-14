//! The background poll loop — the thing that makes rosso a reader rather than a
//! fetch button.
//!
//! Detached and idempotent: it is not drained on shutdown, because every poll it
//! performs is safe to repeat. A restart mid-poll costs one re-fetch.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;

use super::{poll_feed, schedule};
use crate::store;
use crate::AppState;

/// Feeds examined per tick. A ceiling, not a target — it just keeps one tick from
/// queueing a thousand fetches after a long outage.
const BATCH: u32 = 50;

pub fn spawn(state: AppState) {
    tokio::spawn(async move { run(state).await });
}

async fn run(state: AppState) {
    // Let the server bind and answer /status before the first fetch competes for
    // the runtime.
    tokio::time::sleep(Duration::from_secs(5)).await;

    let base = Duration::from_secs(state.cfg.poll_tick_s);
    let permits = Arc::new(Semaphore::new(state.cfg.fetch_concurrency));

    loop {
        if let Err(err) = tick(&state, &permits).await {
            // A tick that fails must never end the loop — that would silently
            // turn rosso back into a click-to-fetch reader.
            tracing::error!(err = ?err, "poll tick failed");
        }
        // Jittered so feeds added in one batch do not stay in lockstep and hit
        // their hosts on the same second forever.
        tokio::time::sleep(schedule::jitter(base)).await;
    }
}

async fn tick(state: &AppState, permits: &Arc<Semaphore>) -> anyhow::Result<()> {
    let due = store::due_feeds(&state.db, BATCH).await?;
    if due.is_empty() {
        return Ok(());
    }
    tracing::debug!(count = due.len(), "polling due feeds");

    let mut tasks = Vec::with_capacity(due.len());
    for feed in due {
        let state = state.clone();
        let permits = Arc::clone(permits);
        tasks.push(tokio::spawn(async move {
            // Held for the fetch's lifetime, so `fetch_concurrency` really is the
            // number of sockets open at once.
            let _permit = permits.acquire_owned().await;
            poll_feed(&state, feed).await
        }));
    }
    for task in tasks {
        let _ = task.await;
    }
    Ok(())
}
