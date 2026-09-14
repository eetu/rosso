//! Summaries and scores, against a wiremock Ollama.
//!
//! The model host is stubbed, so these assert the wiring and the degraded paths
//! rather than the quality of any particular model's judgement.

use rosso_integration::Stack;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const BODY: &str = "PyO3 lets a Python package ship a compiled Rust extension module, so the \
     hot loop runs as native code while the public interface stays ordinary Python. The build \
     produces a wheel per platform, which is why a project that adopts it grows a matrix of \
     release jobs. Every value crossing the boundary is converted, and conversion is where the \
     time goes when the speedup fails to appear.";

/// A full-text feed. The body is deliberately well over the teaser threshold:
/// a shorter one would be flagged `truncated`, and enrichment waits for
/// extraction to finish on those — so the test would be measuring the extraction
/// worker's retry schedule rather than anything about summaries.
fn feed_with_content(base: &str) -> String {
    let body = format!("{BODY} {BODY}");
    format!(
        "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel>\
         <title>Test blog</title><link>{base}/</link>\
         <item><title>Rust inside Python</title><link>{base}/post</link><guid>g1</guid>\
         <content:encoded xmlns:content=\"http://purl.org/rss/1.0/modules/content/\">\
         <![CDATA[<p>{body}</p>]]></content:encoded>\
         </item></channel></rss>"
    )
}

/// An Ollama that answers `/api/chat` with a fixed payload and lists one model.
async fn fake_ollama(chat: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models": [{ "name": "gemma4:e4b-mlx" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(chat)
        .mount(&server)
        .await;
    server
}

fn chat_reply(content: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "message": { "role": "assistant", "content": content.to_string() }
    }))
}

async fn feed_server() -> MockServer {
    let server = MockServer::start().await;
    let body = feed_with_content(&server.uri());
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/rss+xml"))
        .mount(&server)
        .await;
    server
}

