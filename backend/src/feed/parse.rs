//! Bytes → a normalized feed, via feed-rs.
//!
//! Everything a publisher controls is untrusted: HTML is sanitized here, once,
//! before it can reach the database. Nothing downstream re-sanitizes, so nothing
//! downstream may bypass this.

use std::collections::{HashMap, HashSet};

use ammonia::{Builder, UrlRelative};
use chrono::{DateTime, Utc};
use feed_rs::model::{Entry, Feed as RawFeed, Link};
use url::Url;

use super::comments;

/// Below this many characters of text, a body is treated as a teaser rather than
/// the article — the flag the extraction worker keys off.
const FULL_TEXT_CHARS: usize = 400;

#[derive(Debug)]
pub struct ParsedFeed {
    pub title: Option<String>,
    pub site_url: Option<String>,
    pub icon: Option<String>,
    /// RSS `ttl`, in minutes. A publisher asking to be polled less often.
    pub ttl_minutes: Option<u32>,
    pub items: Vec<ParsedItem>,
}

#[derive(Debug)]
pub struct ParsedItem {
    pub guid: String,
    pub url: Option<String>,
    /// Where the discussion lives, when the item has one separate from the
    /// article — the defining shape of an aggregator feed.
    pub comments_url: Option<String>,
    pub title: String,
    pub author: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub content_html: Option<String>,
    pub content_text: Option<String>,
    pub truncated: bool,
}

pub fn parse(bytes: &[u8], feed_url: &str) -> anyhow::Result<ParsedFeed> {
    let raw: RawFeed = feed_rs::parser::parse(bytes)?;
    let base = Url::parse(feed_url).ok();

    // Borrowed for valid UTF-8, which every feed worth reading is.
    let discussions = comments::discussion_urls(&String::from_utf8_lossy(bytes));

    let site_url = pick_site_link(&raw.links, feed_url, base.as_ref());
    let items = raw
        .entries
        .iter()
        .map(|e| parse_entry(e, base.as_ref(), &discussions))
        .collect();

    Ok(ParsedFeed {
        title: raw
            .title
            .map(|t| t.content)
            .filter(|s| !s.trim().is_empty()),
        site_url,
        icon: raw.icon.or(raw.logo).map(|i| i.uri),
        ttl_minutes: raw.ttl,
        items,
    })
}

fn parse_entry(
    entry: &Entry,
    base: Option<&Url>,
    discussions: &HashMap<String, String>,
) -> ParsedItem {
    let url = entry
        .links
        .iter()
        .find(|l| is_html_link(l))
        .or(entry.links.first())
        .map(|l| absolutize(&l.href, base));

    // Prefer the full content element; fall back to the summary, which by
    // definition is a teaser even when it happens to be long.
    let (raw_html, from_summary) = match entry.content.as_ref().and_then(|c| c.body.as_deref()) {
        Some(body) => (Some(body), false),
        None => (entry.summary.as_ref().map(|t| t.content.as_str()), true),
    };

    let item_base = url.as_deref().and_then(|u| Url::parse(u).ok());
    let sanitize_base = item_base.as_ref().or(base);
    let sanitized = raw_html.map(|h| sanitize(h, sanitize_base));

    // A body carrying no prose of its own is worse than no body: rendering it
    // puts "Comments" or "Points: 12" where the article should be. Drop it and
    // let the reader say plainly that the feed shipped nothing.
    let stub = sanitized.as_deref().is_none_or(is_stub);
    let content_html = sanitized.filter(|_| !stub);
    let content_text = content_html.as_deref().map(html_to_text);

    let short = content_text
        .as_deref()
        .is_none_or(|t| t.chars().count() < FULL_TEXT_CHARS);

    let guid = stable_guid(entry, url.as_deref(), content_text.as_deref());

    ParsedItem {
        comments_url: discussion_url(entry, &guid, url.as_deref(), discussions),
        guid,
        url,
        title: entry
            .title
            .as_ref()
            .map(|t| t.content.trim().to_string())
            .unwrap_or_default(),
        author: entry.authors.first().map(|p| p.name.clone()),
        published_at: entry.published.or(entry.updated),
        content_html,
        content_text,
        truncated: from_summary || short,
    }
}

/// Below this many characters of non-link, non-label prose, a body is carrying
/// no article at all. Deliberately small: a real post with under 40 characters
/// of prose has nothing to preview either way.
const MIN_PROSE_CHARS: usize = 40;

