//! `/api/stream` — what a tab left open is told while it sits there.

use std::time::Duration;

use rosso_integration::Stack;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn rss(titles: &[&str]) -> String {
    let entries: String = titles
        .iter()
        .map(|t| {
            format!(
                "<item><title>{t}</title><link>https://example.test/{t}</link>\
                 <guid>{t}</guid><description>body of {t}</description></item>"
            )
        })
        .collect();
    format!(
        "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel>\
         <title>Test feed</title><link>https://example.test/</link>{entries}</channel></rss>"
    )
}

/// Read from the open stream until a line mentions `needle`.
///
/// Reads chunks rather than the whole body, which never ends — a stream that is
/// working is exactly one that does not complete.
async fn wait_for(res: &mut reqwest::Response, needle: &str) -> String {
    let deadline = Duration::from_secs(5);
    tokio::time::timeout(deadline, async {
        let mut seen = String::new();
        while let Some(chunk) = res.chunk().await.expect("stream broke") {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains(needle) {
                return seen;
            }
        }
        panic!("stream ended without {needle}; saw: {seen}");
    })
    .await
    .unwrap_or_else(|_| panic!("no {needle} within {deadline:?}"))
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_poll_that_finds_something_tells_an_open_tab() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(rss(&["First"]), "application/rss+xml"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(rss(&["First", "Second"]), "application/rss+xml"),
        )
        .mount(&server)
        .await;

    let stack = Stack::start().await.unwrap();
    let feed = stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;
    let id = feed["id"].as_i64().unwrap();

    // Connecting returns once the handler has subscribed, so nothing emitted
    // after this line can be missed.
    let mut stream = stack.get("/api/stream").await;
    assert!(stream.status().is_success());
    assert_eq!(
        stream
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );

    stack
        .post_json(&format!("/api/feeds/{id}/refresh"), json!({}))
        .await;

    let seen = wait_for(&mut stream, "items-new").await;
    assert!(
        seen.contains(&format!("\"feed_id\":{id}")) && seen.contains("\"count\":1"),
        "the event named neither the feed nor how many arrived: {seen}"
    );
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn the_stream_is_behind_the_same_gate_as_everything_else() {
    let stack = Stack::start_with_env(&[("DEV_AUTH", "0")]).await.unwrap();
    assert_eq!(stack.get("/api/stream").await.status(), 401);
}
