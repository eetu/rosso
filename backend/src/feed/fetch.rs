//! Conditional HTTP GET with a hard size cap.
//!
//! Both halves matter on a 1 GB board: the validators mean a quiet feed costs one
//! 304 per poll instead of a full re-parse, and the cap means a feed that serves
//! a gigabyte — by accident or otherwise — cannot take the process down with it.

use axum::http::header;
use reqwest::{Client, StatusCode};

/// Ceiling on a feed document.
///
/// 5 MB was the first guess at "generous for even a full-archive Atom file", and
/// a real feed disproved it: danluu.com ships every post in full with no
/// pagination and measures 6.3 MB, so it was refused outright. 16 MB admits that
/// with headroom and is still far under the container's cap, even allowing that
/// feed-rs builds a model several times the size of its input.
pub const MAX_FEED_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum Fetched {
    /// The server confirmed our cached copy is current.
    NotModified,
    Body {
        bytes: Vec<u8>,
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

pub async fn conditional_get(
    http: &Client,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> anyhow::Result<Fetched> {
    let mut req = http.get(url);
    if let Some(etag) = etag {
        req = req.header(header::IF_NONE_MATCH, etag);
    }
    if let Some(lm) = last_modified {
        req = req.header(header::IF_MODIFIED_SINCE, lm);
    }

    let res = req.send().await?;
    if res.status() == StatusCode::NOT_MODIFIED {
        return Ok(Fetched::NotModified);
    }
    let res = res.error_for_status()?;

    let header_str = |name: header::HeaderName| {
        res.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let etag = header_str(header::ETAG);
    let last_modified = header_str(header::LAST_MODIFIED);

    Ok(Fetched::Body {
        bytes: read_capped(res, MAX_FEED_BYTES).await?,
        etag,
        last_modified,
    })
}

/// Read a body chunk by chunk, refusing anything over `cap`.
///
/// Checking `Content-Length` would not be enough — it is advisory, absent on
/// chunked responses, and trivially wrong. Counting what actually arrives is the
/// only cap that holds.
pub async fn read_capped(mut res: reqwest::Response, cap: usize) -> anyhow::Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    while let Some(chunk) = res.chunk().await? {
        if out.len() + chunk.len() > cap {
            anyhow::bail!("response exceeded {cap} bytes");
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}
