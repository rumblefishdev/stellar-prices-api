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
#[utoipa::path(
    get,
    path = "/health",
    tag = "ops",
    responses((status = 200, description = "Service is healthy"))
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
