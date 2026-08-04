//! `Cache-Control` tiers as reusable header values + an `attach` helper. Ported
//! from BE so both APIs share one caching vocabulary. The API Gateway stage
//! cache (Phase 4) keys its per-endpoint TTLs off the same tiering.

use axum::http::HeaderValue;
use axum::http::header::CACHE_CONTROL;
use axum::response::Response;

/// No client/edge caching; always revalidate. For volatile point reads and
/// liveness probes.
pub const LIVE: HeaderValue = HeaderValue::from_static("max-age=0, must-revalidate");
/// 10s freshness window — very short-lived data.
pub const SHORT: HeaderValue = HeaderValue::from_static("public, max-age=10");
/// 60s — list/aggregate endpoints.
pub const MEDIUM: HeaderValue = HeaderValue::from_static("public, max-age=60");
/// 300s — slow-moving data.
pub const LONG: HeaderValue = HeaderValue::from_static("public, max-age=300");
/// 3600s — documents that are static for the life of a deployment and change
/// only when a new build ships (the OpenAPI spec, task 0124).
pub const DEPLOY_STATIC: HeaderValue = HeaderValue::from_static("public, max-age=3600");
/// Never store — error responses.
pub const NO_STORE: HeaderValue = HeaderValue::from_static("no-store");

/// Attach a `Cache-Control` value to a response, replacing any existing one.
pub fn attach(resp: &mut Response, value: HeaderValue) {
    resp.headers_mut().insert(CACHE_CONTROL, value);
}