/// Does this body consist *only* of links and metadata labels?
///
/// The shape to catch, from two variants of the same aggregator feed:
/// `<a href="…">Comments</a>`, and `Article URL: … / Points: 12 / # Comments: 3`.
/// Strip the anchors and the `Label:` prefixes and nothing is left.
///
/// Two things must survive, and both are easy to swallow by accident:
/// a body that is **only an image** (a webcomic is exactly that, and it is the
/// whole post), and a **genuinely short post** that simply says little. So a
/// short body is a stub only when stripping actually emptied it — not merely
/// because it was short to begin with.
fn is_stub(html: &str) -> bool {
    if has_media(html) {
        return false;
    }
    let full = html_to_text(html);
    let prose = strip_labels(&html_to_text(&strip_anchors(html)));

    if full.chars().count() < MIN_PROSE_CHARS {
        return prose.is_empty();
    }
    prose.chars().count() < MIN_PROSE_CHARS
}

/// An embedded image or player is content in its own right, whatever the text
/// around it says.
fn has_media(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    ["<img", "<video", "<audio", "<picture", "<iframe", "<figure"]
        .iter()
        .any(|tag| lower.contains(tag))
}

/// Remove `<a>` elements *and their text* — link text is navigation, not prose.
fn strip_anchors(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0usize;

    while let Some(found) = lower[cursor..].find("<a") {
        let start = cursor + found;
        // `<article>` also starts with `<a`; require a tag-name boundary.
        let boundary = lower[start + 2..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || c == '>');
        if !boundary {
            out.push_str(&html[cursor..start + 2]);
            cursor = start + 2;
            continue;
        }
        out.push_str(&html[cursor..start]);
        cursor = match lower[start..].find("</a>") {
            Some(end) => start + end + 4,
            // Unclosed anchor: the sanitizer should have balanced it, but drop
            // the remainder rather than emitting half a tag.
            None => html.len(),
        };
    }
    out.push_str(&html[cursor..]);
    out
}

/// Drop `Label:` prefixes (`Points:`, `# Comments:`, `Article URL:`), keeping
/// what a human actually wrote.
///
/// A label is the short run of words immediately before a colon, so only those
/// words are removed. Text with no colon at all is returned untouched — treating
/// it as one big label is how a short real post gets mistaken for metadata.
fn strip_labels(text: &str) -> String {
    if !text.contains(':') {
        return text.to_string();
    }
    // Real labels are one or two words ("Points", "# Comments", "Article URL").
    // Taking three would eat the preceding value as well.
    const LABEL_WORDS: usize = 2;

    let segments: Vec<&str> = text.split(':').collect();
    let last = segments.len() - 1;
    let mut kept: Vec<&str> = Vec::new();
    for (i, segment) in segments.iter().enumerate() {
        let words: Vec<&str> = segment.split_whitespace().collect();
        let take = if i == last {
            words.len()
        } else {
            words.len().saturating_sub(LABEL_WORDS)
        };
        kept.extend(&words[..take]);
    }
    kept.join(" ")
}

/// The discussion URL for an entry: RSS `<comments>` recovered by our own pass,
/// else Atom's `rel="replies"` link, which feed-rs does keep.
fn discussion_url(
    entry: &Entry,
    guid: &str,
    url: Option<&str>,
    discussions: &HashMap<String, String>,
) -> Option<String> {
    let from_rss = discussions
        .get(guid)
        .or_else(|| url.and_then(|u| discussions.get(u)));
    if let Some(found) = from_rss {
        return Some(found.clone());
    }
    entry
        .links
        .iter()
        .find(|l| l.rel.as_deref() == Some("replies"))
        .map(|l| l.href.clone())
        // A discussion URL equal to the article URL is not a second destination.
        .filter(|href| Some(href.as_str()) != url)
}

