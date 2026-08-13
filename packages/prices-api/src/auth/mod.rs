//! In-app `X-API-Key` authentication (ADR 0008).
//!
//! Ported from BE's `api/src/auth`, trimmed to the API-key path (no JWT /
//! Turnstile — those are BE's free-tier concern). Keys are compared in constant
//! time. The per-key rate limit and monthly quota live at the API Gateway
//! usage-plan (sized by task 0157, not the design doc's 100 req/s); this gate
//! only enforces *presence of a valid key*, as defense-in-depth and for
//! local/non-gateway runs.
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
    portal_open: bool,
}

/// Layer the API-key gate onto `router` when keys are configured; otherwise
/// return it unchanged (gate disarmed).
pub fn apply(router: axum::Router, config: &AppConfig) -> axum::Router {
    if config.api_keys.is_empty() {
        return router;
    }
    let auth = AuthConfig {
        api_keys: Arc::new(config.api_keys.clone()),
        portal_open: config.portal_enabled,
    };
    router.layer(axum::middleware::from_fn_with_state(auth, require_api_key))
}

/// Paths that never require a key.
///
/// The portal's own backend (`crate::portal`) is exempt as a prefix, not as a
/// list: a visitor signing in has no API key by definition, so gating those
/// routes behind one would make self-service onboarding impossible to enter.
/// Every route a later slice adds under the prefix inherits that without
/// editing this function.
///
/// **But only while the portal is open**, and that condition is the whole
/// point. `portal`'s gate answers a closed portal with an empty `404` chosen to
/// be byte-identical to a path that was never deployed. Exempting the prefix
/// unconditionally breaks exactly that property the moment `API_KEYS` is
/// armed: every other unknown path would answer `401` with an `ErrorEnvelope`,
/// so the portal prefix would become the only unauthenticated surface on the
/// service and thereby uniquely fingerprintable — the disclosure the gate
/// exists to prevent. Closed, the prefix is not exempt, falls through to the
/// checks below and looks like everything else: `401` when keys are armed,
/// empty `404` when they are not. Open, it is public by design and being
/// distinguishable costs nothing.
fn is_exempt(path: &str, portal_open: bool) -> bool {
    matches!(path, "/health" | "/api-docs-json")
        || (portal_open && path.starts_with(crate::portal::PORTAL_API_PREFIX))
}

/// Reject any request that lacks a valid `X-API-Key` (except exempt paths).
pub async fn require_api_key(State(auth): State<AuthConfig>, req: Request, next: Next) -> Response {
    if is_exempt(req.uri().path(), auth.portal_open) {
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
