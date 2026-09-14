use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/status", get(status))
        .merge(crate::api::routes())
        .fallback(get(serve_spa))
        // After `.fallback`, not before: `Router::layer` only wraps what is
        // already in the router, so layering first leaves the SPA document —
        // the one response a CSP actually protects — without the header.
        .layer(csp_layer())
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
fn csp_layer() -> SetResponseHeaderLayer<HeaderValue> {
    const CSP: &str = "default-src 'self'; \
         script-src 'self'; \
         style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
         font-src 'self' data: https://fonts.gstatic.com; \
         img-src 'self' data: blob: https:; \
         media-src 'self' https:; \
         connect-src 'self'; \
         manifest-src 'self'; \
         frame-ancestors 'none'; \
         base-uri 'self'; \
         object-src 'none'; \
         form-action 'self'";
    SetResponseHeaderLayer::if_not_present(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    )
}
