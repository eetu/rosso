use std::sync::Arc;
use std::time::Duration;

use tracing_subscriber::EnvFilter;

pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod extract;
pub mod feed;
pub mod llm;
pub mod routes;
pub mod settings;
pub mod shutdown;
pub mod store;
pub mod util;

use config::Config;
use db::Db;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub db: Db,
    /// Shared client for feed fetches, article extraction and Ollama.
    pub http: reqwest::Client,
    /// Cached "is the model host up?", so `/status` reports what is true rather
    /// than what is configured.
    pub llm_health: llm::Health,
    /// Every generation queues behind this. The model host is one machine doing
    /// one thing at a time; the permit count is what makes that true here.
    pub llm_permits: Arc<tokio::sync::Semaphore>,
}

pub async fn run_server() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    // `rustls-no-provider` leaves the choice to us; without this every HTTPS
    // request panics on the missing default provider.
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install the ring crypto provider"))?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,rosso_backend=debug")),
        )
        .init();

    let cfg = Config::from_env()?;
    if cfg.dev_auth {
        tracing::warn!("DEV_AUTH/ROSSO_OPEN set — forward-auth gate bypassed; not for prod");
    }
    if cfg.ollama_url.is_none() {
        tracing::info!("ROSSO_OLLAMA_URL unset — running as a plain reader, no LLM features");
    }

    let db = Db::open(&cfg.db_path)?;
    let bind = cfg.bind.clone();
    let llm_concurrency = cfg.llm_concurrency;
    let state = AppState {
        cfg: Arc::new(cfg),
        db,
        // Bounded, or one upstream that accepts the connection and then never
        // answers wedges a poll worker forever. Ollama generation gets a longer
        // per-request override at the call site.
        http: reqwest::Client::builder()
            // The feed-reader convention: a name a publisher can look up, and a
            // URL saying what it is. A bare `rosso/0.1.0` is indistinguishable
            // from an unknown scraper to a host deciding whether to serve one.
            .user_agent(concat!(
                "rosso/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/eetu/rosso)"
            ))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()?,
        llm_health: llm::Health::default(),
        llm_permits: llm::enrich::permits(llm_concurrency),
    };

    feed::poller::spawn(state.clone());
    extract::spawn(state.clone());
    llm::enrich::spawn(state.clone());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "rosso listening");
    axum::serve(listener, routes::router(state))
        .with_graceful_shutdown(shutdown::signal_from_env())
        .await?;
    Ok(())
}
