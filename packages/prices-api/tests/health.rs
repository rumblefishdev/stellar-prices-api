//! Phase 0/1 smoke tests: `GET /health` and `GET /api-docs-json` answer 200
//! end-to-end through the real `app()` router, exercised in-process via tower's
//! `oneshot` (no network, no Lambda runtime).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use prices_api::{AppConfig, AppState, app};
use tower::ServiceExt; // for `oneshot`

/// Config with auth disarmed (no API keys) and no CH — the default test setup.
fn test_config() -> AppConfig {
    AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![],
        portal_enabled: false,
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
        portal_rate_limit: None,
    }
}

#[tokio::test]
async fn health_returns_200_ok() {
    let router = app(&test_config(), AppState::without_ch());

    let response = router
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["service"], "prices-api");
}

#[tokio::test]
async fn openapi_spec_is_served() {
    let router = app(&test_config(), AppState::without_ch());

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api-docs-json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // The /health path is documented in the emitted spec.
    assert!(json["paths"]["/health"].is_object());
    assert_eq!(json["info"]["title"], "Stellar Prices API");
}
