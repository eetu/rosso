//! Turn whatever the user pasted into a feed URL.
//!
//! People paste the site, not the feed. So: fetch the URL, try to parse it as a
//! feed, and if that fails look for the `<link rel="alternate">` tags that pages
//! advertise their feeds with.

use reqwest::Client;
use url::Url;

use super::fetch::{read_capped, MAX_FEED_BYTES};
use super::parse::{self, ParsedFeed};

/// A resolved feed: the URL to poll from now on, plus the first parse of it, so
/// adding a feed costs one request rather than two.
pub struct Discovered {
    pub feed_url: String,
    pub parsed: ParsedFeed,
}

pub async fn discover(http: &Client, input: &str) -> anyhow::Result<Discovered> {
    let start = normalize(input)?;
    let body = get_text(http, start.as_str()).await?;

    if let Ok(parsed) = parse::parse(body.as_bytes(), start.as_str()) {
        return Ok(Discovered {
            feed_url: start.to_string(),
            parsed,
        });
    }

    for href in feed_links(&body) {
        let Ok(candidate) = start.join(&href) else {
            continue;
        };
        let Ok(body) = get_text(http, candidate.as_str()).await else {
            continue;
        };
        if let Ok(parsed) = parse::parse(body.as_bytes(), candidate.as_str()) {
            return Ok(Discovered {
                feed_url: candidate.to_string(),
                parsed,
            });
        }
    }

    anyhow::bail!("no feed found at {start}")
}

/// Accept `example.com/feed` as readily as a full URL.
fn normalize(input: &str) -> anyhow::Result<Url> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty url");
    }
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let url = Url::parse(&candidate)?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => anyhow::bail!("unsupported scheme: {other}"),
    }
}

async fn get_text(http: &Client, url: &str) -> anyhow::Result<String> {
    let res = http.get(url).send().await?.error_for_status()?;
    let bytes = read_capped(res, MAX_FEED_BYTES).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Hrefs of the `<link rel="alternate">` tags that point at a feed, in document
/// order.
///
/// Deliberately a scan rather than a DOM parse: the input is one tag shape in the
/// head, and a full HTML parser is a large dependency to carry for it.
fn feed_links(html: &str) -> Vec<String> {
    // ASCII-only lowering, so byte offsets stay aligned with the original.
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut cursor = 0usize;

    while let Some(rel) = lower[cursor..].find("<link") {
        let start = cursor + rel;
        let Some(end_rel) = lower[start..].find('>') else {
            break;
        };
        let end = start + end_rel;
        let tag = &lower[start..end];

        let is_alternate = attr(tag, "rel").is_some_and(|r| r.contains("alternate"));
        let is_feed = attr(tag, "type").is_some_and(|t| {
            t.contains("rss+xml") || t.contains("atom+xml") || t.contains("feed+json")
        });
        if is_alternate && is_feed {
            // Re-read href from the original so the URL keeps its case.
            if let Some(href) = attr_raw(&html[start..end], "href") {
                out.push(href);
            }
        }
        cursor = end + 1;
    }
    out
}

fn attr(tag_lower: &str, name: &str) -> Option<String> {
    attr_raw(tag_lower, name)
}

fn attr_raw(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0usize;
    loop {
        let at = from + lower[from..].find(name)?;
        // Must be a standalone attribute name, not a suffix of another one.
        let preceded_ok = at == 0
            || lower[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace());
        let rest = &lower[at + name.len()..];
        let trimmed = rest.trim_start();
        if preceded_ok && trimmed.starts_with('=') {
            let value_start = at + name.len() + (rest.len() - trimmed.len()) + 1;
            let value = tag[value_start..].trim_start();
            let quote = value.chars().next()?;
            let (delim, body) = if quote == '"' || quote == '\'' {
                (quote, &value[1..])
            } else {
                (' ', value)
            };
            let end = body.find(delim).unwrap_or(body.len());
            return Some(body[..end].to_string());
        }
        from = at + name.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_host_gets_a_scheme() {
        assert_eq!(
            normalize("example.com/feed").unwrap().as_str(),
            "https://example.com/feed"
        );
    }

    #[test]
    fn non_http_schemes_are_refused() {
        // The fetch path would otherwise hand a file:// URL straight to reqwest.
        assert!(normalize("file:///etc/passwd").is_err());
        assert!(normalize("  ").is_err());
    }

    #[test]
    fn alternate_link_tags_are_found_in_document_order() {
        let html = r#"<html><head>
          <link rel="stylesheet" href="/style.css">
          <link rel="alternate" type="application/rss+xml" title="RSS" href="/Feed.XML">
          <link rel="alternate" type="application/atom+xml" href="https://other.example/atom">
        </head></html>"#;
        assert_eq!(
            feed_links(html),
            vec!["/Feed.XML", "https://other.example/atom"]
        );
    }

    #[test]
    fn a_page_with_no_feed_yields_nothing() {
        assert!(feed_links("<html><head><title>hi</title></head></html>").is_empty());
    }

    #[test]
    fn single_quoted_and_unquoted_attributes_parse() {
        let html = "<link rel='alternate' type='application/rss+xml' href='/a.xml'>\
                    <link rel=alternate type=application/atom+xml href=/b.xml>";
        assert_eq!(feed_links(html), vec!["/a.xml", "/b.xml"]);
    }
}
