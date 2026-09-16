//! Full-text extraction for items whose feed shipped only a teaser — or, as with
//! an aggregator, nothing at all.
//!
//! Two paths onto the same function. A background worker drains the backlog at
//! one request at a time, and opening an un-extracted item extracts it inline so
//! the reader shows an article rather than an empty pane. Both are idempotent;
//! whichever gets there first wins.
//!
//! Everything here fetches pages rosso was pointed at by a subscribed feed, so
//! the controls are the same as for feeds: a hard size cap, a bounded timeout,
//! one request at a time, and a minimum gap between hits on the same host.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use dom_smoothie::{Config as ReadabilityConfig, Readability};
use reqwest::Client;
use url::Url;

use crate::feed::fetch::read_capped;
use crate::feed::parse::{html_to_text, sanitize};
use crate::store::{self, PendingExtraction};
use crate::AppState;

/// Article pages are bigger than feeds but not unbounded.
const MAX_ARTICLE_BYTES: usize = 2 * 1024 * 1024;

/// Below this much text, readability found navigation rather than an article.
/// Keeping it would replace an honest "no content" with a cookie banner.
const MIN_ARTICLE_CHARS: usize = 200;

/// Give up on an item after this many failures, so a page that will never parse
/// stops costing a request on every pass.
pub const MAX_ATTEMPTS: i64 = 3;

/// How long an inline extraction may hold a request open before the reader gives
/// up and renders what it already has.
const INLINE_TIMEOUT: Duration = Duration::from_secs(12);

/// A wait between two hits on the same host. Cheap politeness: a feed's items all
/// point at one site, and 30 of them arriving at once should not read as a burst.
const PER_HOST_GAP: Duration = Duration::from_secs(1);

pub struct Extracted {
    pub html: String,
    pub text: String,
}

/// Fetch `url` and reduce it to the article.
pub async fn extract(http: &Client, url: &str, allow_private: bool) -> anyhow::Result<Extracted> {
    if !allow_private {
        refuse_internal(url).await?;
    }
    let res = http.get(url).send().await?.error_for_status()?;

    // A PDF or an image would otherwise be fed to an HTML parser as bytes.
    let content_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !content_type.is_empty() && !content_type.contains("html") {
        anyhow::bail!("not html: {content_type}");
    }

    let bytes = read_capped(res, MAX_ARTICLE_BYTES).await?;
    let page = String::from_utf8_lossy(&bytes).into_owned();
    let base = Url::parse(url).ok();
    let document_url = url.to_string();

    // Readability walks the whole DOM and its types are not Send, so the parse
    // lives entirely inside the blocking pool and only Strings come back.
    let (raw_html, raw_text) =
        tokio::task::spawn_blocking(move || -> anyhow::Result<(String, String)> {
            let cfg = ReadabilityConfig {
                max_elements_to_parse: 30_000,
                ..Default::default()
            };
            let mut readability =
                Readability::new(page.as_str(), Some(document_url.as_str()), Some(cfg))?;
            let article = readability.parse()?;
            Ok((
                article.content.to_string(),
                article.text_content.to_string(),
            ))
        })
        .await??;

    if raw_text.chars().count() < MIN_ARTICLE_CHARS {
        anyhow::bail!("no article found on the page");
    }

    // Same sanitizer as the feed path: a page rosso fetched is no more trusted
    // than a body a publisher handed us. Readability resolves relative URLs
    // itself when given a document_url; passing the base again costs nothing and
    // covers the attributes it does not rewrite.
    let html = sanitize(&raw_html, base.as_ref());
    let text = html_to_text(&html);
    Ok(Extracted { html, text })
}

/// Refuse to fetch anything that is not on the public internet.
///
/// Item URLs are chosen by the publisher, not the user, so extraction is a
/// request rosso makes on a stranger's say-so. Without this, a feed could point
/// an item at `http://127.0.0.1:…`, at the LAN model host, or at a cloud
/// metadata endpoint, and rosso would fetch it and render the response into the
/// reader.
///
/// The host is **resolved** before the check rather than only pattern-matched:
/// most readers test the literal address and so miss a name that resolves
/// inward, which is the easier attack to mount. This still leaves a DNS
/// rebinding window — reqwest resolves again when it connects, and a record that
/// changes in between would slip past. Closing that needs a custom connector
/// bound to the address checked here; the size cap and the fact that only GET is
/// ever issued are what bound the damage until then.
pub async fn refuse_internal(url: &str) -> anyhow::Result<()> {
    let parsed = Url::parse(url)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!("refusing scheme {}", parsed.scheme());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("no host in {url}"))?;
    let port = parsed.port_or_known_default().unwrap_or(80);

    let mut resolved = 0usize;
    for addr in tokio::net::lookup_host((host, port)).await? {
        resolved += 1;
        if !is_public(addr.ip()) {
            anyhow::bail!("refusing to fetch internal address {}", addr.ip());
        }
    }
    if resolved == 0 {
        anyhow::bail!("could not resolve {host}");
    }
    Ok(())
}

