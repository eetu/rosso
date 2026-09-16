//! A site's icon, stored in the database rather than linked to.
//!
//! A feed list is much easier to scan with icons in it, but the obvious way to
//! get them — putting the publisher's URL in an `<img>` — means the reader
//! announces itself to twenty hosts every time it is opened, from whatever
//! network you are on. So the icon is fetched once, on the server, and kept as a
//! `data:` URI on the feed row. The browser never talks to the publisher, and
//! the icon outlives the site.
//!
//! Cheap by construction: one attempt per feed ever, three on failure, then
//! never again. A feed with no icon simply shows none.

use base64::Engine;
use reqwest::Client;
use url::Url;

use super::discover::icon_links;
use super::fetch::read_capped;
use crate::{extract, store, AppState};
use std::time::Duration;

/// Icons are small. This is generous for an `apple-touch-icon` PNG and still far
/// below anything worth holding in memory or in a row.
const MAX_ICON_BYTES: usize = 64 * 1024;

/// Enough of a page to have passed its `<head>`.
const MAX_PAGE_BYTES: usize = 512 * 1024;

/// Give up on a site's icon after this many failures.
pub const MAX_ATTEMPTS: i64 = 3;

const IDLE_PAUSE: Duration = Duration::from_secs(120);
const BUSY_PAUSE: Duration = Duration::from_secs(3);

/// Formats a browser will render from a `data:` URI and that a publisher
/// actually serves. `.ico` is included because it is still the most common
/// answer and browsers read it fine — nothing here decodes the bytes.
const IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/x-icon",
    "image/vnd.microsoft.icon",
    "image/svg+xml",
    "image/jpeg",
    "image/gif",
    "image/webp",
];

pub fn spawn(state: AppState) {
    tokio::spawn(async move { run(state).await });
}

async fn run(state: AppState) {
    tokio::time::sleep(Duration::from_secs(15)).await;
    loop {
        let pause = match pass(&state).await {
            Ok(done) if done > 0 => BUSY_PAUSE,
            Ok(_) => IDLE_PAUSE,
            Err(err) => {
                tracing::error!(err = ?err, "favicon pass failed");
                IDLE_PAUSE
            }
        };
        tokio::time::sleep(pause).await;
    }
}

async fn pass(state: &AppState) -> anyhow::Result<usize> {
    // One at a time: this is a nicety, and a burst of requests to twenty
    // publishers for decoration is not a good use of anyone's server.
    let Some(feed) = store::feed_needing_icon(&state.db).await? else {
        return Ok(0);
    };

    match fetch_icon(&state.http, &feed.site_url, state.cfg.extract_allow_private).await {
        Ok(data_uri) => {
            store::record_icon(&state.db, feed.id, Some(data_uri)).await?;
            tracing::debug!(feed_id = feed.id, "icon stored");
        }
        Err(err) => {
            tracing::debug!(feed_id = feed.id, err = %err, "no icon");
            store::record_icon_failure(&state.db, feed.id).await?;
        }
    }
    Ok(1)
}

/// Find and download a site's icon, as a `data:` URI.
pub async fn fetch_icon(
    http: &Client,
    site_url: &str,
    allow_private: bool,
) -> anyhow::Result<String> {
    let site = Url::parse(site_url)?;
    let mut candidates: Vec<Url> = Vec::new();

    // What the page declares, then the path every browser tries anyway.
    if let Ok(html) = get_text(http, site.as_str(), allow_private).await {
        candidates.extend(icon_links(&html).iter().filter_map(|h| site.join(h).ok()));
    }
    if let Ok(fallback) = site.join("/favicon.ico") {
        candidates.push(fallback);
    }

    for candidate in candidates {
        if let Ok(uri) = download(http, candidate.as_str(), allow_private).await {
            return Ok(uri);
        }
    }
    anyhow::bail!("no icon found for {site_url}")
}

async fn download(http: &Client, url: &str, allow_private: bool) -> anyhow::Result<String> {
    if !allow_private {
        extract::refuse_internal(url).await?;
    }
    let res = http
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await?
        .error_for_status()?;

    // A 404 page served as HTML with a 200 is the usual way `/favicon.ico`
    // "succeeds", and embedding it would put a broken image on every row.
    let mime = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or(v).trim().to_lowercase())
        .unwrap_or_default();
    anyhow::ensure!(
        IMAGE_TYPES.contains(&mime.as_str()),
        "not an image: {mime:?}"
    );

    let bytes = read_capped(res, MAX_ICON_BYTES).await?;
    anyhow::ensure!(!bytes.is_empty(), "empty icon");
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

async fn get_text(http: &Client, url: &str, allow_private: bool) -> anyhow::Result<String> {
    if !allow_private {
        extract::refuse_internal(url).await?;
    }
    let res = http
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await?
        .error_for_status()?;
    let bytes = read_capped(res, MAX_PAGE_BYTES).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
