//! Conditional HTTP GET with a hard size cap.
//!
//! Both halves matter on a 1 GB board: the validators mean a quiet feed costs one
//! 304 per poll instead of a full re-parse, and the cap means a feed that serves
//! a gigabyte — by accident or otherwise — cannot take the process down with it.

use std::time::Duration;

use axum::http::header;
use reqwest::{Client, StatusCode};

/// Default ceiling on a feed document, overridden by `ROSSO_MAX_FEED_MB`.
///
/// 5 MB was the first guess at "generous for even a full-archive Atom file", and
/// a real feed disproved it: danluu.com ships every post in full with no
/// pagination and measures 6.3 MB, so it was refused outright. 16 MB admits that
/// with headroom and is still far under the container's cap, even allowing that
/// feed-rs builds a model several times the size of its input.
pub const DEFAULT_MAX_FEED_MB: usize = 16;

#[derive(Debug)]
pub enum Fetched {
    /// The server confirmed our cached copy is current.
    NotModified,
    /// The server asked us to come back later, and said when. Not a failure of
    /// the feed — an instruction about scheduling, which is why it is a variant
    /// rather than an error.
    RetryAfter(Duration),
    Body {
        bytes: Vec<u8>,
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

/// A status that will answer the same way tomorrow.
///
/// Distinct from an ordinary failure because the response is not "try again
/// later" but "not for you" — the backoff curve has nothing to offer it, and
/// repeating the request weekly for a year is not politeness.
#[derive(Debug)]
pub struct FeedRefused(pub StatusCode);

impl std::fmt::Display for FeedRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.0.as_u16() {
            401 | 403 => "the publisher refuses this client",
            404 | 410 => "the feed is gone",
            _ => "refused",
        };
        write!(f, "{what} ({})", self.0.as_u16())
    }
}

impl std::error::Error for FeedRefused {}

/// `Retry-After`, in either form the spec allows.
///
/// RFC 9110 permits delta-seconds or an HTTP-date, and servers use both. A date
/// already in the past yields zero rather than an error: the server is saying
/// "now is fine", not lying.
fn retry_after(res: &reqwest::Response) -> Option<Duration> {
    let raw = res
        .headers()
        .get(header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .to_string();

    if let Ok(secs) = raw.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let when = chrono::DateTime::parse_from_rfc2822(&raw).ok()?;
    let delta = when.with_timezone(&chrono::Utc) - chrono::Utc::now();
    Some(Duration::from_secs(delta.num_seconds().max(0) as u64))
}

pub async fn conditional_get(
    http: &Client,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
    max_bytes: usize,
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
    // A server that states a wait has said something more specific than our
    // backoff curve knows, so it wins. Checked before `error_for_status` because
    // both statuses that carry it are errors.
    if matches!(
        res.status(),
        StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
    ) {
        if let Some(wait) = retry_after(&res) {
            return Ok(Fetched::RetryAfter(wait));
        }
    }
    if matches!(
        res.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND | StatusCode::GONE
    ) {
        return Err(FeedRefused(res.status()).into());
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
        bytes: read_capped(res, max_bytes).await?,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn with_retry_after(value: &str) -> Option<Duration> {
        let res = axum::http::Response::builder()
            .status(429)
            .header(header::RETRY_AFTER, value)
            .body(Vec::new())
            .unwrap();
        retry_after(&reqwest::Response::from(res))
    }

    #[test]
    fn both_forms_the_spec_allows_are_read() {
        assert_eq!(with_retry_after("120"), Some(Duration::from_secs(120)));
        // An HTTP-date far in the future: the exact figure moves with the clock,
        // so the assertion is that it is read at all and is large.
        let far = with_retry_after("Wed, 21 Oct 2099 07:28:00 GMT").unwrap();
        assert!(far.as_secs() > 86_400, "{far:?}");
    }

    #[test]
    fn a_date_already_past_means_now_rather_than_an_error() {
        // The server is saying "now is fine", not lying — and a negative wait
        // would otherwise underflow into a very long one.
        assert_eq!(
            with_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn nonsense_is_ignored_so_the_ordinary_backoff_applies() {
        assert_eq!(with_retry_after("soon"), None);
        assert_eq!(with_retry_after(""), None);
        // Negative delta-seconds is not a valid delta, so it falls through to
        // the date parse and fails there too.
        assert_eq!(with_retry_after("-5"), None);
    }
}
