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
/// 300s — documents that are static for the life of a deployment and change
/// only when a new build ships (the OpenAPI spec, task 0124).
///
/// Deliberately SHORTER than the 3600s API Gateway TTL on the same route, which
/// is the one place this module's tiers and the stage cache do not agree. The
/// reason: "static for the life of a deployment" is true, but the caches outlive
/// the deployment that filled them. At 3600s a partner who fetched the document
/// minutes before a release would keep generating clients from the old one for
/// the rest of the hour, with nothing telling them it was stale — and that is
/// exactly when integrators go look at it. The gateway entry is flushed at
/// deploy (`make -C infra flush-production-cache`), which the operator controls;
/// a partner's HTTP cache is not, so the client-facing window is the one that
/// has to be short. Revalidating every 5 minutes costs nothing: those requests
/// land on the gateway cache, not the Lambda.
pub const DEPLOY_STATIC: HeaderValue = HeaderValue::from_static("public, max-age=300");
/// Never store — error responses.
pub const NO_STORE: HeaderValue = HeaderValue::from_static("no-store");

/// Attach a `Cache-Control` value to a response, replacing any existing one.
pub fn attach(resp: &mut Response, value: HeaderValue) {
    resp.headers_mut().insert(CACHE_CONTROL, value);
}
