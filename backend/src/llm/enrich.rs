//! The worker that turns items into summaries and scores.
//!
//! Like extraction, the work set is derived from row state rather than pushed
//! onto a queue: an item needs a pass when it has never been enriched, or when
//! the interest profile it was scored against is no longer the current one.
//! Editing the profile is therefore its own invalidation — nothing has to walk
//! the table and enqueue a rescore.
//!
//! Strictly one call at a time. The model host is a single desk machine sharing
//! its memory with an image pipeline, and a background summarizer firing N
//! generations at once is exactly how you take it down.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::Semaphore;

use super::{ollama, prompts};
use crate::settings::{self, Settings};
use crate::store::{self, EnrichCandidate, EnrichKind};
use crate::AppState;

/// Items considered per pass. Small: each one is a generation, and a long batch
/// just delays picking up whatever arrived meanwhile.
const BATCH: u32 = 5;

/// Wait between passes when there was nothing to do. One indexed query, so it is
/// cheap to come back often — and coming back often is what makes an edited
/// profile take effect while the user is still looking at the screen.
const IDLE_PAUSE: Duration = Duration::from_secs(10);

/// Wait between passes when the batch was full — there is more waiting, so come
/// back promptly, but not instantly.
const BUSY_PAUSE: Duration = Duration::from_secs(2);

/// Give up on an item after this many failures.
pub const MAX_ATTEMPTS: i64 = 3;

