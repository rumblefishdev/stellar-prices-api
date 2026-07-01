//! In-app `X-API-Key` authentication (ADR 0008).
//!
//! Ported from BE's `api/src/auth`, trimmed to the API-key path (no JWT /
//! Turnstile — those are BE's free-tier concern). Keys are compared in constant
//! time. The per-key 100 req/s throttle lives at the API Gateway usage-plan;
//! this gate only enforces *presence of a valid key*, as defense-in-depth and
//! for local/non-gateway runs.
//!
//! The gate is **armed only when keys are configured** (`API_KEYS` non-empty),
//! so unconfigured local/dev and the early load test run open. `/health` and
//! `/api-docs-json` are always exempt.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::common::errors;
use crate::config::AppConfig;

/// Cloneable auth state carried by the middleware.
#[derive(Clone)]
pub struct AuthConfig {
    api_keys: Arc<Vec<String>>,
}

/// Layer the API-key gate onto `router` when keys are configured; otherwise
/// return it unchanged (gate disarmed).
pub fn apply(router: axum::Router, config: &AppConfig) -> axum::Router {
    if config.api_keys.is_empty() {
        return router;
    }
    let auth = AuthConfig {
        api_keys: Arc::new(config.api_keys.clone()),
    };
    router.layer(axum::middleware::from_fn_with_state(auth, require_api_key))
}

/// Paths that never require a key.
fn is_exempt(path: &str) -> bool {
    matches!(path, "/health" | "/api-docs-json")
}

/// Reject any request that lacks a valid `X-API-Key` (except exempt paths).
pub async fn require_api_key(State(auth): State<AuthConfig>, req: Request, next: Next) -> Response {
    if is_exempt(req.uri().path()) {
        return next.run(req).await;
    }
    let provided = req.headers().get("x-api-key").and_then(|v| v.to_str().ok());
    let ok = matches!(provided, Some(key)
        if auth.api_keys.iter().any(|k| ct_eq(k.as_bytes(), key.as_bytes())));
    if ok {
        next.run(req).await
    } else {
        errors::unauthorized("missing or invalid API key")
    }
}

/// Constant-time byte-slice equality. Length mismatch returns early (length is
/// not secret); equal-length inputs are compared with a branch-free XOR
/// accumulation so timing does not reveal how many bytes matched. Mirrors BE's
/// `ct_eq`.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::ct_eq;

    #[test]
    fn ct_eq_matches_identical() {
        assert!(ct_eq(b"secret-key", b"secret-key"));
    }

    #[test]
    fn ct_eq_rejects_different_content_same_len() {
        assert!(!ct_eq(b"secret-key", b"secret-keX"));
    }

    #[test]
    fn ct_eq_rejects_different_len() {
        assert!(!ct_eq(b"short", b"longer-key"));
        assert!(!ct_eq(b"", b"x"));
    }
}