/// The identity an item keeps across every future poll.
///
/// feed-rs synthesizes `Entry::id` when the source has no guid: from the link and
/// title when it has them (deterministic, and what we want), but **from a fresh
/// random UUID when the entry has neither**. Trusting that blindly would give the
/// same entry a new guid on every poll and re-insert it forever, so we detect
/// exactly that case and hash the content instead.
fn stable_guid(entry: &Entry, url: Option<&str>, content_text: Option<&str>) -> String {
    let feed_rs_had_something_to_hash = !entry.links.is_empty() || entry.title.is_some();
    if !entry.id.trim().is_empty() && feed_rs_had_something_to_hash {
        return entry.id.clone();
    }
    if let Some(url) = url {
        return url.to_string();
    }

    let title = entry
        .title
        .as_ref()
        .map(|t| t.content.as_str())
        .unwrap_or("");
    let stamp = entry
        .published
        .or(entry.updated)
        .map(|d| d.to_rfc3339())
        .unwrap_or_default();
    // Truncated so a publisher's later typo-fix doesn't read as a new item.
    let body: String = content_text.unwrap_or("").chars().take(200).collect();
    format!(
        "rosso:{:016x}",
        fnv1a(&format!("{title}\u{1}{stamp}\u{1}{body}"))
    )
}

