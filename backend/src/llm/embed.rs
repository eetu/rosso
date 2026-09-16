//! Embeddings, and the dedupe they exist for.
//!
//! The same story reported by five outlets is five rows that each have to be
//! read to be dismissed. This worker gives every item a vector, compares it
//! against the recent ones, and joins the matches into a cluster the list shows
//! as a single row.
//!
//! Vectors are plain `BLOB`s of little-endian `f32`, unit-normalized on the way
//! in so that cosine similarity is a dot product. The plan called for
//! `sqlite-vec`; see the note in the repo's `CLAUDE.md` for why it is not here.
//!
//! Like everything else LLM-shaped in rosso this is additive: with the model
//! host asleep nothing is embedded, nothing is clustered, and every item stays
//! its own row.

use std::time::Duration;

use super::ollama;
use crate::settings::{self, Settings};
use crate::store::{self, EmbedCandidate};
use crate::AppState;

/// Items per `/api/embed` call. One round trip either way, so the size is about
/// how much text to hold at once — 16 × 2 KB is nothing, 500 × 2 KB on a 1 GB
/// board is a choice.
const BATCH: u32 = 16;

/// How much of an article to embed. The opening of a piece is what says which
/// story it is; past that a longer text mostly dilutes the vector, and the model
/// truncates anyway.
const MAX_CHARS: usize = 2000;

/// How far back a duplicate can be. The same story from five outlets arrives
/// within hours; a match against something from March is a topic, not a
/// duplicate.
const WINDOW_HOURS: i64 = 72;

/// Give up on an item after this many failures.
pub const MAX_ATTEMPTS: i64 = 3;

const IDLE_PAUSE: Duration = Duration::from_secs(30);
const BUSY_PAUSE: Duration = Duration::from_secs(2);

pub fn spawn(state: AppState) {
    tokio::spawn(async move { run(state).await });
}

async fn run(state: AppState) {
    if state.cfg.ollama_url.is_none() {
        tracing::info!("no model host configured — items will not be clustered");
        return;
    }
    tokio::time::sleep(Duration::from_secs(8)).await;

    loop {
        let pause = match pass(&state).await {
            Ok(processed) if processed as u32 == BATCH => BUSY_PAUSE,
            Ok(_) => IDLE_PAUSE,
            Err(err) => {
                tracing::error!(err = ?err, "embedding pass failed");
                IDLE_PAUSE
            }
        };
        tokio::time::sleep(pause).await;
    }
}

async fn pass(state: &AppState) -> anyhow::Result<usize> {
    let Some(base) = state.cfg.ollama_url.as_deref() else {
        return Ok(0);
    };
    let settings = settings::load(&state.db, &state.cfg).await?;
    let model = &state.cfg.embed_model;

    let candidates = store::due_for_embedding(&state.db, model, BATCH).await?;
    if candidates.is_empty() {
        return Ok(0);
    }

    let inputs: Vec<String> = candidates.iter().map(embed_input).collect();
    // The same permit generation takes: the model host is one machine, and an
    // embedding batch landing mid-summary is two models resident at once on a
    // box that is also running an image pipeline.
    let vectors = {
        let _permit = state.llm_permits.acquire().await;
        match ollama::embed(&state.http, base, model, &inputs).await {
            Ok(vectors) => vectors,
            Err(err) => {
                // The whole batch failed — one call, one outcome. Charging each
                // item an attempt is right: a text the model chokes on is
                // indistinguishable from here, and three tries retire it.
                tracing::debug!(err = %err, n = candidates.len(), "embedding batch failed");
                for candidate in &candidates {
                    let _ =
                        store::record_embed_failure(&state.db, candidate.id, err.to_string()).await;
                }
                return Ok(0);
            }
        }
    };

    let mut done = 0;
    for (candidate, vector) in candidates.iter().zip(vectors) {
        let dims = vector.len();
        store::record_embedding(
            &state.db,
            candidate.id,
            model.clone(),
            dims,
            to_blob(&normalize(vector)),
        )
        .await?;
        cluster_one(state, &settings, model, candidate.id).await?;
        done += 1;
    }
    Ok(done)
}