/// Is this address on the public internet?
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local() // 169.254/16 — cloud metadata lives here
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || a == 0
                || (a == 100 && (b & 0xc0) == 64) // 100.64/10, carrier NAT
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(mapped));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00 // unique local
                || (first & 0xffc0) == 0xfe80) // link local
        }
    }
}

/// Extract one pending item and record the outcome. Never returns an error: a
/// page that 404s or defeats readability is an ordinary event.
pub async fn run_one(state: &AppState, pending: &PendingExtraction) {
    match extract(&state.http, &pending.url, state.cfg.extract_allow_private).await {
        Ok(article) => {
            if let Err(err) =
                store::record_extraction(&state.db, pending.id, article.html, article.text).await
            {
                tracing::error!(item = pending.id, err = ?err, "could not store extraction");
            } else {
                tracing::debug!(item = pending.id, url = %pending.url, "extracted");
            }
        }
        Err(err) => {
            tracing::debug!(item = pending.id, url = %pending.url, err = %err, "extraction failed");
            if let Err(db_err) =
                store::record_extraction_failure(&state.db, pending.id, err.to_string()).await
            {
                tracing::error!(item = pending.id, err = ?db_err, "could not record failure");
            }
        }
    }
}

/// Extract on demand, for an item the reader just opened.
///
/// Bounded by [`INLINE_TIMEOUT`] so a slow publisher delays the response rather
/// than hanging it; the background worker will retry what times out here.
pub async fn run_inline(state: &AppState, pending: &PendingExtraction) {
    if tokio::time::timeout(INLINE_TIMEOUT, run_one(state, pending))
        .await
        .is_err()
    {
        tracing::debug!(
            item = pending.id,
            "inline extraction timed out; left to the worker"
        );
    }
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move { run(state).await });
}

async fn run(state: AppState) {
    if !state.cfg.extract_enabled {
        tracing::info!("extraction disabled (ROSSO_EXTRACT=0)");
        return;
    }
    // Behind the poller's own boot delay: there is nothing to extract until a
    // poll has run.
    tokio::time::sleep(Duration::from_secs(15)).await;

    let mut last_hit: HashMap<String, Instant> = HashMap::new();
    loop {
        match store::due_for_extraction(&state.db, 10).await {
            Ok(pending) if !pending.is_empty() => {
                for item in &pending {
                    wait_for_host(&mut last_hit, &item.url).await;
                    run_one(&state, item).await;
                }
            }
            Ok(_) => {}
            Err(err) => tracing::error!(err = ?err, "could not list items to extract"),
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

/// Sleep until at least [`PER_HOST_GAP`] has passed since this host was last hit.
async fn wait_for_host(last_hit: &mut HashMap<String, Instant>, url: &str) {
    let Some(host) = Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
    else {
        return;
    };
    if let Some(previous) = last_hit.get(&host) {
        let elapsed = previous.elapsed();
        if elapsed < PER_HOST_GAP {
            tokio::time::sleep(PER_HOST_GAP - elapsed).await;
        }
    }
    last_hit.insert(host, Instant::now());
    // The map only ever holds the hosts of one batch's worth of items.
    if last_hit.len() > 512 {
        last_hit.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn internal_addresses_are_not_public() {
        for addr in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.155",   // the LAN model host
            "169.254.169.254", // cloud metadata
            "0.0.0.0",
            "100.64.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1", // the loopback wearing an IPv6 hat
        ] {
            assert!(!is_public(ip(addr)), "{addr} passed as public");
        }
    }

    #[test]
    fn ordinary_public_addresses_are_allowed() {
        for addr in ["1.1.1.1", "93.184.216.34", "172.32.0.1", "2606:4700::1111"] {
            assert!(is_public(ip(addr)), "{addr} was refused");
        }
    }

    #[tokio::test]
    async fn a_url_pointing_inward_is_refused_before_any_request() {
        for url in [
            "http://127.0.0.1:11434/api/tags",
            "http://192.168.1.155:11434/api/tags",
            "http://[::1]/",
            "file:///etc/passwd",
        ] {
            assert!(refuse_internal(url).await.is_err(), "{url} was allowed");
        }
    }
}