/// FNV-1a. Hand-rolled because these hashes are persisted as item identity:
/// `DefaultHasher` makes no cross-version stability guarantee, so a Rust upgrade
/// could silently re-key every affected item.
fn fnv1a(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

fn is_html_link(link: &Link) -> bool {
    let rel_ok = link.rel.as_deref().is_none_or(|r| r == "alternate");
    let type_ok = link
        .media_type
        .as_deref()
        .is_none_or(|t| t.contains("html"));
    rel_ok && type_ok
}

fn pick_site_link(links: &[Link], feed_url: &str, base: Option<&Url>) -> Option<String> {
    links
        .iter()
        .find(|l| is_html_link(l) && l.href != feed_url)
        .or_else(|| links.iter().find(|l| l.href != feed_url))
        .map(|l| absolutize(&l.href, base))
}

fn absolutize(href: &str, base: Option<&Url>) -> String {
    match base.and_then(|b| b.join(href).ok()) {
        Some(u) => u.to_string(),
        None => href.to_string(),
    }
}

/// Sanitize publisher HTML. `url_relative` rewrites relative `src`/`href`
/// against the article's own URL — a feed that ships `/img/1.png` would
/// otherwise resolve against rosso's origin and 404.
pub fn sanitize(html: &str, base: Option<&Url>) -> String {
    let mut builder = Builder::default();
    if let Some(b) = base {
        builder.url_relative(UrlRelative::RewriteWithBase(b.clone()));
    }
    drop_empty_elements(&builder.clean(html).to_string())
}

/// Containers worth deleting when they hold nothing.
///
/// Deliberately excludes list items and table cells: an empty `<li>` still takes
/// a number and an empty `<td>` still holds a column, so removing those would
/// change the document rather than tidy it.
const PRUNABLE: &[&str] = &[
    "p", "span", "div", "section", "article", "header", "footer", "figure", "em", "strong", "b",
    "i", "small", "h1", "h2", "h3", "h4", "h5", "h6",
];

/// How deep a nest of empty wrappers to unwrap. Each pass peels one layer.
const MAX_PRUNE_PASSES: usize = 8;

/// Remove elements that contain nothing but whitespace.
///
/// Stripping a publisher's classes leaves behind their scaffolding: a typical
/// extracted article opens with `<p><span><span></span><span></span></span></p>`,
/// which renders as blank space the reader cannot explain. Only *innermost*
/// empties are matched each pass, so a nest collapses from the inside out and no
/// tag-nesting bookkeeping is needed. Void elements never match, so an `<img>`
/// is never mistaken for an empty container.
fn drop_empty_elements(html: &str) -> String {
    let mut current = html.to_string();
    for _ in 0..MAX_PRUNE_PASSES {
        let next = prune_pass(&current);
        if next == current {
            break;
        }
        current = next;
    }
    current
}

fn prune_pass(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0usize;

    while let Some(found) = lower[cursor..].find('<') {
        let open_start = cursor + found;
        let Some(open_end_rel) = lower[open_start..].find('>') else {
            break;
        };
        let open_end = open_start + open_end_rel + 1;
        let tag = &lower[open_start..open_end];

        let Some(name) = PRUNABLE.iter().find(|name| is_open_tag(tag, name)) else {
            out.push_str(&html[cursor..open_end]);
            cursor = open_end;
            continue;
        };

        // Innermost only: the very next thing must be this element's own close.
        let rest = &lower[open_end..];
        let trimmed = rest.trim_start();
        let close = format!("</{name}>");
        if trimmed.starts_with(&close) {
            out.push_str(&html[cursor..open_start]);
            cursor = open_end + (rest.len() - trimmed.len()) + close.len();
        } else {
            out.push_str(&html[cursor..open_end]);
            cursor = open_end;
        }
    }
    out.push_str(&html[cursor..]);
    out
}

/// `<p>` and `<p class=…>` are the tag; `<pre>` is not.
fn is_open_tag(tag: &str, name: &str) -> bool {
    let Some(rest) = tag.strip_prefix('<').and_then(|t| t.strip_prefix(name)) else {
        return false;
    };
    rest.starts_with('>') || rest.starts_with(char::is_whitespace)
}

/// Plain text for search, summarization input and the teaser heuristic.
pub fn html_to_text(html: &str) -> String {
    let text = Builder::empty()
        .tags(HashSet::new())
        .clean(html)
        .to_string();
    // The tag strip leaves the whitespace that surrounded block elements.
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS: &str = r#"<?xml version="1.0"?>
      <rss version="2.0"><channel>
        <title>Example</title>
        <link>https://example.com/</link>
        <ttl>45</ttl>
        <item>
          <title>First post</title>
          <link>https://example.com/1</link>
          <guid>tag:example.com,2026:1</guid>
          <pubDate>Tue, 01 Sep 2026 10:00:00 GMT</pubDate>
          <description>&lt;p&gt;Hello &lt;script&gt;alert(1)&lt;/script&gt;world&lt;/p&gt;</description>
        </item>
      </channel></rss>"#;

    #[test]
    fn rss_maps_onto_the_normalized_shape() {
        let feed = parse(RSS.as_bytes(), "https://example.com/feed.xml").unwrap();
        assert_eq!(feed.title.as_deref(), Some("Example"));
        assert_eq!(feed.site_url.as_deref(), Some("https://example.com/"));
        assert_eq!(feed.ttl_minutes, Some(45));

        let item = &feed.items[0];
        assert_eq!(item.guid, "tag:example.com,2026:1");
        assert_eq!(item.title, "First post");
        assert_eq!(item.url.as_deref(), Some("https://example.com/1"));
        assert!(item.published_at.is_some());
    }

    #[test]
    fn script_tags_never_survive_parsing() {
        let feed = parse(RSS.as_bytes(), "https://example.com/feed.xml").unwrap();
        let html = feed.items[0].content_html.as_deref().unwrap();
        assert!(
            !html.contains("script"),
            "sanitizer let a script through: {html}"
        );
        assert!(html.contains("Hello"));
        assert_eq!(feed.items[0].content_text.as_deref(), Some("Hello world"));
    }

    #[test]
    fn a_short_body_is_flagged_truncated() {
        let feed = parse(RSS.as_bytes(), "https://example.com/feed.xml").unwrap();
        assert!(feed.items[0].truncated);
    }

    /// The shape `news.ycombinator.com/rss` serves: the body is one anchor whose
    /// text is the word "Comments".
    const HN_OFFICIAL: &str = r#"<?xml version="1.0"?>
      <rss version="2.0"><channel><title>Hacker News</title>
        <link>https://news.ycombinator.com/</link>
        <item>
          <title>JetKVM Mini</title>
          <link>https://jetkvm.com/blog/introducing-jetkvm-mini</link>
          <comments>https://news.ycombinator.com/item?id=49681152</comments>
          <description><![CDATA[<a href="https://news.ycombinator.com/item?id=49681152">Comments</a>]]></description>
        </item>
      </channel></rss>"#;

    /// The shape `hnrss.org/frontpage` serves: four `Label: value` paragraphs.
    const HN_RSS: &str = r#"<?xml version="1.0"?>
      <rss version="2.0"><channel><title>Hacker News: Front Page</title>
        <link>https://news.ycombinator.com/</link>
        <item>
          <title>Libraries Run Rust Inside Python</title>
          <link>https://belderbos.dev/blog/how-libraries-run-rust-inside-python/</link>
          <guid isPermaLink="false">https://news.ycombinator.com/item?id=49685037</guid>
          <comments>https://news.ycombinator.com/item?id=49685037</comments>
          <description><![CDATA[
            <p>Article URL: <a href="https://belderbos.dev/blog/x">https://belderbos.dev/blog/x</a></p>
            <p>Comments URL: <a href="https://news.ycombinator.com/item?id=49685037">https://news.ycombinator.com/item?id=49685037</a></p>
            <p>Points: 12</p>
            <p># Comments: 3</p>
          ]]></description>
        </item>
      </channel></rss>"#;

    #[test]
    fn an_aggregator_stub_body_is_dropped_rather_than_shown() {
        // Rendering these would put the word "Comments" — or "Points: 12" —
        // where the article should be.
        for (name, xml) in [("hn official", HN_OFFICIAL), ("hnrss", HN_RSS)] {
            let feed = parse(xml.as_bytes(), "https://news.ycombinator.com/rss").unwrap();
            let item = &feed.items[0];
            assert!(
                item.content_html.is_none(),
                "{name} kept a stub body: {:?}",
                item.content_html
            );
            assert!(item.truncated, "{name} should still be flagged truncated");
        }
    }

    #[test]
    fn a_real_teaser_survives_stub_detection() {
        let rss = RSS.replace(
            "&lt;p&gt;Hello &lt;script&gt;alert(1)&lt;/script&gt;world&lt;/p&gt;",
            "&lt;p&gt;The new release rewrites the parser so that large documents \
             stream instead of buffering, which cuts peak memory by half.&lt;/p&gt;",
        );
        let feed = parse(rss.as_bytes(), "https://example.com/feed.xml").unwrap();
        assert!(feed.items[0].content_html.is_some());
        assert!(feed.items[0]
            .content_text
            .as_deref()
            .unwrap()
            .contains("cuts peak memory by half"));
    }

    #[test]
    fn the_discussion_url_is_recovered_from_the_comments_element() {
        // feed-rs drops <comments> entirely, so without our own pass an
        // aggregator item has no way to reach its thread.
        for (name, xml) in [("hn official", HN_OFFICIAL), ("hnrss", HN_RSS)] {
            let feed = parse(xml.as_bytes(), "https://news.ycombinator.com/rss").unwrap();
            assert!(
                feed.items[0]
                    .comments_url
                    .as_deref()
                    .is_some_and(|u| u.starts_with("https://news.ycombinator.com/item?id=")),
                "{name} lost its discussion url"
            );
        }
    }

    #[test]
    fn an_ordinary_blog_item_has_no_discussion_url() {
        let feed = parse(RSS.as_bytes(), "https://example.com/feed.xml").unwrap();
        assert!(feed.items[0].comments_url.is_none());
    }

    #[test]
    fn an_image_only_body_is_never_a_stub() {
        // A webcomic's whole post is the image. Dropping it as "no prose" would
        // make the feed unreadable.
        let rss = RSS.replace(
            "&lt;p&gt;Hello &lt;script&gt;alert(1)&lt;/script&gt;world&lt;/p&gt;",
            "&lt;img src=\"https://imgs.xkcd.com/comics/a.png\" title=\"alt text\"&gt;",
        );
        let feed = parse(rss.as_bytes(), "https://example.com/feed.xml").unwrap();
        assert!(
            feed.items[0].content_html.is_some(),
            "dropped an image-only body"
        );
    }

    #[test]
    fn a_genuinely_short_post_is_not_mistaken_for_metadata() {
        // "Hello world" is under the prose floor but is the actual post; only a
        // body that *stripping* emptied counts as a stub.
        let feed = parse(RSS.as_bytes(), "https://example.com/feed.xml").unwrap();
        assert_eq!(feed.items[0].content_text.as_deref(), Some("Hello world"));
    }

    #[test]
    fn labels_are_stripped_but_ordinary_prose_is_not() {
        assert_eq!(strip_labels("Points: 12 # Comments: 3"), "12 3");
        assert_eq!(strip_labels("Hello world"), "Hello world");
        // A colon inside a sentence must not eat the words before it.
        assert_eq!(
            strip_labels("One thing is clear: the parser is faster than before"),
            "One thing the parser is faster than before"
        );
    }

    #[test]
    fn nested_empty_wrappers_collapse_completely() {
        // The exact opener a Guardian liveblog extracts to. Rendered, it is a
        // blank gap the reader cannot account for.
        assert_eq!(
            drop_empty_elements("<p><span><span></span><span></span></span></p><p>Key events</p>"),
            "<p>Key events</p>"
        );
    }

    #[test]
    fn pruning_keeps_anything_that_holds_content() {
        for html in [
            "<p>text</p>",
            "<p><img src=\"https://x/a.png\"></p>",
            "<div><p>nested text</p></div>",
            // Structural emptiness that carries meaning stays put.
            "<ul><li></li><li>b</li></ul>",
            "<table><tr><td></td><td>x</td></tr></table>",
        ] {
            assert_eq!(drop_empty_elements(html), html, "changed {html}");
        }
    }

    #[test]
    fn pruning_does_not_confuse_a_prefix_for_a_tag() {
        // `<pre>` starts with "p" and `<section>` with "s"; neither is `<p>`.
        assert_eq!(drop_empty_elements("<pre>  </pre>"), "<pre>  </pre>");
        assert_eq!(drop_empty_elements("<p class=\"x\">  </p>"), "");
    }

    #[test]
    fn stripping_anchors_leaves_surrounding_prose_and_spares_other_tags() {
        assert_eq!(
            strip_anchors("<p>before <a href=\"x\">link</a> after</p>"),
            "<p>before  after</p>"
        );
        // `<article>` also begins with `<a`.
        assert_eq!(
            strip_anchors("<article>kept</article>"),
            "<article>kept</article>"
        );
    }

    #[test]
    fn relative_urls_resolve_against_the_article_not_our_origin() {
        let rss = RSS.replace(
            "&lt;p&gt;Hello &lt;script&gt;alert(1)&lt;/script&gt;world&lt;/p&gt;",
            "&lt;img src=\"/img/1.png\"&gt;",
        );
        let feed = parse(rss.as_bytes(), "https://example.com/feed.xml").unwrap();
        let html = feed.items[0].content_html.as_deref().unwrap();
        assert!(html.contains("https://example.com/img/1.png"), "got {html}");
    }

    #[test]
    fn an_entry_without_a_guid_keeps_the_same_identity_across_polls() {
        let rss = RSS.replace("<guid>tag:example.com,2026:1</guid>", "");
        let first = parse(rss.as_bytes(), "https://example.com/feed.xml").unwrap();
        let second = parse(rss.as_bytes(), "https://example.com/feed.xml").unwrap();
        assert_eq!(first.items[0].guid, second.items[0].guid);
    }

    #[test]
    fn an_entry_with_nothing_stable_does_not_get_a_fresh_id_every_poll() {
        // feed-rs falls back to a random UUID for an entry with no id, no link
        // and no title. Taking that at face value would re-insert the entry on
        // every single poll.
        let rss = r#"<?xml version="1.0"?>
          <rss version="2.0"><channel><title>Bare</title>
            <item><description>a body with no title and no link</description></item>
          </channel></rss>"#;
        let first = parse(rss.as_bytes(), "https://example.com/feed.xml").unwrap();
        let second = parse(rss.as_bytes(), "https://example.com/feed.xml").unwrap();
        assert_eq!(first.items[0].guid, second.items[0].guid);
        assert!(first.items[0].guid.starts_with("rosso:"));
    }

    #[test]
    fn a_publisher_supplied_guid_is_used_verbatim() {
        let feed = parse(RSS.as_bytes(), "https://example.com/feed.xml").unwrap();
        assert_eq!(feed.items[0].guid, "tag:example.com,2026:1");
    }

    #[test]
    fn atom_parses_through_the_same_path() {
        let atom = r#"<?xml version="1.0"?>
          <feed xmlns="http://www.w3.org/2005/Atom">
            <title>Atom example</title>
            <link rel="alternate" type="text/html" href="https://example.org/"/>
            <entry>
              <id>urn:uuid:1</id>
              <title>Atom post</title>
              <link href="https://example.org/post"/>
              <updated>2026-09-01T10:00:00Z</updated>
              <content type="html">&lt;p&gt;body&lt;/p&gt;</content>
            </entry>
          </feed>"#;
        let feed = parse(atom.as_bytes(), "https://example.org/atom.xml").unwrap();
        assert_eq!(feed.title.as_deref(), Some("Atom example"));
        assert_eq!(feed.items[0].title, "Atom post");
        assert_eq!(feed.site_url.as_deref(), Some("https://example.org/"));
    }

    #[test]
    fn malformed_input_errors_rather_than_panicking() {
        assert!(parse(b"<html>not a feed</html>", "https://example.com/x").is_err());
    }
}
