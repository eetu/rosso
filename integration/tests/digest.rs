//! The daily digest, against a wiremock Ollama.
//!
//! Generated on demand rather than waiting for the hour — the scheduled path is
//! the same `generate` call behind a clock check.

use rosso_integration::Stack;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const BODY: &str = "PyO3 lets a Python package ship a compiled Rust extension module, so the \
     hot loop runs as native code while the public interface stays ordinary Python. The build \
     produces a wheel per platform, which is why a project that adopts it grows a matrix of \
     release jobs. Every value crossing the boundary is converted, and conversion is where the \
     time goes when the speedup fails to appear.";

fn feed(base: &str, guids: &[&str]) -> String {
    let entries: String = guids
        .iter()
        .map(|guid| {
            format!(
                "<item><title>Item {guid}</title><link>{base}/{guid}</link><guid>{guid}</guid>\
                 <content:encoded xmlns:content=\"http://purl.org/rss/1.0/modules/content/\">\
                 <![CDATA[<p>{BODY} {BODY}</p>]]></content:encoded></item>"
            )
        })
        .collect();
    format!(
        "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel>\
         <title>Test blog</title><link>{base}/</link>{entries}</channel></rss>"
    )
}

/// Answers the enrichment calls with a summary, and the digest call with a
/// digest — told apart by whether the prompt asked for one.
///
/// The digest names one real id taken from the prompt and one that does not
/// exist, so the test can show the invented one being dropped.
struct Ollama;

impl Respond for Ollama {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).expect("chat request");
        let prompt = body["messages"][0]["content"].as_str().unwrap_or_default();
        let content = if prompt.contains("daily digest") {
            // The ids the digest may refer to are in the user message as `[n]`.
            let user = body["messages"][1]["content"].as_str().unwrap_or_default();
            let first = user
                .split_once('[')
                .and_then(|(_, rest)| rest.split_once(']'))
                .map(|(id, _)| id.to_string())
                .expect("no candidate ids in the digest prompt");
            json!({
                "intro": "a quiet day with one thread worth reading.",
                "threads": [
                    { "title": "Rust inside Python", "note": "PyO3 again.",
                      "item_ids": [first.parse::<i64>().unwrap(), 99999] },
                    { "title": "Entirely invented", "note": "nothing real.",
                      "item_ids": [99998] }
                ]
            })
        } else {
            json!({ "summary": "a summary", "score": 70, "reason": "r", "tags": ["rust"] })
        };
        ResponseTemplate::new(200).set_body_json(json!({
            "message": { "role": "assistant", "content": content.to_string() }
        }))
    }
}

async fn stub_ollama() -> MockServer {
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
        .respond_with(Ollama)
        .mount(&server)
        .await;
    // Embedding is irrelevant here, but the worker runs against the same host
    // and an unstubbed endpoint would fill the log with 404s.
    Mock::given(method("POST"))
        .and(path("/api/embed"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    server
}

/// Today in UTC — the day the harness's freshly-fetched items land on.
fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_day_becomes_a_digest_that_links_back_to_its_items() {
    let feeds = MockServer::start().await;
    let body = feed(&feeds.uri(), &["a", "b", "c", "d"]);
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/rss+xml"))
        .mount(&feeds)
        .await;

    let ollama = stub_ollama().await;
    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", &ollama.uri())])
        .await
        .unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": feeds.uri() + "/feed.xml" }))
        .await;

    let day = today();
    let made = stack.post_json("/api/digests", json!({ "day": day })).await;
    assert_eq!(made["item_count"], 4);

    let digest = stack.get_json(&format!("/api/digests/{day}")).await;
    assert_eq!(digest["day"], day);
    assert_eq!(
        digest["intro"],
        "a quiet day with one thread worth reading."
    );

    // The thread whose ids were all invented is gone, and the surviving one kept
    // only the id that resolves. A link to an item that does not exist is worse
    // than a thread with one fewer.
    let threads = digest["threads"].as_array().unwrap();
    assert_eq!(
        threads.len(),
        1,
        "an invented thread survived: {threads:#?}"
    );
    assert_eq!(threads[0]["item_ids"].as_array().unwrap().len(), 1);

    // And the items are resolved once, so the reader renders rows rather than
    // asking for each id.
    let items = digest["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], threads[0]["item_ids"][0]);
    assert!(items[0]["title"].as_str().unwrap().starts_with("Item "));

    assert_eq!(stack.get_json("/api/digests").await["days"][0], day);
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_day_with_almost_nothing_in_it_is_not_worth_a_digest() {
    let feeds = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(feed(&feeds.uri(), &["a"]), "application/rss+xml"),
        )
        .mount(&feeds)
        .await;

    let ollama = stub_ollama().await;
    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", &ollama.uri())])
        .await
        .unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": feeds.uri() + "/feed.xml" }))
        .await;

    let res = stack.post("/api/digests", json!({ "day": today() })).await;
    assert_eq!(res.status(), 400);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("too little"),
        "unhelpful error: {body}"
    );
    // Nothing was written, so tomorrow's tick is free to try again.
    assert!(stack.get_json("/api/digests").await["days"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_reader_with_no_model_host_still_answers_about_digests() {
    // The whole LLM layer is additive. Asking for a digest that cannot be
    // written is a 400 with a reason, never a 500, and the section is empty
    // rather than broken.
    let stack = Stack::start().await.unwrap();
    assert!(stack.get_json("/api/digests").await["days"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(stack.get("/api/digests/2026-09-15").await.status(), 404);
    assert_eq!(
        stack.post("/api/digests", json!({})).await.status(),
        400,
        "no model host must not be a server error"
    );
    // A malformed day says so rather than 404ing as a miss.
    let res = stack
        .post("/api/digests", json!({ "day": "yesterday" }))
        .await;
    assert_eq!(res.status(), 400);
}
