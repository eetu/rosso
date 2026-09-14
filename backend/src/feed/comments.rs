//! Recover the RSS `<comments>` element, which feed-rs drops.
//!
//! It matters for aggregators: on Hacker News, Lobsters or a Reddit feed the
//! `link` is someone else's article and the discussion lives at a second URL.
//! Without it the reader can only offer "open article", which is the half of an
//! aggregator item people care about least.
//!
//! Atom's equivalent is `<link rel="replies">`, which feed-rs *does* keep, so
//! that case is handled from the parsed entry instead (see `parse`).

use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::Reader;

/// Discussion URL per item, keyed by **both** the item's guid and its link, so
/// the lookup succeeds whichever of the two ended up as our guid.
pub fn discussion_urls(xml: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    // Almost no feed carries the element; skip the whole pass when it is absent
    // rather than walking a multi-megabyte document for nothing.
    if !xml.contains("<comments") {
        return out;
    }

    let mut reader = Reader::from_str(xml);
    let mut field: Option<Field> = None;
    // Text arrives in pieces: quick-xml ends a Text event at every entity
    // reference, so `&amp;` in a query string splits one URL across three
    // events. Accumulate and commit at the closing tag rather than taking the
    // first piece, which would silently truncate the URL.
    let mut buf = String::new();
    let mut item: Option<Item> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                match e.name().as_ref() {
                    "item" => item = Some(Item::default()),
                    "guid" => field = Some(Field::Guid),
                    "link" => field = Some(Field::Link),
                    "comments" => field = Some(Field::Comments),
                    _ => field = None,
                }
                buf.clear();
            }
            Ok(Event::Text(t)) if field.is_some() => buf.push_str(&t.xml10_content()),
            Ok(Event::CData(t)) if field.is_some() => buf.push_str(&t.into_inner()),
            Ok(Event::GeneralRef(r)) if field.is_some() => {
                if let Some(resolved) = resolve_entity(&r.clone().into_inner()) {
                    buf.push_str(&resolved);
                }
            }
            Ok(Event::End(e)) => {
                if let Some(which) = field.take() {
                    if let Some(item) = item.as_mut() {
                        let value = buf.trim();
                        if !value.is_empty() {
                            which.set(item, value.to_string());
                        }
                    }
                }
                buf.clear();

                if e.name().as_ref() == "item" {
                    if let Some(done) = item.take() {
                        if let Some(comments) = done.comments {
                            for key in [done.guid, done.link].into_iter().flatten() {
                                out.insert(key, comments.clone());
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            // A malformed document is not worth failing a poll over — the feed
            // itself already parsed, this pass is an extra.
            Err(_) => break,
            _ => {}
        }
    }
    out
}

/// The five entities XML predefines, plus numeric character references. Anything
/// else in a URL would be a DTD entity, which feeds do not use.
fn resolve_entity(name: &str) -> Option<String> {
    match name {
        "amp" => Some("&".into()),
        "lt" => Some("<".into()),
        "gt" => Some(">".into()),
        "quot" => Some("\"".into()),
        "apos" => Some("'".into()),
        _ => quick_xml::events::BytesRef::new(name.to_string())
            .resolve_char_ref()
            .ok()
            .flatten()
            .map(|c| c.to_string()),
    }
}

#[derive(Clone, Copy)]
enum Field {
    Guid,
    Link,
    Comments,
}

impl Field {
    fn set(self, item: &mut Item, value: String) {
        let slot = match self {
            Field::Guid => &mut item.guid,
            Field::Link => &mut item.link,
            Field::Comments => &mut item.comments,
        };
        // First value wins: a nested element would otherwise overwrite the
        // item's own.
        slot.get_or_insert(value);
    }
}

#[derive(Default)]
struct Item {
    guid: Option<String>,
    link: Option<String>,
    comments: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hacker_news_style_items_yield_their_discussion_url() {
        // The shape news.ycombinator.com/rss serves: no guid, and the only thing
        // distinguishing the discussion from the article is <comments>.
        let xml = r#"<rss><channel><item>
            <title>JetKVM Mini</title>
            <link>https://jetkvm.com/blog/introducing-jetkvm-mini</link>
            <comments>https://news.ycombinator.com/item?id=49681152</comments>
          </item></channel></rss>"#;
        let map = discussion_urls(xml);
        assert_eq!(
            map.get("https://jetkvm.com/blog/introducing-jetkvm-mini")
                .map(String::as_str),
            Some("https://news.ycombinator.com/item?id=49681152")
        );
    }

    #[test]
    fn both_guid_and_link_key_the_same_discussion() {
        // hnrss sets guid to the discussion and link to the article; we cannot
        // know which one parse() picked, so both must resolve.
        let xml = r#"<rss><channel><item>
            <link>https://example.com/article</link>
            <guid isPermaLink="false">https://news.ycombinator.com/item?id=1</guid>
            <comments>https://news.ycombinator.com/item?id=1</comments>
          </item></channel></rss>"#;
        let map = discussion_urls(xml);
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("https://example.com/article"));
        assert!(map.contains_key("https://news.ycombinator.com/item?id=1"));
    }

    #[test]
    fn entities_in_the_url_are_decoded() {
        let xml = r#"<rss><channel><item>
            <link>https://example.com/a</link>
            <comments>https://example.com/c?id=1&amp;sort=new</comments>
          </item></channel></rss>"#;
        assert_eq!(
            discussion_urls(xml)
                .get("https://example.com/a")
                .map(String::as_str),
            Some("https://example.com/c?id=1&sort=new")
        );
    }

    #[test]
    fn a_feed_without_the_element_costs_nothing_and_returns_nothing() {
        let xml =
            r#"<rss><channel><item><link>https://example.com/a</link></item></channel></rss>"#;
        assert!(discussion_urls(xml).is_empty());
    }

    #[test]
    fn a_truncated_document_does_not_panic() {
        assert!(discussion_urls("<rss><channel><item><comments>https://x/").is_empty());
    }
}
