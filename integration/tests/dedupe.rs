//! The same story from two outlets becomes one row.
//!
//! The embedder is stubbed with a bag-of-words model rather than a fixed reply,
//! so the clustering maths is actually exercised: two texts about the same thing
//! come back close together and a third comes back orthogonal, exactly as a real
//! embedding would, without needing the mini to be awake.

use rosso_integration::Stack;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// The stub's whole vocabulary. A text's vector is its counts over these words,
/// which is enough for "same topic" to mean "same direction".
const VOCAB: [&str; 8] = [
    "pyo3",
    "python",
    "wheel",
    "tomato",
    "compost",
    "greenhouse",
    "kernel",
    "scheduler",
];

struct BagOfWords;

impl Respond for BagOfWords {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).expect("embed request");
        let embeddings: Vec<Vec<f32>> = body["input"]
            .as_array()
            .expect("`input` is an array — the singular endpoint is not the one rosso calls")
            .iter()
            .map(|text| {
                let lower = text.as_str().unwrap_or_default().to_lowercase();
                VOCAB
                    .iter()
                    .map(|w| lower.matches(w).count() as f32)
                    .collect()
            })
            .collect();
        ResponseTemplate::new(200).set_body_json(json!({ "embeddings": embeddings }))
    }
}

/// Well past the 400-character full-text threshold. Below it an item is flagged
/// `truncated`, both workers wait for extraction to finish with it, and the test
/// measures the extraction retry schedule instead of anything about clustering.
fn padded(text: &str) -> String {
    text.repeat(20)
}

fn feed(base: &str, title: &str, items: &[(&str, &str, &str)]) -> String {
    let entries: String = items
        .iter()
        .map(|(guid, item_title, body)| {
            format!(
                "<item><title>{item_title}</title><link>{base}/{guid}</link><guid>{guid}</guid>\
                 <content:encoded xmlns:content=\"http://purl.org/rss/1.0/modules/content/\">\
                 <![CDATA[<p>{}</p>]]></content:encoded></item>",
                padded(body)
            )
        })
        .collect();
    format!(
        "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel>\
         <title>{title}</title><link>{base}/</link>{entries}</channel></rss>"
    )
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
        .and(path("/api/embed"))
        .respond_with(BagOfWords)
        .mount(&server)
        .await;
    // Enrichment runs against the same host. It has nothing to do with dedupe,
    // but leaving it unstubbed would fill the log with 404s.
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": { "role": "assistant", "content": "{\"summary\":\"s\",\"tags\":[]}" }
        })))
        .mount(&server)
        .await;
    server
}

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

