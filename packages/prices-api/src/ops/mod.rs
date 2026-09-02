//! Operational (non-versioned) endpoints: liveness/readiness. Lives outside
//! `/v1` and, like BE's `ops`, is exempt from auth/edge gates when those land.

use axum::Json;
use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;

use crate::common::cache_control;
use crate::state::AppState;

/// Router for operational routes (health now; metrics later).
pub fn router() -> Router<AppState> {
    Router::new().route("/health", get(health))
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    service: &'static str,
}

/// `GET /health` — liveness probe. Returns 200 with a tiny JSON body and a
/// non-cacheable `Cache-Control`. Deliberately does NOT touch ClickHouse, so it
/// answers even when the CH client is absent (mirrors BE's health exemption).
// Explicit `summary`/`description`: without them utoipa publishes the whole
// rustdoc above as the summary — 223 characters of maintainer-facing prose
// where a one-line label belongs (task 0124).
#[utoipa::path(
    get,
    path = "/health",
    tag = "ops",
    summary = "`GET /health` — liveness probe.",
    description = "Returns 200 with a small JSON body carrying `\"status\": \"ok\"` and a \
                   non-cacheable `Cache-Control`. Answered at the edge without \
                   reaching the service, so it reports that the API is up — not that \
                   its data is fresh, and not that the database is reachable.",
    // Opts out of the global `x-api-key` requirement — /health is a keyless
    // API Gateway mock and is exempt from the in-app gate (task 0124).
    security(()),
    responses(
        (status = 200, description = "The API is up"),
        // Keyless does not mean unthrottled: the stage-wide throttle covers
        // `/*` `*`, so API Gateway can 429 this route too. No body — the
        // gateway produces this response, not the handler (there is none: this
        // is a MockIntegration). Documented so a generated client has an error
        // branch instead of trying to deserialize `{"message": …}` as `Health`.
        (status = 429, description = "Rate limit exceeded"),
    )
)]
pub async fn health() -> Response {
    let mut resp = (
        StatusCode::OK,
        Json(Health {
            status: "ok",
            service: "prices-api",
        }),
    )
        .into_response();
    cache_control::attach(&mut resp, cache_control::LIVE);
    resp
}
