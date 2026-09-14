//! Graceful shutdown.
//!
//! The binary runs as PID 1 in its container, and PID 1 gets no default signal
//! actions from the kernel — a process with no SIGTERM handler installed simply
//! *ignores* it, so every deploy sits through podman's 10 s stop timeout and then
//! SIGKILLs. Handling the signal stops the listener accepting and lets in-flight
//! requests finish.
//!
//! The drain is bounded: `with_graceful_shutdown` alone waits for every open
//! connection, and rosso holds SSE streams open indefinitely, so on its own it
//! would never return. A watchdog exits once the grace window expires.
//!
//! The default sits just under podman's 10 s stop timeout. Raising [`GRACE_ENV`]
//! past that only helps if the quadlet's `StopTimeout` goes up to match.

use std::time::Duration;

pub const GRACE_ENV: &str = "ROSSO_SHUTDOWN_GRACE_S";

pub const DEFAULT_GRACE: Duration = Duration::from_secs(9);

pub fn grace_from_env() -> Duration {
    std::env::var(GRACE_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_GRACE)
}

/// Resolves on SIGTERM or SIGINT, then arms a watchdog that force-exits once
/// `grace` has elapsed. Pass to `axum::serve(..).with_graceful_shutdown(..)`.
///
/// Exiting 0 rather than aborting is deliberate: a bounded drain is a normal stop,
/// and `Restart=always` shouldn't see it as a crash.
pub async fn signal(grace: Duration) {
    let received = wait_for_signal().await;
    tracing::info!(
        signal = received,
        grace_secs = grace.as_secs(),
        "shutdown signal received — draining in-flight requests"
    );
    tokio::spawn(async move {
        tokio::time::sleep(grace).await;
        tracing::warn!(
            grace_secs = grace.as_secs(),
            "drain window expired with requests still open — exiting anyway"
        );
        std::process::exit(0);
    });
}

pub async fn signal_from_env() {
    signal(grace_from_env()).await
}

#[cfg(unix)]
async fn wait_for_signal() -> &'static str {
    use tokio::signal::unix::{signal as unix_signal, SignalKind};
    // A failure here means the process silently keeps ignoring SIGTERM, i.e. back
    // to the SIGKILL behaviour this module exists to remove — worth failing loudly
    // at startup rather than degrading quietly.
    let mut term = unix_signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut int = unix_signal(SignalKind::interrupt()).expect("install SIGINT handler");
    tokio::select! {
        _ = term.recv() => "SIGTERM",
        _ = int.recv() => "SIGINT",
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() -> &'static str {
    tokio::signal::ctrl_c()
        .await
        .expect("install ctrl-c handler");
    "ctrl-c"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grace_defaults_when_unset_or_junk() {
        unsafe { std::env::remove_var(GRACE_ENV) };
        assert_eq!(grace_from_env(), DEFAULT_GRACE);

        unsafe { std::env::set_var(GRACE_ENV, "not-a-number") };
        assert_eq!(grace_from_env(), DEFAULT_GRACE);

        unsafe { std::env::set_var(GRACE_ENV, "45") };
        assert_eq!(grace_from_env(), Duration::from_secs(45));

        unsafe { std::env::remove_var(GRACE_ENV) };
    }
}
