//! The prompts and response schemas, kept apart from the orchestration because
//! they are the part that actually gets tuned.

use serde_json::json;

use crate::store::FeedbackExample;

/// How much of an article the model sees. Enough for it to know what the piece
/// says; short enough that a long read does not become a long generation on a
/// machine doing one item at a time.
const CONTENT_CHARS: usize = 4000;

/// Past thumbs shown to the model as calibration. A handful is steering; a long
/// list crowds out the article itself.
const MAX_EXAMPLES: usize = 8;

const RULES_SUMMARY: &str = "\
- summary: two sentences saying what the item actually reports, concretely. \
Name the thing it is about. No preamble, no \"this article\", no hedging.
- tags: one to four lowercase topic tags.";

const RULES_SCORE: &str = "\
- score: 0-100, how well this matches their stated interests. 0 is irrelevant to \
them, 100 is squarely what they asked for. Judge fit to those interests, not how \
good the writing is.
- reason: one short clause saying why that score.";

pub fn summary_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "summary": { "type": "string" },
            "tags": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["summary", "tags"]
    })
}

pub fn enrich_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "summary": { "type": "string" },
            "score": { "type": "integer", "minimum": 0, "maximum": 100 },
            "reason": { "type": "string" },
            "tags": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["summary", "score", "reason", "tags"]
    })
}

pub fn score_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "score": { "type": "integer", "minimum": 0, "maximum": 100 },
            "reason": { "type": "string" }
        },
        "required": ["score", "reason"]
    })
}

/// System prompt for the first pass over an item.
///
/// With no interest profile there is nothing to score against, so the prompt
/// asks only for a summary — a number invented without a yardstick would look
/// exactly like a real one in the UI.
pub fn enrich_system(profile: &str, examples: &[FeedbackExample]) -> String {
    let profile = profile.trim();
    if profile.is_empty() {
        return format!(
            "You summarize feed items for a reader.\n\nReturn JSON only.\n{RULES_SUMMARY}"
        );
    }
    format!(
        "You read feeds on someone's behalf and judge what is worth their time.\n\n\
         Their interests, in their own words:\n{profile}\n\n\
         Return JSON only.\n{RULES_SUMMARY}\n{RULES_SCORE}{}",
        examples_block(examples)
    )
}

/// System prompt for re-scoring an item that already has a summary.
pub fn score_system(profile: &str, examples: &[FeedbackExample]) -> String {
    format!(
        "You judge whether a feed item matches someone's interests.\n\n\
         Their interests, in their own words:\n{}\n\n\
         Return JSON only.\n{RULES_SCORE}{}",
        profile.trim(),
        examples_block(examples)
    )
}

/// The item, as the model sees it.
pub fn item_user(feed_title: &str, title: &str, body: Option<&str>) -> String {
    let mut out = format!("Feed: {feed_title}\nTitle: {title}\n");
    if let Some(body) = body {
        let excerpt: String = body.chars().take(CONTENT_CHARS).collect();
        out.push('\n');
        out.push_str(&excerpt);
    }
    out
}

/// Thumbs the reader has given, as calibration.
fn examples_block(examples: &[FeedbackExample]) -> String {
    if examples.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\nTheir past verdicts, for calibration:");
    for example in examples.iter().take(MAX_EXAMPLES) {
        let verdict = if example.feedback > 0 {
            "wanted"
        } else {
            "did not want"
        };
        out.push_str(&format!("\n- {verdict}: {}", example.title.trim()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example(title: &str, feedback: i64) -> FeedbackExample {
        FeedbackExample {
            title: title.into(),
            feedback,
        }
    }

    #[test]
    fn without_a_profile_the_model_is_never_asked_to_score() {
        let system = enrich_system("  ", &[]);
        assert!(
            !system.contains("score"),
            "asked for a score anyway: {system}"
        );
        assert!(system.contains("summary"));
    }

    #[test]
    fn with_a_profile_the_prompt_carries_it_verbatim() {
        let system = enrich_system("rust, sqlite, no crypto", &[]);
        assert!(system.contains("rust, sqlite, no crypto"));
        assert!(system.contains("score"));
    }

    #[test]
    fn thumbs_become_labelled_calibration_lines() {
        let system = enrich_system(
            "rust",
            &[example("A good one", 1), example("A bad one", -1)],
        );
        assert!(system.contains("wanted: A good one"));
        assert!(system.contains("did not want: A bad one"));
    }

    #[test]
    fn the_body_is_capped_so_one_long_read_cannot_dominate_a_run() {
        let body = "x".repeat(CONTENT_CHARS * 3);
        let user = item_user("Feed", "Title", Some(&body));
        assert!(user.chars().count() < CONTENT_CHARS + 200);
    }

    #[test]
    fn an_item_with_no_body_still_produces_a_usable_prompt() {
        let user = item_user("Hacker News", "JetKVM Mini", None);
        assert!(user.contains("JetKVM Mini"));
        assert!(user.contains("Hacker News"));
    }
}