#[derive(Debug, Deserialize)]
struct SummaryOnly {
    summary: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Enriched {
    summary: String,
    score: i64,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Scored {
    score: i64,
    #[serde(default)]
    reason: String,
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move { run(state).await });
}

async fn run(state: AppState) {
    if state.cfg.ollama_url.is_none() {
        tracing::info!("no model host configured — items will not be summarized or scored");
        return;
    }
    // Just enough to let the server bind and the first poll start. Ordering
    // against extraction is enforced by the candidate query, not by this sleep —
    // a delay long enough to be a correctness guarantee would have to be longer
    // than the slowest possible page fetch, which is not a number that exists.
    tokio::time::sleep(Duration::from_secs(5)).await;

    loop {
        let pause = match pass(&state).await {
            Ok(processed) if processed as u32 == BATCH => BUSY_PAUSE,
            Ok(_) => IDLE_PAUSE,
            Err(err) => {
                tracing::error!(err = ?err, "enrichment pass failed");
                IDLE_PAUSE
            }
        };
        tokio::time::sleep(pause).await;
    }
}

async fn pass(state: &AppState) -> anyhow::Result<usize> {
    let settings = settings::load(&state.db, &state.cfg).await?;
    let candidates =
        store::due_for_enrichment(&state.db, &settings.profile_fingerprint(), BATCH).await?;
    if candidates.is_empty() {
        return Ok(0);
    }

    let examples = store::feedback_examples(&state.db, 8).await?;
    let mut done = 0;
    for candidate in &candidates {
        enrich_one(state, &settings, &examples, candidate).await;
        done += 1;
    }
    Ok(done)
}

/// Enrich one item and record the outcome. Never returns an error: a model that
/// is asleep or answers badly is an ordinary state for rosso.
pub async fn enrich_one(
    state: &AppState,
    settings: &Settings,
    examples: &[store::FeedbackExample],
    candidate: &EnrichCandidate,
) {
    let Some(base) = state.cfg.ollama_url.as_deref() else {
        return;
    };
    // Held for the whole call, so `llm_concurrency` is the number of generations
    // actually in flight rather than the number of tasks that think they may.
    let _permit = state.llm_permits.acquire().await;

    let result = match candidate.kind {
        EnrichKind::Full => full_pass(state, settings, examples, candidate, base).await,
        EnrichKind::Rescore => rescore_pass(state, settings, examples, candidate, base).await,
    };

    match result {
        Ok(()) => tracing::debug!(item = candidate.id, kind = ?candidate.kind, "enriched"),
        Err(err) => {
            tracing::debug!(item = candidate.id, err = %err, "enrichment failed");
            if let Err(db_err) =
                store::record_enrich_failure(&state.db, candidate.id, err.to_string()).await
            {
                tracing::error!(item = candidate.id, err = ?db_err, "could not record failure");
            }
        }
    }
}

async fn full_pass(
    state: &AppState,
    settings: &Settings,
    examples: &[store::FeedbackExample],
    candidate: &EnrichCandidate,
    base: &str,
) -> anyhow::Result<()> {
    let system = prompts::enrich_system(&settings.interest_profile, examples);
    let user = prompts::item_user(
        &candidate.feed_title,
        &candidate.title,
        candidate.content_text.as_deref(),
    );
    let model = &settings.llm_model;
    let fingerprint = settings.profile_fingerprint();

    if settings.scoring_enabled() {
        let out: Enriched = ollama::chat_json(
            &state.http,
            base,
            model,
            &system,
            &user,
            prompts::enrich_schema(),
        )
        .await?;
        store::record_enrichment(
            &state.db,
            candidate.id,
            out.summary,
            Some(out.score.clamp(0, 100)),
            Some(out.reason),
            tags_json(&out.tags),
            model.clone(),
            fingerprint,
        )
        .await?;
    } else {
        let out: SummaryOnly = ollama::chat_json(
            &state.http,
            base,
            model,
            &system,
            &user,
            prompts::summary_schema(),
        )
        .await?;
        store::record_enrichment(
            &state.db,
            candidate.id,
            out.summary,
            None,
            None,
            tags_json(&out.tags),
            model.clone(),
            fingerprint,
        )
        .await?;
    }
    Ok(())
}

/// Re-score against an edited profile, reusing the stored summary.
///
/// This is why editing the profile is cheap: the expensive half — reading the
/// article — is already done, so a rescore is a short prompt over a summary.
async fn rescore_pass(
    state: &AppState,
    settings: &Settings,
    examples: &[store::FeedbackExample],
    candidate: &EnrichCandidate,
    base: &str,
) -> anyhow::Result<()> {
    let fingerprint = settings.profile_fingerprint();

    // The profile was cleared: drop the stale score rather than leaving a number
    // that no longer answers any question.
    if !settings.scoring_enabled() {
        store::record_rescore(&state.db, candidate.id, None, None, fingerprint).await?;
        return Ok(());
    }

    let system = prompts::score_system(&settings.interest_profile, examples);
    let user = prompts::item_user(
        &candidate.feed_title,
        &candidate.title,
        candidate.summary.as_deref(),
    );
    let out: Scored = ollama::chat_json(
        &state.http,
        base,
        &settings.llm_model,
        &system,
        &user,
        prompts::score_schema(),
    )
    .await?;

    store::record_rescore(
        &state.db,
        candidate.id,
        Some(out.score.clamp(0, 100)),
        Some(out.reason),
        fingerprint,
    )
    .await?;
    Ok(())
}

fn tags_json(tags: &[String]) -> Option<String> {
    if tags.is_empty() {
        return None;
    }
    serde_json::to_string(tags).ok()
}

/// Shared permit pool, so every caller queues behind the same one generation.
pub fn permits(concurrency: usize) -> Arc<Semaphore> {
    Arc::new(Semaphore::new(concurrency.max(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_stored_as_json_or_not_at_all() {
        assert_eq!(tags_json(&[]), None);
        assert_eq!(
            tags_json(&["rust".into(), "sqlite".into()]).as_deref(),
            Some(r#"["rust","sqlite"]"#)
        );
    }

    #[test]
    fn a_model_that_omits_optional_fields_still_parses() {
        // `format` constrains decoding, but a model can still answer thinly, and
        // losing a whole summary over a missing `reason` would be absurd.
        let out: Enriched = serde_json::from_str(r#"{"summary":"s","score":70}"#).unwrap();
        assert_eq!(out.score, 70);
        assert!(out.reason.is_empty());
        assert!(out.tags.is_empty());
    }

    #[test]
    fn an_out_of_range_score_is_clamped_not_trusted() {
        let out: Enriched =
            serde_json::from_str(r#"{"summary":"s","score":9000,"reason":"r"}"#).unwrap();
        assert_eq!(out.score.clamp(0, 100), 100);
    }
}
