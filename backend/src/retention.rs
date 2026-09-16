//! Throwing away what you have finished with.
//!
//! An archive that only grows is fine for a long time and then is not: the item
//! bodies are the large part of the file, the board it lives on has a 1 GB
//! budget, and the nightly backup copies all of it.
//!
//! **Off by default.** Deleting a reader's archive is not something to start
//! doing to someone who never asked for it, so `retention_days = 0` keeps
//! everything and that is what a database with no setting row gets.
//!
//! What is never deleted: anything starred, anything unread, and anything a
//! digest refers to. The first two are the reader saying it still matters; the
//! third is what keeps a digest from becoming a page of dead links.

use std::time::Duration;

use crate::store;
use crate::{settings, AppState};

/// Once an hour. The candidate set moves by a day at a time, so anything more
/// eager is a query that finds nothing.
const TICK: Duration = Duration::from_secs(3600);

/// Deleted per pass. A cap so the first run after turning retention on does not
/// hold the write lock through a hundred thousand rows — it takes an hour
/// longer and nothing notices.
const BATCH: u32 = 2000;

pub fn spawn(state: AppState) {
    tokio::spawn(async move { run(state).await });
}

async fn run(state: AppState) {
    // A pause before the first pass, so a boot loop cannot turn into a delete
    // loop while something else is wrong.
    tokio::time::sleep(Duration::from_secs(120)).await;
    loop {
        if let Err(err) = pass(&state).await {
            tracing::error!(err = ?err, "retention pass failed");
        }
        tokio::time::sleep(TICK).await;
    }
}

async fn pass(state: &AppState) -> anyhow::Result<()> {
    let days = settings::load(&state.db, &state.cfg).await?.retention_days;
    if days <= 0 {
        return Ok(());
    }
    let deleted = store::prune_items(&state.db, days, BATCH).await?;
    if deleted > 0 {
        tracing::info!(deleted, days, "pruned read items");
    }
    Ok(())
}
