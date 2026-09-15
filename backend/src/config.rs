//! Plain `env::var()` → `Config`. The four contract fields (`bind`, `dev_auth`,
//! `db_path`, `static_dir`) are fixed by the house seam so the frontend proxy and
//! the keel quadlet line up; everything below them is rosso's own.

use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    /// Listen address. `ROSSO_BIND`.
    pub bind: String,
    /// When set, `/api/*` is reachable without forward-auth headers. `DEV_AUTH=1`
    /// (local dev) or `ROSSO_OPEN=1` (a LAN-only deploy with no oauth2-proxy).
    pub dev_auth: bool,
    /// SQLite file. `ROSSO_DB_PATH`.
    pub db_path: PathBuf,
    /// Built SPA directory. `STATIC_DIR`.
    pub static_dir: PathBuf,

    /// Ollama base URL on the mini. Unset disables every LLM feature; the reader
    /// keeps working.
    pub ollama_url: Option<String>,
    /// Generation model for summaries, scores and digests.
    pub llm_model: String,
    /// Embedding model. 768 dims for `embeddinggemma:300m`.
    pub embed_model: String,
    /// Concurrent Ollama calls. One by default: the mini is a single box whose
    /// memory is shared with ComfyUI, and a background summarizer is exactly the
    /// thing that would otherwise fire N generations at once.
    pub llm_concurrency: usize,
    /// Fetch and readability-parse the linked page when a feed ships only a
    /// teaser. `ROSSO_EXTRACT=0` turns it off; the reader then shows whatever the
    /// feed gave and nothing else.
    pub extract_enabled: bool,
    /// Allow extraction to fetch private, loopback and link-local addresses.
    /// Off by default: item URLs come from the publisher, so an unguarded
    /// extractor is an SSRF hole pointed at the LAN. Turn on only to read a feed
    /// whose articles genuinely live on your own network — and note the
    /// integration harness sets it, because wiremock serves from 127.0.0.1.
    pub extract_allow_private: bool,
    /// Ceiling on a feed document, in bytes. `ROSSO_MAX_FEED_MB`.
    ///
    /// Full-archive feeds vary by two orders of magnitude — most are well under
    /// a megabyte, danluu.com is 6.3 — so the useful ceiling depends on what you
    /// subscribe to and how much memory the box has.
    pub max_feed_bytes: usize,
    /// Concurrent feed fetches per poll tick.
    pub fetch_concurrency: usize,
    /// How often the poller looks for due feeds, in seconds.
    pub poll_tick_s: u64,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let dev_auth = env::var("DEV_AUTH").as_deref() == Ok("1")
            || env::var("ROSSO_OPEN").as_deref() == Ok("1");

        Ok(Self {
            bind: env::var("ROSSO_BIND").unwrap_or_else(|_| "0.0.0.0:3008".into()),
            dev_auth,
            db_path: PathBuf::from(env::var("ROSSO_DB_PATH").unwrap_or_else(|_| "rosso.db".into())),
            static_dir: PathBuf::from(env::var("STATIC_DIR").unwrap_or_else(|_| "./dist".into())),

            ollama_url: opt_env("ROSSO_OLLAMA_URL").map(|s| s.trim_end_matches('/').to_string()),
            llm_model: opt_env("ROSSO_LLM_MODEL").unwrap_or_else(|| "gemma4:e4b-mlx".into()),
            embed_model: opt_env("ROSSO_EMBED_MODEL")
                .unwrap_or_else(|| "embeddinggemma:300m".into()),
            extract_enabled: opt_env("ROSSO_EXTRACT").as_deref() != Some("0"),
            extract_allow_private: opt_env("ROSSO_EXTRACT_ALLOW_PRIVATE").as_deref() == Some("1"),
            max_feed_bytes: num_env::<usize>(
                "ROSSO_MAX_FEED_MB",
                crate::feed::fetch::DEFAULT_MAX_FEED_MB,
            )
            .clamp(1, 256)
                * 1024
                * 1024,
            llm_concurrency: num_env("ROSSO_LLM_CONCURRENCY", 1).max(1),
            fetch_concurrency: num_env("ROSSO_FETCH_CONCURRENCY", 4).max(1),
            poll_tick_s: num_env("ROSSO_POLL_TICK_S", 60).max(10),
        })
    }
}

/// An empty string counts as unset — a quadlet that declares a variable it has no
/// value for should not read as "configured with the empty string".
fn opt_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|s| !s.trim().is_empty())
}

fn num_env<T: std::str::FromStr>(key: &str, default: T) -> T {
    opt_env(key).and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_dev_auth_bypass() {
        // Serial within one test: these mutate process-wide env.
        unsafe { env::set_var("ROSSO_OPEN", "1") };
        unsafe { env::set_var("ROSSO_OLLAMA_URL", "http://example:11434/") };
        let cfg = Config::from_env().unwrap();
        assert!(cfg.dev_auth);
        assert_eq!(cfg.static_dir, PathBuf::from("./dist"));
        assert_eq!(cfg.ollama_url.as_deref(), Some("http://example:11434"));
        assert_eq!(cfg.llm_concurrency, 1);

        unsafe { env::set_var("ROSSO_OLLAMA_URL", "  ") };
        assert!(Config::from_env().unwrap().ollama_url.is_none());

        unsafe { env::remove_var("ROSSO_OPEN") };
        unsafe { env::remove_var("ROSSO_OLLAMA_URL") };
    }

    #[test]
    fn the_feed_ceiling_is_read_in_megabytes_and_bounded() {
        unsafe { env::set_var("ROSSO_MAX_FEED_MB", "32") };
        assert_eq!(Config::from_env().unwrap().max_feed_bytes, 32 * 1024 * 1024);

        // A zero would refuse every feed; an absurd value would let one document
        // take the process down. Neither is a useful thing to have configured.
        unsafe { env::set_var("ROSSO_MAX_FEED_MB", "0") };
        assert_eq!(Config::from_env().unwrap().max_feed_bytes, 1024 * 1024);
        unsafe { env::set_var("ROSSO_MAX_FEED_MB", "99999") };
        assert_eq!(
            Config::from_env().unwrap().max_feed_bytes,
            256 * 1024 * 1024
        );

        unsafe { env::remove_var("ROSSO_MAX_FEED_MB") };
        assert_eq!(Config::from_env().unwrap().max_feed_bytes, 16 * 1024 * 1024);
    }

    #[test]
    fn numeric_floors_survive_junk() {
        unsafe { env::set_var("ROSSO_POLL_TICK_S", "0") };
        assert_eq!(Config::from_env().unwrap().poll_tick_s, 10);
        unsafe { env::set_var("ROSSO_POLL_TICK_S", "not-a-number") };
        assert_eq!(Config::from_env().unwrap().poll_tick_s, 60);
        unsafe { env::remove_var("ROSSO_POLL_TICK_S") };
    }
}
