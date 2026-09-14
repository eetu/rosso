//! Full-text extraction: a feed that ships a teaser, and the article page behind
//! it. Both are wiremocked — nothing here touches the network.

use rosso_integration::Stack;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ARTICLE_BODY: &str = "PyO3 lets a Python package ship a compiled Rust extension \
     module, so the hot loop runs as native code while the public interface stays \
     ordinary Python. The build produces a wheel per platform, which is why a project \
     that adopts it usually grows a matrix of release jobs. The interesting part is the \
     boundary: every value crossing it is converted, and conversion is where the time \
     goes when the speedup fails to show up in benchmarks.";

fn article_page() -> String {
    format!(
        "<html><head><title>Rust inside Python</title></head><body>\
         <nav><a href=\"/\">home</a><a href=\"/about\">about</a></nav>\
         <article><h1>Rust inside Python</h1><p>{ARTICLE_BODY}</p>\
         <p>{ARTICLE_BODY}</p></article>\
         <footer>cookie notice</footer></body></html>"
    )
}

/// A feed whose item links at `{base}/article` and carries only a short teaser.
fn teaser_feed(base: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel>\
         <title>Teaser blog</title><link>{base}/</link>\
         <item><title>Rust inside Python</title><link>{base}/article</link>\
         <guid>teaser-1</guid>\
         <pubDate>Tue, 01 Sep 2026 10:00:00 GMT</pubDate>\
         <description>A short teaser that stops well before the interesting part.</description>\
         </item></channel></rss>"
    )
}

async fn server_with(article: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    let feed = teaser_feed(&server.uri());
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(feed, "application/rss+xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/article"))
        .respond_with(article)
        .mount(&server)
        .await;
    server
}

async fn subscribe_and_open(stack: &Stack, server: &MockServer) -> serde_json::Value {
    stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;
    let items = stack.get_json("/api/items").await;
    let id = items["items"][0]["id"].as_i64().unwrap();
    stack.get_json(&format!("/api/items/{id}")).await
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn opening_a_teaser_item_extracts_the_article_behind_it() {
    let server = server_with(
        ResponseTemplate::new(200).set_body_raw(article_page(), "text/html; charset=utf-8"),
    )
    .await;
    let stack = Stack::start().await.unwrap();

    let detail = subscribe_and_open(&stack, &server).await;
    let html = detail["content_html"].as_str().unwrap_or_default();
    assert!(
        html.contains("compiled Rust extension"),
        "the article body did not replace the teaser: {html}"
    );
    // Readability's job: the chrome around the article does not come with it.
    assert!(!html.contains("cookie notice"), "kept the footer: {html}");
    assert!(!html.contains("href=\"/about\""), "kept the nav: {html}");
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_page_with_no_article_leaves_the_feed_body_alone() {
    // Readability finding only navigation must not overwrite what the feed gave
    // with a cookie banner.
    let server = server_with(ResponseTemplate::new(200).set_body_raw(
        "<html><body><nav><a href=\"/\">home</a></nav><p>Accept cookies?</p></body></html>",
        "text/html",
    ))
    .await;
    let stack = Stack::start().await.unwrap();

    let detail = subscribe_and_open(&stack, &server).await;
    assert!(detail["content_html"]
        .as_str()
        .unwrap_or_default()
        .contains("A short teaser"));
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_failing_article_page_does_not_fail_the_request() {
    let server = server_with(ResponseTemplate::new(500)).await;
    let stack = Stack::start().await.unwrap();

    let detail = subscribe_and_open(&stack, &server).await;
    // The item still renders with whatever the feed gave.
    assert!(detail["content_html"]
        .as_str()
        .unwrap_or_default()
        .contains("A short teaser"));
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_non_html_target_is_refused_before_it_reaches_the_parser() {
    let server = server_with(
        ResponseTemplate::new(200).set_body_raw(vec![0xff, 0xd8, 0xff, 0xe0], "image/jpeg"),
    )
    .await;
    let stack = Stack::start().await.unwrap();

    let detail = subscribe_and_open(&stack, &server).await;
    assert!(detail["content_html"]
        .as_str()
        .unwrap_or_default()
        .contains("A short teaser"));
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn extraction_can_be_turned_off_entirely() {
    let server = server_with(
        ResponseTemplate::new(200).set_body_raw(article_page(), "text/html; charset=utf-8"),
    )
    .await;
    let stack = Stack::start_with_env(&[("ROSSO_EXTRACT", "0")])
        .await
        .unwrap();

    let detail = subscribe_and_open(&stack, &server).await;
    let html = detail["content_html"].as_str().unwrap_or_default();
    assert!(
        html.contains("A short teaser"),
        "extracted despite the switch"
    );
    assert!(!html.contains("compiled Rust extension"));
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_page_that_never_parses_is_given_up_on() {
    // Without a cap, an item whose page will never yield an article costs a
    // request every time it is opened and on every pass of the worker, forever.
    let server = MockServer::start().await;
    let feed = teaser_feed(&server.uri());
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(feed, "application/rss+xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/article"))
        .respond_with(ResponseTemplate::new(404))
        .expect(3) // MAX_ATTEMPTS — asserted when the server drops
        .mount(&server)
        .await;

    let stack = Stack::start().await.unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;
    let id = stack.get_json("/api/items").await["items"][0]["id"]
        .as_i64()
        .unwrap();

    // Five opens, three attempts.
    for _ in 0..5 {
        let detail = stack.get_json(&format!("/api/items/{id}")).await;
        assert!(detail["content_html"]
            .as_str()
            .unwrap_or_default()
            .contains("A short teaser"));
    }
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn an_item_pointing_at_the_lan_is_refused() {
    // Item URLs come from the publisher. Left unguarded, a feed could aim an
    // item at a loopback or LAN service and have rosso fetch it and render the
    // response. The harness normally sets ROSSO_EXTRACT_ALLOW_PRIVATE so that
    // wiremock works at all; this is the one test that runs without it.
    let server = MockServer::start().await;
    let feed = teaser_feed(&server.uri());
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(feed, "application/rss+xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/article"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(article_page(), "text/html"))
        .expect(0) // never reached: the address is refused before any request
        .mount(&server)
        .await;

    let stack = Stack::start_with_env(&[("ROSSO_EXTRACT_ALLOW_PRIVATE", "0")])
        .await
        .unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;
    let id = stack.get_json("/api/items").await["items"][0]["id"]
        .as_i64()
        .unwrap();

    let detail = stack.get_json(&format!("/api/items/{id}")).await;
    assert!(
        detail["content_html"]
            .as_str()
            .unwrap_or_default()
            .contains("A short teaser"),
        "extracted from a loopback address"
    );
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_feed_with_full_content_is_never_fetched_again() {
    // Only `truncated` items are candidates, so a full-text feed must cost no
    // outbound requests at all — the article mock asserts that on drop.
    let server = MockServer::start().await;
    let long = ARTICLE_BODY.repeat(2);
    let feed = format!(
        "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel>\
         <title>Full text</title><link>{base}/</link>\
         <item><title>Complete post</title><link>{base}/article</link><guid>full-1</guid>\
         <content:encoded xmlns:content=\"http://purl.org/rss/1.0/modules/content/\">\
         <![CDATA[<p>{long}</p>]]></content:encoded>\
         </item></channel></rss>",
        base = server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(feed, "application/rss+xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/article"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let stack = Stack::start().await.unwrap();
    let detail = subscribe_and_open(&stack, &server).await;
    assert!(detail["content_html"]
        .as_str()
        .unwrap_or_default()
        .contains("PyO3 lets a Python package"));
}
