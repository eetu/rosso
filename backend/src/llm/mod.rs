//! The model host, as far as rosso currently needs it: is it there?
//!
//! Grows into the summarize/score/embed client. For now it exists so `/status`
//! can report the *resolved* state rather than the configured one — a URL in the
//! environment says nothing about whether the box at the other end is awake, and
//! the mini frequently is not.

use std::time::{Duration, Instant};

pub mod digest;
pub mod embed;
pub mod enrich;
pub mod ollama;
pub mod prompts;

/// How long a probe result stands. `/status` is polled by the SPA and by the
/// uptime monitor, and neither should turn into a health-check flood against a
/// machine whose whole job is running one model at a time.
const TTL: Duration = Duration::from_secs(60);

/// Short: an unreachable host should make `/status` slow, not hang it.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Cached answer to "is the model host up?".
#[derive(Clone, Default)]
pub struct Health {
    last: std::sync::Arc<tokio::sync::Mutex<Option<(Instant, bool)>>>,
}

impl Health {
    /// `false` when no host is configured, when it cannot be reached, or when it
    /// answers with an error. Never returns an error of its own: an absent model
    /// host is an ordinary state for rosso, not a fault.
    pub async fn available(&self, http: &reqwest::Client, url: Option<&str>) -> bool {
        let Some(url) = url else {
            return false;
        };

        let mut cached = self.last.lock().await;
        if let Some((at, healthy)) = *cached {
            if at.elapsed() < TTL {
                return healthy;
            }
        }

        let healthy = probe(http, url).await;
        *cached = Some((Instant::now(), healthy));
        healthy
    }
}

async fn probe(http: &reqwest::Client, url: &str) -> bool {
    // `/api/tags` is the cheapest endpoint that proves Ollama itself is
    // answering — it lists what is installed and loads nothing.
    match http
        .get(format!("{url}/api/tags"))
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
    {
        Ok(res) => res.status().is_success(),
        Err(err) => {
            tracing::debug!(%url, err = %err, "model host unreachable");
            false
        }
    }
}
