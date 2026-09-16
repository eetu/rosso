//! Per-feed LLM opt-out, retention pruning, and site icons.

use rosso_integration::Stack;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const BODY: &str = "PyO3 lets a Python package ship a compiled Rust extension module, so the \
     hot loop runs as native code while the public interface stays ordinary Python. The build \
     produces a wheel per platform, which is why a project that adopts it grows a matrix of \
     release jobs. Every value crossing the boundary is converted, and conversion is where the \
     time goes when the speedup fails to appear.";

fn feed(base: &str, guid: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel>\
         <title>Test blog</title><link>{base}/</link>\
         <item><title>Item {guid}</title><link>{base}/{guid}</link><guid>{guid}</guid>\
         <content:encoded xmlns:content=\"http://purl.org/rss/1.0/modules/content/\">\
         <![CDATA[<p>{BODY} {BODY}</p>]]></content:encoded>\
         </item></channel></rss>"
    )
}

async fn chatty_ollama() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "models": [{ "name": "m" }] })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": {
                "role": "assistant",
                "content": "{\"summary\":\"a summary\",\"score\":80,\"reason\":\"r\",\"tags\":[]}"
            }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/embed"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    server
}

async fn wait_for(
    stack: &Stack,
    route: &str,
    mut ready: impl FnMut(&serde_json::Value) -> bool,
) -> serde_json::Value {
    for _ in 0..80 {
        let body = stack.get_json(route).await;
        if ready(&body) {
            return body;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    panic!("{route} never reached the expected state");
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_feed_opted_out_of_the_model_is_never_summarized() {
    let feeds = MockServer::start().await;
    for name in ["on", "off"] {
        Mock::given(method("GET"))
            .and(path(format!("/{name}.xml")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(feed(&feeds.uri(), name), "application/rss+xml"),
            )
            .mount(&feeds)
            .await;
    }

    let ollama = chatty_ollama().await;
    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", &ollama.uri())])
        .await
        .unwrap();

    let quiet = stack
        .post_json(
            "/api/feeds",
            json!({ "url": format!("{}/off.xml", feeds.uri()) }),
        )
        .await;
    assert_eq!(quiet["llm_enabled"], true, "the default is on");

    // Opted out before the worker's first pass.
    let patched = stack
        .patch_json(
            &format!("/api/feeds/{}", quiet["id"].as_i64().unwrap()),
            json!({ "llm_enabled": false }),
        )
        .await;
    assert_eq!(patched["llm_enabled"], false);

    stack
        .post_json(
            "/api/feeds",
            json!({ "url": format!("{}/on.xml", feeds.uri()) }),
        )
        .await;

    // The opted-in feed gets its summary; the opted-out one is still plain once
    // it has. Waiting on the first proves the worker ran at all, which is what
    // makes the second assertion mean something.
    wait_for(&stack, "/api/items?view=all", |b| {
        b["items"]
            .as_array()
            .is_some_and(|a| a.iter().any(|i| i["summary"].is_string()))
    })
    .await;

    let items = stack.get_json("/api/items?view=all").await;
    let by_title = |t: &str| {
        items["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["title"] == t)
            .cloned()
            .unwrap()
    };
    assert!(by_title("Item on")["summary"].is_string());
    assert!(
        by_title("Item off")["summary"].is_null(),
        "an opted-out feed was summarized anyway"
    );

    // Turning it back on needs no invalidation pass: it was never marked
    // enriched, so it becomes a candidate on its own.
    stack
        .patch_json(
            &format!("/api/feeds/{}", quiet["id"].as_i64().unwrap()),
            json!({ "llm_enabled": true }),
        )
        .await;
    wait_for(&stack, "/api/items?view=all", |b| {
        b["items"]
            .as_array()
            .is_some_and(|a| a.iter().all(|i| i["summary"].is_string()))
    })
    .await;
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_lazily_loaded_article_image_reaches_the_reader() {
    // ammonia keeps only `align alt height src width` on an img, so a data
    // attribute holding the real picture is stripped and a one-pixel
    // placeholder is all that survives.
    let server = MockServer::start().await;
    let body = format!(
        "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel>\
         <title>Pictures</title><link>{base}/</link>\
         <item><title>With a picture</title><link>{base}/p</link><guid>p</guid>\
         <content:encoded xmlns:content=\"http://purl.org/rss/1.0/modules/content/\">\
         <![CDATA[<p>{BODY}</p><figure><img src=\"data:image/gif;base64,R0lGODlhAQABAAA\" \
         data-src=\"{base}/real.jpg\"><figcaption>a caption</figcaption></figure>]]>\
         </content:encoded></item></channel></rss>",
        base = server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/rss+xml"))
        .mount(&server)
        .await;

    let stack = Stack::start_with_env(&[("ROSSO_EXTRACT", "0")])
        .await
        .unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;

    let id = stack.get_json("/api/items").await["items"][0]["id"]
        .as_i64()
        .unwrap();
    let html = stack.get_json(&format!("/api/items/{id}")).await["content_html"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(html.contains("/real.jpg"), "the picture was lost: {html}");
    assert!(!html.contains("data:image/gif"), "{html}");
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_site_icon_is_stored_rather_than_linked_to() {
    // A link to the publisher's favicon would announce the reader to every site
    // in the list on every page load, so the bytes are fetched once and kept.
    let server = MockServer::start().await;
    let page = format!(
        "<html><head><link rel=\"icon\" href=\"/i.png\">\
         <link rel=\"alternate\" type=\"application/rss+xml\" href=\"{}/feed.xml\">\
         </head></html>",
        server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(page, "text/html"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(feed(&server.uri(), "a"), "application/rss+xml"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/i.png"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(vec![0x89, b'P', b'N', b'G'], "image/png"),
        )
        .mount(&server)
        .await;

    // The harness's servers are on loopback, which the fetch refuses without
    // this — the same flag the extraction tests need, and for the same reason.
    let stack = Stack::start_with_env(&[("ROSSO_EXTRACT_ALLOW_PRIVATE", "1")])
        .await
        .unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;

    let feeds = wait_for(&stack, "/api/feeds", |b| b["feeds"][0]["icon"].is_string()).await;
    let icon = feeds["feeds"][0]["icon"].as_str().unwrap();
    assert!(
        icon.starts_with("data:image/png;base64,"),
        "not an inlined icon: {icon}"
    );
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn an_html_error_page_is_never_stored_as_an_icon() {
    // `/favicon.ico` answering 200 with a 404 page is the usual way this
    // "succeeds"; embedding it would put a broken image on every row.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(feed(&server.uri(), "a"), "application/rss+xml"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw("<html>not found</html>", "text/html"),
        )
        .mount(&server)
        .await;

    let stack = Stack::start_with_env(&[("ROSSO_EXTRACT_ALLOW_PRIVATE", "1")])
        .await
        .unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;

    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    assert!(
        stack.get_json("/api/feeds").await["feeds"][0]["icon"].is_null(),
        "an html page was stored as an icon"
    );
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn pruning_keeps_what_the_reader_still_wants() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(feed(&server.uri(), "a"), "application/rss+xml"),
        )
        .mount(&server)
        .await;

    let stack = Stack::start().await.unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": server.uri() + "/feed.xml" }))
        .await;

    // Off by default: a reader's archive is not something to start deleting on
    // its own.
    let settings = stack.get_json("/api/settings").await;
    assert_eq!(settings["retention_days"], 0);

    // A retention of a day would delete this morning's reading before the
    // evening, so anything above zero is floored at a week.
    let saved = stack
        .put_json("/api/settings", json!({ "retention_days": 1 }))
        .await;
    assert_eq!(saved["retention_days"], 7);

    // Today's item is inside any window, read or not — the worker cannot touch
    // it, which is what this asserts about the cutoff rather than the schedule.
    let id = stack.get_json("/api/items").await["items"][0]["id"]
        .as_i64()
        .unwrap();
    stack
        .patch_json(&format!("/api/items/{id}"), json!({ "read": true }))
        .await;
    assert_eq!(
        stack.get_json("/api/items?view=all").await["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
