//! OPML — the way a subscription list gets in and out of rosso.
//!
//! This is the migration path, so import is deliberately forgiving: anything
//! that carries an `xmlUrl` is a subscription and everything else in the file is
//! ignored. Readers disagree about where the title lives (`title` or `text`),
//! whether `type="rss"` is present, and how deeply outlines nest, and none of
//! that changes which feeds you are subscribed to.
//!
//! **Folders are flattened.** rosso stores a `folder_id` but has no UI for one,
//! so importing a foldered file would create structure nothing can show and
//! exporting would claim a hierarchy that was never used.

use quick_xml::escape::escape;
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};

use crate::store::Feed;

/// One subscription lifted out of an OPML file.
#[derive(Debug, PartialEq)]
pub struct Outline {
    pub xml_url: String,
    pub title: Option<String>,
    pub site_url: Option<String>,
}

/// Every subscription in the document, in file order.
///
/// A malformed file yields whatever was read before the break rather than an
/// error: half an import is more use than none, and the caller reports the count.
pub fn parse(xml: &str) -> Vec<Outline> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();

    loop {
        // `outline` is empty-element in most files and a container in foldered
        // ones, so both event kinds carry subscriptions.
        let tag = match reader.read_event() {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => e,
            Ok(Event::Eof) | Err(_) => break,
            _ => continue,
        };
        if tag.name().as_ref() != "outline" {
            continue;
        }

        // Attribute names are matched lower-cased: readers write `xmlUrl`,
        // `xmlurl` and `XMLURL`, and a case-sensitive match would import nothing
        // from one of them while looking like it worked.
        let attrs: Vec<(String, String)> = tag
            .attributes()
            .flatten()
            .filter_map(|a| {
                let key: &str = a.key.as_ref();
                let key = key.to_lowercase();
                let value = a
                    .normalized_value(XmlVersion::Implicit1_0)
                    .ok()?
                    .trim()
                    .to_string();
                (!value.is_empty()).then_some((key, value))
            })
            .collect();

        let find = |name: &str| {
            attrs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };
        // No `xmlUrl` means a folder, or one of the outline kinds OPML also
        // covers (a link, a note). Not a subscription either way.
        let Some(xml_url) = find("xmlurl") else {
            continue;
        };
        out.push(Outline {
            xml_url,
            title: find("title").or_else(|| find("text")),
            site_url: find("htmlurl"),
        });
    }
    out
}

/// The subscription list as an OPML 2.0 document.
pub fn render(feeds: &[Feed], now: &str) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <opml version=\"2.0\">\n  <head>\n    <title>rosso subscriptions</title>\n",
    );
    xml.push_str(&format!("    <dateCreated>{}</dateCreated>\n", escape(now)));
    xml.push_str("  </head>\n  <body>\n");
    for feed in feeds {
        // `text` is the attribute OPML actually requires and some readers show
        // only that one; `title` is the attribute most readers write. Both, then.
        let title = escape(&feed.title);
        xml.push_str(&format!(
            "    <outline type=\"rss\" text=\"{title}\" title=\"{title}\" xmlUrl=\"{}\"",
            escape(&feed.url)
        ));
        if let Some(site) = feed.site_url.as_deref() {
            xml.push_str(&format!(" htmlUrl=\"{}\"", escape(site)));
        }
        xml.push_str("/>\n");
    }
    xml.push_str("  </body>\n</opml>\n");
    xml
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_flatten_and_non_subscriptions_are_skipped() {
        let xml = r#"<?xml version="1.0"?>
            <opml version="1.0"><body>
              <outline text="Tech">
                <outline type="rss" text="Fallback" xmlUrl="https://a.example/feed"/>
                <outline type="rss" title="Named" text="Ignored"
                         xmlUrl="https://b.example/feed" htmlUrl="https://b.example/"/>
              </outline>
              <outline text="A note with no feed"/>
            </body></opml>"#;

        assert_eq!(
            parse(xml),
            vec![
                Outline {
                    xml_url: "https://a.example/feed".into(),
                    // No `title`, so `text` stands in.
                    title: Some("Fallback".into()),
                    site_url: None,
                },
                Outline {
                    xml_url: "https://b.example/feed".into(),
                    title: Some("Named".into()),
                    site_url: Some("https://b.example/".into()),
                },
            ]
        );
    }

    #[test]
    fn attribute_case_and_entities_survive() {
        // Readers write `xmlUrl`, `xmlurl` and `XMLURL`; a case-sensitive match
        // would silently import nothing from one of them.
        let xml = r#"<opml><body>
            <outline XMLURL="https://x.example/feed?a=1&amp;b=2" TITLE="Q &amp; A"/>
        </body></opml>"#;
        let found = parse(xml);
        assert_eq!(found[0].xml_url, "https://x.example/feed?a=1&b=2");
        assert_eq!(found[0].title.as_deref(), Some("Q & A"));
    }

    #[test]
    fn a_truncated_file_keeps_what_it_read() {
        let xml = r#"<opml><body>
            <outline type="rss" xmlUrl="https://a.example/feed"/>
            <outline type="rss" xmlUrl="https://b.exa"#;
        assert_eq!(parse(xml).len(), 1);
    }

    #[test]
    fn an_export_reimports_as_itself() {
        let feeds = vec![Feed {
            id: 1,
            url: "https://a.example/feed?x=1&y=2".into(),
            site_url: Some("https://a.example/".into()),
            title: "Ampersands & \"quotes\"".into(),
            folder_id: None,
            icon: None,
            unread: 3,
            last_fetch_at: None,
            next_fetch_at: "2026-09-16T00:00:00Z".into(),
            last_error: None,
            disabled: false,
        }];
        let round_tripped = parse(&render(&feeds, "2026-09-16T00:00:00Z"));
        assert_eq!(
            round_tripped,
            vec![Outline {
                xml_url: feeds[0].url.clone(),
                title: Some(feeds[0].title.clone()),
                site_url: feeds[0].site_url.clone(),
            }]
        );
    }
}
