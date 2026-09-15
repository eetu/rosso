use std::path::Path;

use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::AppState;

pub fn router(state: AppState) -> Router {
    let csp = csp_layer(&state.cfg.static_dir);
    Router::new()
        .route("/status", get(status))
        .merge(crate::api::routes())
        .fallback(get(serve_spa))
        // After `.fallback`, not before: `Router::layer` only wraps what is
        // already in the router, so layering first leaves the SPA document —
        // the one response a CSP actually protects — without the header.
        .layer(csp)
        .with_state(state)
}

/// Liveness probe — unauthenticated, booleans and a version only.
///
/// Always 200, never 503: a 503 just makes the orchestrator kill a container that
/// can heal itself. Report the *resolved* state (did the DB actually answer), not
/// the configured one, so the SPA can hide affordances that won't work.
async fn status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let db_ok = state
        .db
        .with(|c| c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0)))
        .await
        .is_ok();

    // Configured and available are different answers, and only the second one
    // tells the SPA whether to expect summaries: the mini is a desk machine that
    // spends plenty of time asleep with its URL still sitting in the environment.
    let llm_available = state
        .llm_health
        .available(&state.http, state.cfg.ollama_url.as_deref())
        .await;

    Json(json!({
        "service": "rosso",
        "version": env!("CARGO_PKG_VERSION"),
        "db_healthy": db_ok,
        "llm_configured": state.cfg.ollama_url.is_some(),
        "llm_available": llm_available,
        "extract_enabled": state.cfg.extract_enabled,
    }))
}

/// Serve a real built asset under `static_dir`, else `index.html` with **200** so
/// the client router owns the route. Deliberately not
/// `ServeDir.not_found_service(ServeFile)`: that leaks a 404 status onto every
/// client route, so a hard refresh on a sub-route returns 404 with the shell body.
/// canonicalize + `starts_with` rejects `..` traversal.
async fn serve_spa(State(state): State<AppState>, uri: Uri) -> Response {
    let base = &state.cfg.static_dir;
    let rel = uri.path().trim_start_matches('/');
    if !rel.is_empty() {
        let cand = base.join(rel);
        if let (Ok(c), Ok(b)) = (cand.canonicalize(), base.canonicalize()) {
            if c.starts_with(&b) && c.is_file() {
                if let Ok(bytes) = tokio::fs::read(&c).await {
                    let mime = mime_guess::from_path(&c).first_or_octet_stream();
                    return ([(header::CONTENT_TYPE, mime.as_ref())], bytes).into_response();
                }
            }
        }
    }
    match tokio::fs::read_to_string(base.join("index.html")).await {
        Ok(html) => Html(html).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// Same-origin baseline plus the Google Fonts hosts halo-design uses.
///
/// `img-src` allows any https origin: feed content embeds images from wherever the
/// publisher hosts them, and stripping them would gut the reader. Everything else
/// stays locked down. HSTS / X-Frame-Options / X-Content-Type-Options are Traefik's
/// job, not the binary's.
fn csp_layer(static_dir: &Path) -> SetResponseHeaderLayer<HeaderValue> {
    // SvelteKit boots the app from an *inline* script in index.html whose body
    // carries a per-build random identifier, so no hash can be written down
    // here — and `script-src 'self'` alone silently refuses to start the SPA.
    // Hashing what is actually in the file being served keeps 'unsafe-inline'
    // out and needs no attention after a rebuild.
    let hashes: String = inline_script_hashes(static_dir)
        .into_iter()
        .map(|hash| format!(" '{hash}'"))
        .collect();

    let csp = format!(
        "default-src 'self'; \
         script-src 'self'{hashes}; \
         style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
         font-src 'self' data: https://fonts.gstatic.com; \
         img-src 'self' data: blob: https:; \
         media-src 'self' https:; \
         connect-src 'self'; \
         manifest-src 'self'; \
         frame-ancestors 'none'; \
         base-uri 'self'; \
         object-src 'none'; \
         form-action 'self'"
    );

    let value = HeaderValue::from_str(&csp).unwrap_or_else(|_| {
        tracing::error!("computed CSP was not a valid header value; falling back");
        HeaderValue::from_static("default-src 'self'")
    });
    SetResponseHeaderLayer::if_not_present(header::CONTENT_SECURITY_POLICY, value)
}

/// `sha256-…` for every inline `<script>` in the served `index.html`.
fn inline_script_hashes(static_dir: &Path) -> Vec<String> {
    let Ok(html) = std::fs::read_to_string(static_dir.join("index.html")) else {
        // No shell to serve, so nothing to allow. The API still answers.
        return Vec::new();
    };
    inline_scripts(&html)
        .iter()
        .map(|body| sha256_base64(body))
        .collect()
}

/// Bodies of `<script>` elements that have no `src`.
fn inline_scripts(html: &str) -> Vec<&str> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut cursor = 0usize;

    while let Some(found) = lower[cursor..].find("<script") {
        let open = cursor + found;
        let Some(rel) = lower[open..].find('>') else {
            break;
        };
        let body_start = open + rel + 1;
        let tag = &lower[open..body_start];
        let Some(rel_end) = lower[body_start..].find("</script") else {
            break;
        };
        let body_end = body_start + rel_end;

        // A script with a `src` loads from the origin and is already covered by
        // 'self'; only inline bodies need a hash.
        if !tag.contains(" src=") {
            out.push(&html[body_start..body_end]);
        }
        cursor = body_end;
    }
    out
}

fn sha256_base64(body: &str) -> String {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(body.as_bytes());
    format!(
        "sha256-{}",
        base64::engine::general_purpose::STANDARD.encode(digest)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_inline_scripts_are_hashed() {
        let html = r#"<html><head>
            <script src="/_app/start.js"></script>
          </head><body>
            <script>
              { __sveltekit_9tz2ab = { base: "" }; }
            </script>
          </body></html>"#;
        let scripts = inline_scripts(html);
        assert_eq!(scripts.len(), 1, "got {scripts:?}");
        assert!(scripts[0].contains("__sveltekit_9tz2ab"));
    }

    #[test]
    fn a_hash_is_the_browser_format_and_covers_the_exact_body() {
        assert_eq!(
            sha256_base64(""),
            "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        );
        // Whitespace is part of what a browser hashes, so it is part of this too.
        assert_ne!(sha256_base64("a"), sha256_base64(" a"));
    }

    #[test]
    fn a_page_with_no_inline_script_adds_nothing() {
        assert!(inline_scripts("<html><body><p>hi</p></body></html>").is_empty());
        assert!(inline_scripts("<script src=\"/a.js\"></script>").is_empty());
    }
}