async fn feeds_server() -> MockServer {
    let server = MockServer::start().await;
    let base = server.uri();
    let one = feed(
        &base,
        "Outlet one",
        &[
            (
                "a",
                "PyO3 ships a wheel",
                "pyo3 builds a python wheel per platform. ",
            ),
            (
                "c",
                "Tomatoes under glass",
                "a greenhouse tomato wants compost and warmth. ",
            ),
        ],
    );
    let two = feed(
        &base,
        "Outlet two",
        &[(
            "b",
            "A python wheel from PyO3",
            "python packaging with pyo3 produces a wheel. ",
        )],
    );
    Mock::given(method("GET"))
        .and(path("/one.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(one, "application/rss+xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/two.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(two, "application/rss+xml"))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn two_outlets_on_one_story_collapse_to_a_single_row() {
    let ollama = stub_ollama().await;
    let feeds = feeds_server().await;
    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", &ollama.uri())])
        .await
        .unwrap();

    for name in ["one", "two"] {
        stack
            .post_json(
                "/api/feeds",
                json!({ "url": format!("{}/{name}.xml", feeds.uri()) }),
            )
            .await;
    }
    // Three items in, before anything has been embedded.
    assert_eq!(
        stack.get_json("/api/items?view=all").await["items"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    let body = wait_for(&stack, "/api/items?view=all", |b| {
        b["items"].as_array().is_some_and(|a| a.len() == 2)
    })
    .await;
    let items = body["items"].as_array().unwrap();

    let head = items
        .iter()
        .find(|i| i["cluster_size"] == 2)
        .expect("no row stands for two");
    // The unrelated item is untouched — a dedupe that swallowed it would pass a
    // count-only assertion.
    assert!(
        items
            .iter()
            .any(|i| i["title"] == "Tomatoes under glass" && i["cluster_size"] == 1),
        "the unrelated item was clustered too: {items:#?}"
    );

    // Nothing is lost: the row that was hidden is listed on the one that stayed.
    let detail = stack
        .get_json(&format!("/api/items/{}", head["id"].as_i64().unwrap()))
        .await;
    let siblings = detail["siblings"].as_array().unwrap();
    assert_eq!(siblings.len(), 1);
    assert_ne!(siblings[0]["feed_title"], head["feed_title"]);

    // And a search still reaches it. Collapsing is for the list — a search that
    // hid the report you were looking for would be a bug you could not see.
    let found = stack.get_json("/api/items?q=pyo3").await;
    assert_eq!(found["items"].as_array().unwrap().len(), 2);
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_search_by_meaning_finds_what_the_words_do_not() {
    let ollama = stub_ollama().await;
    let feeds = feeds_server().await;
    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", &ollama.uri())])
        .await
        .unwrap();
    stack
        .post_json(
            "/api/feeds",
            json!({ "url": format!("{}/one.xml", feeds.uri()) }),
        )
        .await;

    // Waiting on the semantic result *is* waiting for the embedder. Waiting on
    // the text index instead would prove nothing: FTS is populated by a trigger
    // at insert, so it answers long before a vector exists.
    let semantic = wait_for(&stack, "/api/items?q=greenhouse&mode=semantic", |b| {
        b["items"].as_array().is_some_and(|a| !a.is_empty())
    })
    .await;
    assert_eq!(semantic["mode"], "semantic");
    assert_eq!(semantic["items"][0]["title"], "Tomatoes under glass");
    // Ranked, so no cursor: it is a shortlist, like the interesting view.
    assert!(semantic["next_cursor"].is_null());

    let text = stack.get_json("/api/items?q=greenhouse").await;
    assert_eq!(text["mode"], "text");
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_semantic_search_falls_back_to_the_words_when_the_host_is_asleep() {
    // The reader never depends on the mini being awake. A search box that
    // stopped working because a LAN machine is off would be exactly that.
    let feeds = feeds_server().await;
    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", "http://127.0.0.1:1")])
        .await
        .unwrap();
    stack
        .post_json(
            "/api/feeds",
            json!({ "url": format!("{}/one.xml", feeds.uri()) }),
        )
        .await;

    let body = stack.get_json("/api/items?q=compost&mode=semantic").await;
    assert_eq!(
        body["mode"], "text",
        "a semantic search with no host must answer from the text index"
    );
    assert_eq!(body["items"][0]["title"], "Tomatoes under glass");
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_threshold_of_one_leaves_every_item_alone() {
    let ollama = stub_ollama().await;
    let feeds = feeds_server().await;
    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", &ollama.uri())])
        .await
        .unwrap();
    stack
        .put_json("/api/settings", json!({ "dedupe_threshold": 1.0 }))
        .await;

    for name in ["one", "two"] {
        stack
            .post_json(
                "/api/feeds",
                json!({ "url": format!("{}/{name}.xml", feeds.uri()) }),
            )
            .await;
    }

    // Wait for the embedder to have run — the items still get vectors, they just
    // never join anything. Asserting straight away would pass before the worker
    // had done a thing.
    wait_for(&stack, "/api/items?view=all", |b| {
        b["items"]
            .as_array()
            .is_some_and(|a| a.iter().all(|i| i["summary"].is_string()))
    })
    .await;
    let items = stack.get_json("/api/items?view=all").await;
    assert_eq!(items["items"].as_array().unwrap().len(), 3);
}
