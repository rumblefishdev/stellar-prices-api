//! Phase 1 auth-gate tests: when `API_KEYS` is configured, the in-app gate
//! rejects keyless/wrong-key requests to non-exempt paths (401) while letting
//! valid keys through and keeping `/health` + `/api-docs-json` exempt.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use prices_api::{AppConfig, AppState, app};
use tower::ServiceExt;

const KEY: &str = "test-secret-key";

fn armed_config() -> AppConfig {
    AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![KEY.to_string()],
    }
}

async fn send(req: Request<Body>) -> StatusCode {
    app(&armed_config(), AppState::without_ch())
        .oneshot(req)
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn protected_path_without_key_is_401() {
    let status = send(
        Request::builder()
            .uri("/v1/anything")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_path_with_wrong_key_is_401() {
    let status = send(
        Request::builder()
            .uri("/v1/anything")
            .header("x-api-key", "wrong")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_path_with_valid_key_passes_gate() {
    // Valid key clears the gate; there is no such route yet, so it 404s
    // *after* auth — proving the gate let it through (not 401).
    let status = send(
        Request::builder()
            .uri("/v1/anything")
            .header("x-api-key", KEY)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn health_is_exempt_even_when_armed() {
    let status = send(
        Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn openapi_is_exempt_even_when_armed() {
    let status = send(
        Request::builder()
            .uri("/api-docs-json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
