//! Forward-auth gate.
//!
//! rosso sits behind oauth2-proxy: Traefik asks the proxy before routing, and the
//! proxy hands back `X-Auth-Request-User`. Traefik deletes header aliases at the
//! entry point, so the header cannot be spoofed from outside. The binary only
//! asserts the edge vouched — 401 when the header is absent, as defence in depth
//! against being reached by something other than the edge.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;

use crate::AppState;

const HDR_USER: &str = "x-auth-request-user";

/// Proof that a request is authenticated. Add `_: Auth` to every `/api/*` handler.
/// Bypassed by `cfg.dev_auth` (`DEV_AUTH=1` / `ROSSO_OPEN=1`), which is how local
/// dev and the integration harness reach `/api/*`. `/status` stays unauth.
pub struct Auth;

impl FromRequestParts<AppState> for Auth {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if state.cfg.dev_auth {
            return Ok(Auth);
        }
        let user = parts
            .headers
            .get(HDR_USER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if user.is_empty() {
            return Err((StatusCode::UNAUTHORIZED, "unauthorized"));
        }
        Ok(Auth)
    }
}
