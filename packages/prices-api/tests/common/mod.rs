//! Shared helpers for the CH-less negative-input tests (task 0119).
//!
//! Every test here drives the real router with [`AppState::without_ch`], which
//! panics on any ClickHouse access — so a clean `400` also proves the rule
//! rejected *before* the DB call (task 0119 AC 7).
// Each test binary compiles this module separately and uses a subset of the
// helpers, so "unused" here is per-binary noise, not real dead code.
#![allow(dead_code)]

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use prices_api::{AppConfig, AppState, app};
use tower::ServiceExt;

/// The wired application router with no ClickHouse behind it.
pub fn app_without_ch() -> Router {
    let config = AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![],
        portal_enabled: false,
    };
    app(&config, AppState::without_ch())
}

/// Send `req` through a fresh router; return status, headers, and the body
/// parsed as JSON (panics with the raw body in the message when it isn't —
/// a plain-text error body is itself a test failure worth seeing).
pub async fn send(req: Request<Body>) -> (StatusCode, HeaderMap, serde_json::Value) {
    let response = app_without_ch().oneshot(req).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
        panic!(
            "non-JSON body (status {status}): {}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, headers, json)
}

/// GET `uri` and return `(status, headers, json_body)`.
pub async fn get(uri: &str) -> (StatusCode, HeaderMap, serde_json::Value) {
    send(Request::builder().uri(uri).body(Body::empty()).unwrap()).await
}
