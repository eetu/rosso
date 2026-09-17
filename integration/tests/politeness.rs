//! What a publisher asks for, and whether rosso does it.
//!
//! `Retry-After`, a `ttl` longer than our own ceiling, and a run of refusals
//! retiring a feed rather than asking forever.

use rosso_integration::Stack;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn feed_with(base: &str, extra: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel>\
         <title>Test feed</title><link>{base}/</link>{extra}\
         <item><title>First</title><link>{base}/a</link><guid>a</guid>\
         <description>body of first</description></item></channel></rss>"
    )
}

async fn inspection(stack: &Stack, url: &str) -> serde_json::Value {
    let body = stack.get_json("/api/feeds/inspect").await;
    body["feeds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["url"] == url)
        .cloned()
        .unwrap_or_else(|| panic!("{url} missing from the inspection"))
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_server_that_names_a_wait_gets_exactly_that_wait() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(feed_with(&server.uri(), ""), "application/rss+xml"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Then the host asks for a wait far longer than any backoff we would pick.
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "7200"))
        .mount(&server)
        .await;

    let stack = Stack::start().await.unwrap();
    let url = server.uri() + "/feed.xml";
    let feed = stack.post_json("/api/feeds", json!({ "url": url })).await;
    stack
        .post_json(
            &format!("/api/feeds/{}/refresh", feed["id"].as_i64().unwrap()),
            json!({}),
        )
        .await;

    let seen = inspection(&stack, &url).await;
    assert_eq!(seen["retry_after_s"], 7200);
    // A stated wait is not a failure: it must not also push the feed down the
    // backoff curve or toward being retired.
    assert_eq!(seen["failures"], 0);
    assert_eq!(seen["refusals"], 0);
    assert_eq!(seen["last_error"], serde_json::Value::Null);
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_ttl_longer_than_our_ceiling_is_still_honoured() {
    // Our six-hour ceiling is a guess about a feed that told us nothing. A ttl
    // is the publisher telling us, and clamping it meant a feed asking for
    // twelve hours was polled twice as often as it asked.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            feed_with(&server.uri(), "<ttl>720</ttl>"),
            "application/rss+xml",
        ))
        .mount(&server)
        .await;

    let stack = Stack::start().await.unwrap();
    let url = server.uri() + "/feed.xml";
    stack.post_json("/api/feeds", json!({ "url": url })).await;

    let seen = inspection(&stack, &url).await;
    assert_eq!(seen["ttl_minutes"], 720);
    assert_eq!(seen["interval_s"], 720 * 60, "the ttl was undercut");
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_feed_that_keeps_refusing_retires_itself_and_can_be_revived() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(feed_with(&server.uri(), ""), "application/rss+xml"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let stack = Stack::start().await.unwrap();
    let url = server.uri() + "/feed.xml";
    let feed = stack.post_json("/api/feeds", json!({ "url": url })).await;
    let id = feed["id"].as_i64().unwrap();
    let refresh = format!("/api/feeds/{id}/refresh");

    // Under the limit it is still polled: a 403 can be a misconfigured CDN for
    // an afternoon.
    stack.post_json(&refresh, json!({})).await;
    let seen = inspection(&stack, &url).await;
    assert_eq!(seen["refusals"], 1);
    assert_eq!(seen["disabled"], false);

    for _ in 0..3 {
        stack.post_json(&refresh, json!({})).await;
    }
    let seen = inspection(&stack, &url).await;
    assert_eq!(seen["refusals"], 4);
    assert_eq!(seen["disabled"], true, "kept asking a host that said no");
    assert!(seen["last_error"]
        .as_str()
        .unwrap()
        .contains("refuses this client"));

    // Retired, not deleted: the reason stays readable and the decision stays
    // the reader's.
    stack
        .patch_json(&format!("/api/feeds/{id}"), json!({ "disabled": false }))
        .await;
    let seen = inspection(&stack, &url).await;
    assert_eq!(seen["disabled"], false);
    assert_eq!(
        seen["refusals"], 0,
        "re-enabling must clear the count, or it retires again on the next poll"
    );
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn the_inspection_reports_what_a_quiet_feed_offers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(feed_with(&server.uri(), ""), "application/rss+xml")
                .insert_header("etag", "\"v1\""),
        )
        .mount(&server)
        .await;

    let stack = Stack::start().await.unwrap();
    let url = server.uri() + "/feed.xml";
    stack.post_json("/api/feeds", json!({ "url": url })).await;

    let seen = inspection(&stack, &url).await;
    assert_eq!(
        seen["conditional"], true,
        "an etag was offered and not stored"
    );
    assert_eq!(seen["ttl_minutes"], serde_json::Value::Null);
    assert_eq!(seen["retry_after_s"], serde_json::Value::Null);
    assert!(seen["interval_s"].as_i64().unwrap() > 0);
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_feeds_declared_icon_url_never_reaches_the_browser_as_a_link() {
    // `record_success` used to write the declared icon URL into the column the
    // sidebar renders, which would put an <img> pointing at the publisher on
    // every page load — the request the favicon worker exists to avoid.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            feed_with(
                &server.uri(),
                "<image><url>https://elsewhere.test/i.png</url>\
                 <title>t</title><link>https://elsewhere.test/</link></image>",
            ),
            "application/rss+xml",
        ))
        .mount(&server)
        .await;

    let stack = Stack::start().await.unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;

    let icon = stack.get_json("/api/feeds").await["feeds"][0]["icon"].clone();
    assert!(
        icon.is_null() || icon.as_str().is_some_and(|s| s.starts_with("data:")),
        "a publisher URL reached the sidebar: {icon}"
    );
}
