//! The daily digest: what the day amounted to, in one pass.
//!
//! The reader already tells you what arrived and how well it matched your
//! profile. What it cannot tell you is what the day *was* — which of forty items
//! were the same argument from three directions, and which one thing was worth
//! the morning. That is one generation over summaries the model has already
//! written, so it costs a single call.
//!
//! **Days are UTC.** The runtime image is `scratch` and carries no tzdata, so
//! `chrono::Local` would silently resolve to UTC anyway; saying UTC out loud is
//! better than a "local hour" setting that quietly is not one. The UI labels the
//! hour as UTC.
//!
//! Additive like everything else here: with no model host there is no digest and
//! nothing else notices.

use std::time::Duration;

use chrono::{Timelike, Utc};
use serde::{Deserialize, Serialize};

use super::{ollama, prompts};
use crate::settings;
use crate::store::{self, DigestCandidate};
use crate::AppState;

/// Items the model is shown. Enough that a busy day is represented; few enough
/// that the prompt stays one generation rather than a context-window problem.
const CANDIDATES: u32 = 40;

/// Below this the day is not worth a digest. Two items are a list, not a day.
const MIN_ITEMS: usize = 3;

/// How often to check whether the day's digest is due. The check is one indexed
/// lookup, and a coarse tick would mean a digest arriving up to that late.
const TICK: Duration = Duration::from_secs(600);

/// What the model returns, and what the API hands the SPA.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Digest {
    pub intro: String,
    pub threads: Vec<Thread>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Thread {
    pub title: String,
    pub note: String,
    /// Ids from the candidate list. Anything the model invented is dropped
    /// before storage — a link to an item that does not exist is worse than a
    /// thread with one fewer.
    pub item_ids: Vec<i64>,
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move { run(state).await });
}

async fn run(state: AppState) {
    if state.cfg.ollama_url.is_none() {
        tracing::info!("no model host configured — no daily digest");
        return;
    }
    loop {
        if let Err(err) = tick(&state).await {
            tracing::error!(err = ?err, "digest tick failed");
        }
        tokio::time::sleep(TICK).await;
    }
}

/// Generate yesterday's digest once the configured hour has passed.
///
/// Yesterday's, not today's: a digest of a day still in progress is a digest of
/// the morning. The hour is when it appears, and it covers the whole day before.
async fn tick(state: &AppState) -> anyhow::Result<()> {
    let settings = settings::load(&state.db, &state.cfg).await?;
    let now = Utc::now();
    if (now.hour() as i64) < settings.digest_hour {
        return Ok(());
    }

    let day = (now - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    // Already written. The day is the primary key, so this is also what keeps a
    // restart loop from regenerating it every boot.
    if store::get_digest(&state.db, &day).await?.is_some() {
        return Ok(());
    }

    match generate(state, &day).await {
        Ok(Some(count)) => tracing::info!(%day, count, "digest written"),
        // A quiet day costs one indexed count per tick and no generation, so
        // there is nothing to back off from.
        Ok(None) => tracing::debug!(%day, "too little to digest"),
        // Retried on the next tick, which is the point: the model host is a desk
        // machine that is often simply not on yet. It stops being retried when
        // the day rolls over, since only yesterday is ever considered.
        Err(err) => tracing::warn!(%day, err = %err, "digest generation failed"),
    }
    Ok(())
}

/// Build and store one day's digest. `None` when the day held too little.
pub async fn generate(state: &AppState, day: &str) -> anyhow::Result<Option<usize>> {
    let base = state
        .cfg
        .ollama_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no model host configured"))?;
    let settings = settings::load(&state.db, &state.cfg).await?;

    let candidates = store::digest_candidates(&state.db, day, CANDIDATES).await?;
    if candidates.len() < MIN_ITEMS {
        return Ok(None);
    }

    let system = prompts::digest_system(&settings.interest_profile, day);
    let user = prompts::digest_user(&candidates);
    let digest: Digest = {
        let _permit = state.llm_permits.acquire().await;
        ollama::chat_json(
            &state.http,
            base,
            &settings.llm_model,
            &system,
            &user,
            prompts::digest_schema(),
        )
        .await?
    };

    let digest = prune(digest, &candidates);
    let count = candidates.len();
    store::record_digest(
        &state.db,
        day.to_string(),
        serde_json::to_string(&digest)?,
        settings.llm_model.clone(),
        count,
    )
    .await?;
    Ok(Some(count))
}

/// Drop ids the model invented, and threads left with nothing.
///
/// Models do hallucinate an id, and one that does not resolve would render as a
/// thread promising items the reader cannot open. Constraining the *schema* to
/// integers is not the same as constraining them to these integers.
fn prune(mut digest: Digest, candidates: &[DigestCandidate]) -> Digest {
    let known: std::collections::HashSet<i64> = candidates.iter().map(|c| c.id).collect();
    for thread in &mut digest.threads {
        thread.item_ids.retain(|id| known.contains(id));
    }
    digest.threads.retain(|t| !t.item_ids.is_empty());
    digest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: i64) -> DigestCandidate {
        DigestCandidate {
            id,
            title: format!("Item {id}"),
            feed_title: "A feed".into(),
            summary: None,
            score: None,
        }
    }

    #[test]
    fn invented_ids_and_the_threads_left_empty_are_dropped() {
        let digest = Digest {
            intro: "a day".into(),
            threads: vec![
                Thread {
                    title: "Real".into(),
                    note: "n".into(),
                    item_ids: vec![1, 999, 2],
                },
                Thread {
                    title: "Entirely invented".into(),
                    note: "n".into(),
                    item_ids: vec![998, 999],
                },
            ],
        };
        let pruned = prune(digest, &[candidate(1), candidate(2), candidate(3)]);
        assert_eq!(pruned.threads.len(), 1);
        assert_eq!(pruned.threads[0].item_ids, vec![1, 2]);
    }
}
