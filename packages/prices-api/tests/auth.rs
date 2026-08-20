//! Phase 1 auth-gate tests: when `API_KEYS` is configured, the in-app gate
//! rejects keyless/wrong-key requests to non-exempt paths (401) while letting
//! valid keys through and keeping `/health` + `/api-docs-json` exempt.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use prices_api::{AppConfig, AppState, app};
use tower::ServiceExt;

const KEY: &str = "test-secret-key";

fn armed_config() -> AppConfig {
    armed_config_with_portal(false)
}

fn armed_config_with_portal(portal_enabled: bool) -> AppConfig {
    AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![KEY.to_string()],
        portal_enabled,
        // Sign-in credentials are loaded asynchronously from Secrets Manager
        // (task 0186) and are never part of the environment; `None` is the shape
        // every non-portal test wants.
        portal_oauth: None,
        // Discord endpoints are part of the config now, not read from the
        // process environment per router — see `AppConfig::portal_endpoints`.
        portal_endpoints: Default::default(),
        // Task 0187: the control-plane client for self-service keys. `None`
        // is what every non-portal test wants — with no client in the
        // config there is no code path here that can reach API Gateway.
        portal_keys: None,
        portal_eligibility: None,
    }
}

async fn send(req: Request<Body>) -> StatusCode {
    send_with(&armed_config(), req).await
}

async fn send_with(config: &AppConfig, req: Request<Body>) -> StatusCode {
    app(config, AppState::without_ch())
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

/// The portal's exemption is **conditional on the portal being open**, and both
/// halves are load-bearing.
///
/// Open: its routes must answer without a key, because a visitor signing in has
/// none by definition — gating self-service onboarding behind the credential it
/// hands out is a closed loop.
#[tokio::test]
async fn portal_paths_are_exempt_when_the_portal_is_open() {
    let status = send_with(
        &armed_config_with_portal(true),
        Request::builder()
            .uri(prices_api::portal::CONFIG_PATH)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

/// Closed: the exemption is withdrawn, so a portal path answers `401` like any
/// other unknown path rather than an empty `404`. Exempting it unconditionally
/// would make the portal prefix the only unauthenticated surface on an armed
/// service — uniquely fingerprintable, which is the disclosure the portal gate
/// exists to prevent. Kept in step with `tests/portal.rs`.
#[tokio::test]
async fn portal_paths_are_not_exempt_when_the_portal_is_closed() {
    let portal = send(
        Request::builder()
            .uri("/api-tokens/api/key")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let absent = send(
        Request::builder()
            .uri("/v1/anything")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(portal, StatusCode::UNAUTHORIZED);
    assert_eq!(portal, absent);
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