/// Find this item's duplicates and join them.
async fn cluster_one(
    state: &AppState,
    settings: &Settings,
    model: &str,
    item_id: i64,
) -> anyhow::Result<()> {
    if !settings.dedupe_enabled() {
        return Ok(());
    }
    let Some(mine) = store::embedding_of(&state.db, item_id).await? else {
        return Ok(());
    };
    let mine = from_blob(&mine);

    let neighbours = store::neighbours_within(&state.db, model, WINDOW_HOURS, item_id).await?;
    let best = neighbours
        .iter()
        .map(|n| (n, similarity(&mine, &from_blob(&n.embedding))))
        .filter(|(_, score)| *score >= settings.dedupe_threshold)
        // `total_cmp` rather than `partial_cmp().unwrap()`: a NaN from a
        // degenerate vector would panic the worker instead of losing a match.
        .max_by(|(_, a), (_, b)| a.total_cmp(b));

    if let Some((neighbour, score)) = best {
        // Join the neighbour's existing cluster rather than starting a new one
        // with it, or a third report of the same story would split the group in
        // two depending on which of the first two it happened to match best.
        let head = neighbour.cluster_id.unwrap_or(neighbour.item_id);
        store::join_cluster(&state.db, item_id, head).await?;
        tracing::debug!(item = item_id, head, score, "clustered");
    }
    Ok(())
}

/// What gets embedded: the headline, then the opening of the article.
///
/// The title carries most of the signal for "is this the same story", and it is
/// the one field that is never missing.
fn embed_input(candidate: &EmbedCandidate) -> String {
    let body = candidate
        .content_text
        .as_deref()
        .unwrap_or_default()
        .chars()
        .take(MAX_CHARS)
        .collect::<String>();
    if body.trim().is_empty() {
        candidate.title.clone()
    } else {
        format!("{}\n\n{}", candidate.title, body)
    }
}

/// Scale to unit length, so similarity is a dot product rather than a dot
/// product plus two square roots per comparison.
///
/// A zero vector is left alone: it cannot be normalized, and it will simply
/// never match anything, which is the right outcome for a text the model made
/// nothing of.
fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Cosine similarity of two already-normalized vectors.
///
/// Length-mismatched vectors score 0 rather than panicking or comparing a
/// prefix: they come from different models and are not comparable at all.
fn similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Little-endian `f32`s back out. A blob whose length is not a multiple of four
/// yields the whole floats in it and drops the tail, which cannot happen for
/// anything `to_blob` wrote.
fn from_blob(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(f32::from_le_bytes)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_survive_the_round_trip_through_a_blob() {
        let v = vec![1.0_f32, -2.5, 0.0, 1e-8];
        assert_eq!(from_blob(&to_blob(&v)), v);
    }

    #[test]
    fn normalizing_makes_similarity_a_dot_product() {
        let a = normalize(vec![3.0, 4.0]);
        // 3-4-5 triangle: unit length within f32's slack.
        assert!((a.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-6);
        // A vector with itself is 1; an orthogonal one is 0.
        assert!((similarity(&a, &a) - 1.0).abs() < 1e-6);
        assert!(similarity(&normalize(vec![1.0, 0.0]), &normalize(vec![0.0, 1.0])).abs() < 1e-6);
        // Opposite directions are -1, which no threshold in [0,1] accepts.
        assert!(
            (similarity(&normalize(vec![1.0, 0.0]), &normalize(vec![-1.0, 0.0])) + 1.0).abs()
                < 1e-6
        );
    }

    #[test]
    fn a_zero_vector_normalizes_to_itself_and_matches_nothing() {
        let zero = normalize(vec![0.0, 0.0]);
        assert_eq!(zero, vec![0.0, 0.0]);
        assert_eq!(similarity(&zero, &normalize(vec![1.0, 1.0])), 0.0);
    }

    #[test]
    fn vectors_from_different_models_never_match() {
        // Two widths means two vector spaces. Comparing a prefix would produce a
        // plausible number for a comparison that has no meaning.
        assert_eq!(similarity(&[1.0, 0.0, 0.0], &[1.0, 0.0]), 0.0);
    }

    #[test]
    fn an_item_with_no_body_is_embedded_on_its_title_alone() {
        let bare = EmbedCandidate {
            id: 1,
            title: "Headline".into(),
            content_text: Some("   ".into()),
        };
        assert_eq!(embed_input(&bare), "Headline");

        let full = EmbedCandidate {
            id: 2,
            title: "Headline".into(),
            content_text: Some("x".repeat(MAX_CHARS + 500)),
        };
        assert_eq!(embed_input(&full).chars().count(), MAX_CHARS + 10);
    }
}