/// Poll until the predicate holds, or give up. The enrichment worker runs on its
/// own clock, so the tests wait for it rather than reaching into it.
async fn wait_for(
    stack: &Stack,
    route: &str,
    mut ready: impl FnMut(&serde_json::Value) -> bool,
) -> serde_json::Value {
    for _ in 0..120 {
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
async fn an_item_gets_a_summary_and_a_score_from_the_profile() {
    let ollama = fake_ollama(chat_reply(json!({
        "summary": "PyO3 compiles a Rust extension into a Python wheel.",
        "score": 88,
        "reason": "rust and python interop",
        "tags": ["rust", "python"]
    })))
    .await;
    let feeds = feed_server().await;

    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", &ollama.uri())])
        .await
        .unwrap();
    stack
        .put_json(
            "/api/settings",
            json!({ "interest_profile": "rust, python" }),
        )
        .await;
    stack
        .post_json("/api/feeds", json!({ "url": feeds.uri() + "/feed.xml" }))
        .await;

    let body = wait_for(&stack, "/api/items", |b| {
        b["items"][0]["summary"].is_string()
    })
    .await;
    assert_eq!(
        body["items"][0]["summary"],
        "PyO3 compiles a Rust extension into a Python wheel."
    );
    assert_eq!(body["items"][0]["score"], 88);
    assert_eq!(body["items"][0]["score_reason"], "rust and python interop");
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn with_no_profile_items_are_summarized_but_never_scored() {
    // A number produced with nothing to judge against would look exactly like a
    // real one in the UI, so there must not be one.
    let ollama = fake_ollama(chat_reply(json!({
        "summary": "A summary.",
        "tags": ["rust"]
    })))
    .await;
    let feeds = feed_server().await;

    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", &ollama.uri())])
        .await
        .unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": feeds.uri() + "/feed.xml" }))
        .await;

    let body = wait_for(&stack, "/api/items", |b| {
        b["items"][0]["summary"].is_string()
    })
    .await;
    assert_eq!(body["items"][0]["summary"], "A summary.");
    assert_eq!(body["items"][0]["score"], serde_json::Value::Null);
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn the_interesting_view_holds_only_unread_items_above_the_cutoff() {
    let ollama = fake_ollama(chat_reply(json!({
        "summary": "s", "score": 90, "reason": "r", "tags": []
    })))
    .await;
    let feeds = feed_server().await;

    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", &ollama.uri())])
        .await
        .unwrap();
    stack
        .put_json(
            "/api/settings",
            json!({ "interest_profile": "rust", "score_threshold": 80 }),
        )
        .await;
    stack
        .post_json("/api/feeds", json!({ "url": feeds.uri() + "/feed.xml" }))
        .await;

    wait_for(&stack, "/api/items", |b| b["items"][0]["score"].is_i64()).await;
    let interesting = stack.get_json("/api/items?view=interesting").await;
    assert_eq!(interesting["items"].as_array().unwrap().len(), 1);
    // Score-ordered views cannot be paged by the time cursor, so they say so.
    assert_eq!(interesting["next_cursor"], serde_json::Value::Null);

    // Raising the bar above the score empties the view.
    stack
        .put_json("/api/settings", json!({ "score_threshold": 95 }))
        .await;
    let raised = stack.get_json("/api/items?view=interesting").await;
    assert!(raised["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn editing_the_profile_rescores_what_was_already_summarized() {
    // The rescore path reuses the stored summary, which is what makes editing
    // the profile cheap rather than a full re-read of the archive.
    let ollama = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [] })))
        .mount(&ollama)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(chat_reply(
            json!({ "summary": "s", "score": 20, "reason": "off topic", "tags": [] }),
        ))
        .up_to_n_times(1)
        .mount(&ollama)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(chat_reply(json!({ "score": 95, "reason": "now on topic" })))
        .mount(&ollama)
        .await;

    let feeds = feed_server().await;
    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", &ollama.uri())])
        .await
        .unwrap();
    stack
        .put_json("/api/settings", json!({ "interest_profile": "knitting" }))
        .await;
    stack
        .post_json("/api/feeds", json!({ "url": feeds.uri() + "/feed.xml" }))
        .await;

    wait_for(&stack, "/api/items", |b| b["items"][0]["score"] == 20).await;

    stack
        .put_json(
            "/api/settings",
            json!({ "interest_profile": "rust, python" }),
        )
        .await;
    let rescored = wait_for(&stack, "/api/items", |b| b["items"][0]["score"] == 95).await;
    // The summary survived: only the score was redone.
    assert_eq!(rescored["items"][0]["summary"], "s");
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn an_unreachable_model_host_leaves_the_reader_whole() {
    let feeds = feed_server().await;
    let dead = format!(
        "http://127.0.0.1:{}",
        rosso_integration::free_port().unwrap()
    );

    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", &dead)])
        .await
        .unwrap();
    stack
        .put_json("/api/settings", json!({ "interest_profile": "rust" }))
        .await;
    stack
        .post_json("/api/feeds", json!({ "url": feeds.uri() + "/feed.xml" }))
        .await;

    let body = stack.get_json("/api/items").await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["summary"], serde_json::Value::Null);
    assert_eq!(body["items"][0]["score"], serde_json::Value::Null);
    // The item is still fully readable — the LLM layer is additive, not load-bearing.
    let id = body["items"][0]["id"].as_i64().unwrap();
    let detail = stack.get_json(&format!("/api/items/{id}")).await;
    assert!(detail["content_html"]
        .as_str()
        .unwrap_or_default()
        .contains("PyO3"));
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_model_answering_with_nonsense_does_not_wedge_the_worker() {
    let ollama = fake_ollama(ResponseTemplate::new(200).set_body_json(json!({
        "message": { "role": "assistant", "content": "I'm afraid I can't do that." }
    })))
    .await;
    let feeds = feed_server().await;

    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", &ollama.uri())])
        .await
        .unwrap();
    stack
        .post_json("/api/feeds", json!({ "url": feeds.uri() + "/feed.xml" }))
        .await;

    // Give the worker time to try and give up; the item stays unsummarized and
    // everything else keeps working.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let body = stack.get_json("/api/items").await;
    assert_eq!(body["items"][0]["summary"], serde_json::Value::Null);
    assert!(stack.get("/api/feeds").await.status().is_success());
}
