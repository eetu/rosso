pub mod comments;
pub mod discover;
pub mod fetch;
pub mod opml;
pub mod parse;
pub mod poller;
pub mod schedule;

use schedule::PollResult;

use crate::store::{self, DueFeed};
use crate::AppState;

/// Poll one feed and record the outcome.
///
/// Never returns an error: a feed that 500s, times out or serves garbage is an
/// ordinary event, recorded on the feed row and backed off. Only a caller that
/// wants to report the outcome (the manual refresh button) looks at the result.
pub async fn poll_feed(state: &AppState, feed: DueFeed) -> PollResult {
    match poll_inner(state, &feed).await {
        Ok(result) => result,
        Err(err) => {
            let failures = feed.failures + 1;
            let next = schedule::next_interval(feed.interval_s, PollResult::Failed, failures, None);
            tracing::warn!(feed_id = feed.id, url = %feed.url, failures, err = %err, "poll failed");
            if let Err(db_err) =
                store::record_failure(&state.db, feed.id, err.to_string(), next).await
            {
                tracing::error!(feed_id = feed.id, err = ?db_err, "could not record poll failure");
            }
            PollResult::Failed
        }
    }
}

async fn poll_inner(state: &AppState, feed: &DueFeed) -> anyhow::Result<PollResult> {
    let fetched = fetch::conditional_get(
        &state.http,
        &feed.url,
        feed.etag.as_deref(),
        feed.last_modified.as_deref(),
        state.cfg.max_feed_bytes,
    )
    .await?;

    let (bytes, etag, last_modified) = match fetched {
        fetch::Fetched::NotModified => {
            let next = schedule::next_interval(feed.interval_s, PollResult::Unchanged, 0, None);
            store::record_unchanged(&state.db, feed.id, next).await?;
            tracing::debug!(feed_id = feed.id, "not modified");
            return Ok(PollResult::Unchanged);
        }
        fetch::Fetched::Body {
            bytes,
            etag,
            last_modified,
        } => (bytes, etag, last_modified),
    };

    // feed-rs walks the whole document, so keep it off the async worker threads.
    let url = feed.url.clone();
    let parsed = tokio::task::spawn_blocking(move || parse::parse(&bytes, &url)).await??;

    let (inserted, interval) = store::record_success(
        &state.db,
        feed.id,
        parsed,
        etag,
        last_modified,
        feed.interval_s,
    )
    .await?;

    tracing::debug!(feed_id = feed.id, inserted, interval, "polled");
    Ok(if inserted > 0 {
        PollResult::NewItems
    } else {
        PollResult::Unchanged
    })
}
