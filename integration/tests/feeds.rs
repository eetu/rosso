//! The subscribe → poll → read loop, against a wiremock feed server.
//!
//! Real backend, real SQLite, stubbed publisher. Nothing here needs the network
//! or the LLM.

use rosso_integration::Stack;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn rss(items: &[(&str, &str)]) -> String {
    let entries: String = items
        .iter()
        .map(|(guid, title)| {
            format!(
                "<item><title>{title}</title><link>https://example.test/{guid}</link>\
                 <guid>{guid}</guid>\
                 <pubDate>Tue, 01 Sep 2026 10:00:00 GMT</pubDate>\
                 <description>body of {title}</description></item>"
            )
        })
        .collect();
    format!(
        "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel>\
         <title>Test feed</title><link>https://example.test/</link>{entries}</channel></rss>"
    )
}

async fn feed_server(body: String) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(body, "application/rss+xml")
                .insert_header("etag", "\"v1\""),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn subscribing_stores_the_feed_and_its_items_immediately() {
    let server = feed_server(rss(&[("a", "First"), ("b", "Second")])).await;
    let stack = Stack::start().await.unwrap();

    let feed = stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;
    assert_eq!(feed["title"], "Test feed");
    // A new feed must have content straight away, not after the first tick.
    assert_eq!(feed["unread"], 2);

    let items = stack.get_json("/api/items").await;
    assert_eq!(items["items"].as_array().unwrap().len(), 2);
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn items_can_be_filtered_by_feed_without_a_cursor() {
    // The optional filters are bound parameters that must still appear in the
    // SQL when unset — dropping the clause instead makes rusqlite reject the
    // binding and turns an ordinary list request into a 500.
    let server = feed_server(rss(&[("a", "First")])).await;
    let stack = Stack::start().await.unwrap();
    let feed = stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;
    let id = feed["id"].as_i64().unwrap();

    for route in [
        "/api/items".to_string(),
        format!("/api/items?feed_id={id}"),
        "/api/items?view=all".to_string(),
        "/api/items?view=starred".to_string(),
        format!("/api/items?feed_id={id}&view=all&limit=5"),
    ] {
        let res = stack.get(&route).await;
        assert!(res.status().is_success(), "GET {route} → {}", res.status());
    }
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn re_polling_the_same_content_inserts_nothing() {
    let server = feed_server(rss(&[("a", "First"), ("b", "Second")])).await;
    let stack = Stack::start().await.unwrap();
    let feed = stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;
    let id = feed["id"].as_i64().unwrap();

    let refreshed = stack
        .post_json(&format!("/api/feeds/{id}/refresh"), json!({}))
        .await;
    assert_eq!(refreshed["last_error"], serde_json::Value::Null);

    let items = stack.get_json("/api/items?view=all").await;
    assert_eq!(
        items["items"].as_array().unwrap().len(),
        2,
        "a re-poll duplicated items"
    );
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_failing_feed_is_recorded_rather_than_erroring_the_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(rss(&[("a", "First")]), "application/rss+xml"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let stack = Stack::start().await.unwrap();
    let feed = stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;
    let id = feed["id"].as_i64().unwrap();

    // The publisher now 500s. Refresh must still answer 200 with the failure
    // recorded on the feed — a broken feed is an ordinary state, not an error.
    let refreshed = stack
        .post_json(&format!("/api/feeds/{id}/refresh"), json!({}))
        .await;
    assert!(
        refreshed["last_error"].is_string(),
        "expected the error on the feed row, got {refreshed}"
    );
    // And the items already fetched are untouched.
    assert_eq!(
        stack.get_json("/api/items").await["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn an_aggregator_feed_yields_links_rather_than_a_fake_body() {
    // The shape news.ycombinator.com/rss serves: the description is one anchor
    // reading "Comments", and the thread URL is in <comments>, which feed-rs
    // discards. Both halves have to survive the round trip to the SPA.
    //
    // Extraction is off here so this tests stub suppression alone — and so the
    // item's off-site link is never fetched. Every URL a test hands the backend
    // must be one the test itself serves; an item link pointing at the real
    // internet makes the suite depend on someone else's uptime.
    let server = MockServer::start().await;
    let body = format!(
        "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel>\
        <title>Hacker News</title><link>https://news.ycombinator.com/</link>\
        <item><title>JetKVM Mini</title>\
        <link>{base}/article</link>\
        <comments>https://news.ycombinator.com/item?id=49681152</comments>\
        <description><![CDATA[<a href=\"https://news.ycombinator.com/item?id=49681152\">Comments</a>]]></description>\
        </item></channel></rss>",
        base = server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/rss"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/rss+xml"))
        .mount(&server)
        .await;

    let stack = Stack::start_with_env(&[("ROSSO_EXTRACT", "0")])
        .await
        .unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/rss" }))
        .await;

    let items = stack.get_json("/api/items").await;
    let id = items["items"][0]["id"].as_i64().unwrap();
    assert_eq!(
        items["items"][0]["comments_url"],
        "https://news.ycombinator.com/item?id=49681152"
    );

    let detail = stack.get_json(&format!("/api/items/{id}")).await;
    assert_eq!(
        detail["content_html"],
        serde_json::Value::Null,
        "the word \"Comments\" was stored as if it were the article"
    );
    assert_eq!(detail["url"], server.uri() + "/article");
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn subscribing_twice_is_a_conflict_not_a_second_feed() {
    let server = feed_server(rss(&[("a", "First")])).await;
    let stack = Stack::start().await.unwrap();
    let url = server.uri() + "/feed.xml";

    stack.post_json("/api/feeds", json!({ "url": url })).await;
    let again = stack.post("/api/feeds", json!({ "url": url })).await;
    assert_eq!(again.status(), 409);

    assert_eq!(
        stack.get_json("/api/feeds").await["feeds"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_url_with_no_feed_is_a_bad_request_with_a_reason() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "<html><head><title>no feeds here</title></head></html>",
            "text/html",
        ))
        .mount(&server)
        .await;

    let stack = Stack::start().await.unwrap();
    let res = stack
        .post("/api/feeds", json!({ "url": server.uri() }))
        .await;
    assert_eq!(res.status(), 400);

    let body: serde_json::Value = res.json().await.unwrap();
    // The SPA shows this string verbatim, so it has to say something useful.
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("could not find a feed"),
        "unhelpful error: {body}"
    );
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn reading_and_starring_moves_items_between_views() {
    let server = feed_server(rss(&[("a", "First"), ("b", "Second")])).await;
    let stack = Stack::start().await.unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;

    let items = stack.get_json("/api/items").await;
    let id = items["items"][0]["id"].as_i64().unwrap();

    let updated = stack
        .patch_json(
            &format!("/api/items/{id}"),
            json!({ "starred": true, "read": true }),
        )
        .await;
    assert_eq!(updated["starred"], true);
    assert_eq!(updated["read"], true);

    assert_eq!(
        stack.get_json("/api/items").await["items"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "the read item should have left the unread view"
    );
    assert_eq!(
        stack.get_json("/api/items?view=starred").await["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let marked = stack.post_json("/api/items/mark-read", json!({})).await;
    assert_eq!(marked["marked"], 1);
    assert!(stack.get_json("/api/items").await["items"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn mark_read_clears_the_narrowing_it_was_given_and_nothing_else() {
    // The bug this pins: mark-read took only `feed_id`, so pressing it while
    // the list was filtered to one topic or one search cleared every unread
    // item in the archive — a destructive action on a selection it could not
    // see, with nothing to undo it.
    let one = feed_server(rss(&[("a", "Hedgehog season"), ("b", "Badger season")])).await;
    let two = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(rss(&[("c", "Something else")]), "application/rss+xml"),
        )
        .mount(&two)
        .await;

    let stack = Stack::start().await.unwrap();
    let first = stack
        .post_json("/api/feeds", json!({ "url": one.uri() + "/feed.xml" }))
        .await;
    stack
        .post_json("/api/feeds", json!({ "url": two.uri() + "/feed.xml" }))
        .await;
    assert_eq!(
        stack.get_json("/api/items").await["items"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    // A search marks what the search found, and leaves the rest alone.
    let marked = stack
        .post_json("/api/items/mark-read", json!({ "q": "hedgehog" }))
        .await;
    assert_eq!(marked["marked"], 1);
    assert_eq!(
        stack.get_json("/api/items").await["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // A feed marks its own, and leaves the other feed alone.
    let marked = stack
        .post_json(
            "/api/items/mark-read",
            json!({ "feed_id": first["id"].as_i64().unwrap() }),
        )
        .await;
    assert_eq!(marked["marked"], 1);
    let left = stack.get_json("/api/items").await;
    assert_eq!(left["items"].as_array().unwrap().len(), 1);
    assert_eq!(left["items"][0]["title"], "Something else");

    // A search that matches nothing marks nothing — emphatically not
    // everything, which is what dropping the clause would do.
    let marked = stack
        .post_json("/api/items/mark-read", json!({ "q": "zzzznothing" }))
        .await;
    assert_eq!(marked["marked"], 0);
    assert_eq!(
        stack.get_json("/api/items").await["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // And unnarrowed still means everything.
    let marked = stack.post_json("/api/items/mark-read", json!({})).await;
    assert_eq!(marked["marked"], 1);
    assert!(stack.get_json("/api/items").await["items"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn search_reaches_read_items_the_active_view_would_hide() {
    let server = feed_server(rss(&[("a", "Hedgehog season"), ("b", "Something else")])).await;
    let stack = Stack::start().await.unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;

    let items = stack.get_json("/api/items").await;
    let id = items["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["title"] == "Hedgehog season")
        .and_then(|i| i["id"].as_i64())
        .unwrap();
    stack
        .patch_json(&format!("/api/items/{id}"), json!({ "read": true }))
        .await;

    // Read, so the default view no longer holds it — and a search that honoured
    // the view would find nothing, which is the whole point of searching.
    let found = stack.get_json("/api/items?q=hedgehog").await;
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    assert_eq!(found["items"][0]["id"], id);

    // The body is indexed too, and a partial word matches as you type it.
    assert_eq!(
        stack.get_json("/api/items?q=body+of+hedge").await["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Punctuation is a separator, not FTS5 syntax — neither of these may 500.
    for route in ["/api/items?q=%22unclosed", "/api/items?q=-NEAR+OR"] {
        let res = stack.get(route).await;
        assert!(res.status().is_success(), "GET {route} → {}", res.status());
    }
    // A query the tokenizer empties means "nothing matched", not "no filter".
    assert!(stack.get_json("/api/items?q=%2A%2A%2A").await["items"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn an_opml_file_round_trips_through_subscribe_and_export() {
    let server = feed_server(rss(&[("a", "First")])).await;
    let stack = Stack::start().await.unwrap();
    let url = server.uri() + "/feed.xml";

    let opml = format!(
        "<opml version=\"2.0\"><body><outline text=\"Tech\">\
         <outline type=\"rss\" title=\"Imported\" xmlUrl=\"{url}\"/>\
         </outline></body></opml>"
    );
    let report: serde_json::Value = stack
        .post_text("/api/opml/import", &opml)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(report["added"], 1);

    // Importing takes the URL as given rather than running discovery, so the
    // feed exists before it has ever been fetched and gets its title on the
    // first poll.
    let feeds = stack.get_json("/api/feeds").await;
    assert_eq!(feeds["feeds"].as_array().unwrap().len(), 1);
    let id = feeds["feeds"][0]["id"].as_i64().unwrap();
    stack
        .post_json(&format!("/api/feeds/{id}/refresh"), json!({}))
        .await;
    assert_eq!(
        stack.get_json("/api/items").await["items"][0]["title"],
        "First"
    );

    // The same file again subscribes to nothing — re-importing is how people
    // check a migration worked.
    let again: serde_json::Value = stack
        .post_text("/api/opml/import", &opml)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!((&again["added"], &again["skipped"]), (&json!(0), &json!(1)));

    let exported = stack.get("/api/opml/export").await;
    assert_eq!(
        exported
            .headers()
            .get("content-disposition")
            .and_then(|v| v.to_str().ok()),
        Some("attachment; filename=\"rosso.opml\"")
    );
    assert!(exported.text().await.unwrap().contains(&url));
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_file_with_no_subscriptions_is_a_bad_request() {
    let stack = Stack::start().await.unwrap();
    let res = stack
        .post_text("/api/opml/import", "<opml><body></body></opml>")
        .await;
    assert_eq!(res.status(), 400);
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn the_api_is_closed_when_the_dev_bypass_is_off() {
    // DEV_AUTH is applied before `extra`, so this overrides the harness default
    // and exercises the same gate production runs behind.
    let stack = Stack::start_with_env(&[("DEV_AUTH", "0")]).await.unwrap();

    assert_eq!(stack.get("/api/feeds").await.status(), 401);
    assert_eq!(
        stack
            .post("/api/feeds", json!({ "url": "x" }))
            .await
            .status(),
        401
    );
    // Liveness stays open so gatus can probe a gated host.
    assert!(stack.get("/status").await.status().is_success());
}
