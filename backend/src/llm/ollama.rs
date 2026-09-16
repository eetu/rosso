//! The Ollama calls rosso makes.
//!
//! One shape: a non-streaming chat that must come back as JSON matching a
//! schema. Ollama's `format` field takes a full JSON Schema and constrains
//! decoding to it, which is why nothing here parses prose.

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;

/// Generation is slow and occasionally very slow — a cold model load alone can
/// run past ten seconds, and the big model on a busy box is worse. This is far
/// above the shared client's 60 s, so it is always set explicitly.
const GENERATE_TIMEOUT: Duration = Duration::from_secs(240);

/// Embedding is a forward pass, not a generation — far quicker than a summary,
/// but a batch still pays the cold-load cost the first time.
const EMBED_TIMEOUT: Duration = Duration::from_secs(120);

/// Keep the model resident between items. The worker runs one item after
/// another, so paying the load cost each time would dominate the run.
const KEEP_ALIVE: &str = "15m";

#[derive(Debug, Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: String,
}

/// Ask the model for a value of `T`, constrained by `schema`.
pub async fn chat_json<T: DeserializeOwned>(
    http: &reqwest::Client,
    base: &str,
    model: &str,
    system: &str,
    user: &str,
    schema: serde_json::Value,
) -> anyhow::Result<T> {
    let body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
        "stream": false,
        "format": schema,
        // Judgements should not wander between runs; the same item scored twice
        // an hour apart reading differently is worse than either answer.
        "options": { "temperature": 0 },
        "keep_alive": KEEP_ALIVE,
    });

    let res = http
        .post(format!("{base}/api/chat"))
        .timeout(GENERATE_TIMEOUT)
        .json(&body)
        .send()
        .await?
        .error_for_status()?;

    let parsed: ChatResponse = res.json().await?;
    let content = extract_json(parsed.message.content.trim());
    serde_json::from_str(content).map_err(|err| {
        anyhow::anyhow!(
            "model returned unusable json ({err}): {}",
            truncate(content, 200)
        )
    })
}

/// Embed a batch of texts.
///
/// `/api/embed` takes `input` as an array and answers in one round trip; the
/// older `/api/embeddings` is one request per string, which over a backlog is the
/// same work plus a few hundred handshakes.
///
/// The width is whatever the model returns — `embeddinggemma:300m` gives 768,
/// measured rather than assumed, and the caller records it alongside the vector
/// so a model swap is detectable instead of silently comparing across two
/// different vector spaces.
pub async fn embed(
    http: &reqwest::Client,
    base: &str,
    model: &str,
    inputs: &[String],
) -> anyhow::Result<Vec<Vec<f32>>> {
    #[derive(Deserialize)]
    struct EmbedResponse {
        embeddings: Vec<Vec<f32>>,
    }

    let res = http
        .post(format!("{base}/api/embed"))
        .timeout(EMBED_TIMEOUT)
        .json(&json!({ "model": model, "input": inputs, "keep_alive": KEEP_ALIVE }))
        .send()
        .await?
        .error_for_status()?;

    let parsed: EmbedResponse = res.json().await?;
    // A short answer would silently pair vectors with the wrong items.
    anyhow::ensure!(
        parsed.embeddings.len() == inputs.len(),
        "asked for {} embeddings, got {}",
        inputs.len(),
        parsed.embeddings.len()
    );
    Ok(parsed.embeddings)
}

/// List the models the host has installed.
pub async fn models(http: &reqwest::Client, base: &str) -> anyhow::Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Tags {
        models: Vec<Model>,
    }
    #[derive(Deserialize)]
    struct Model {
        name: String,
    }

    let res = http
        .get(format!("{base}/api/tags"))
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .error_for_status()?;
    let tags: Tags = res.json().await?;
    Ok(tags.models.into_iter().map(|m| m.name).collect())
}

/// Find the JSON in a model's answer.
///
/// `format` constrains decoding, but not the packaging around it. Observed from
/// gemma on the real host: a bare `json` line before the object, with no code
/// fence at all — which is not a fence to strip and not JSON to parse, and threw
/// away otherwise perfect answers. Fences appear too. So rather than enumerate
/// wrappers, take the outermost braces and parse what is between them.
fn extract_json(content: &str) -> &str {
    let unfenced = strip_code_fence(content);
    let Some(start) = unfenced.find(['{', '[']) else {
        return unfenced;
    };
    let opener = unfenced.as_bytes()[start];
    let closer = if opener == b'{' { '}' } else { ']' };
    match unfenced.rfind(closer) {
        Some(end) if end > start => &unfenced[start..=end],
        _ => unfenced,
    }
}

fn strip_code_fence(content: &str) -> &str {
    let Some(rest) = content.strip_prefix("```") else {
        return content;
    };
    // ```json\n{...}\n```
    let rest = rest.split_once('\n').map_or(rest, |(_lang, body)| body);
    rest.rsplit_once("```")
        .map_or(rest, |(body, _)| body)
        .trim()
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fenced_answers_are_unwrapped() {
        assert_eq!(extract_json("{\"a\":1}"), "{\"a\":1}");
        assert_eq!(extract_json("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(extract_json("```\n{\"a\":1}\n```"), "{\"a\":1}");
    }

    #[test]
    fn an_unterminated_fence_still_yields_its_body() {
        assert_eq!(extract_json("```json\n{\"a\":1}"), "{\"a\":1}");
    }

    #[test]
    fn a_bare_language_line_before_the_object_is_ignored() {
        // Seen from gemma on the real host: no fence, just the word `json` on
        // its own line. Four perfectly good answers were discarded over it.
        assert_eq!(extract_json("json\n{\"a\":1}"), "{\"a\":1}");
        assert_eq!(
            extract_json("Here you go:\n{\"a\":1}\nHope that helps."),
            "{\"a\":1}"
        );
    }

    #[test]
    fn an_answer_with_no_json_at_all_is_passed_through_to_fail_loudly() {
        assert_eq!(
            extract_json("I'm afraid I can't do that."),
            "I'm afraid I can't do that."
        );
    }

    #[test]
    fn nested_objects_keep_their_closing_brace() {
        assert_eq!(
            extract_json("json\n{\"a\":{\"b\":1},\"c\":2}"),
            "{\"a\":{\"b\":1},\"c\":2}"
        );
    }
}
