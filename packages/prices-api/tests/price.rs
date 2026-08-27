//! Phase 2 `/v1/assets/{id}/price` tests that don't need ClickHouse.
//!
//! The happy path (200 with real data) needs a live CH and lives in a
//! CH-gated integration test (added when local CH is stood up). Here we cover
//! the pre-DB behavior: a malformed identifier 400s before any CH call, so the
//! route is reachable and validation runs first.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use prices_api::{AppConfig, AppState, app};
use tower::ServiceExt;

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
        portal_eligibility: None,
        portal_rate_limit: None,
    }
}

#[tokio::test]
async fn price_with_invalid_identifier_is_400_without_touching_ch() {
    // CH-less state: if the handler reached `state.ch()` it would panic; a clean
    // 400 proves validation short-circuits before the DB call.
    let response = app(&test_config(), AppState::without_ch())
        .oneshot(
            Request::builder()
                .uri("/v1/assets/zzz-not-an-asset/price")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], "invalid_id");
}

#[tokio::test]
async fn price_path_is_documented_in_openapi() {
    let response = app(&test_config(), AppState::without_ch())
        .oneshot(
            Request::builder()
                .uri("/api-docs-json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["paths"]["/v1/assets/{asset_identifier}/price"].is_object(),
        "price path missing from OpenAPI spec"
    );
}

/// Task 0118 — `?min_volume_usd=` validation runs before any CH call: the
/// CH-less state would panic if the handler reached `state.ch()`.
#[tokio::test]
async fn price_with_invalid_min_volume_is_400_without_touching_ch() {
    // serde rejects non-numeric text; the handler itself must reject values
    // f64::from_str happily parses (NaN, inf), negatives, and absurd
    // magnitudes — 0119's contract: every invalid input is a 400 in the
    // standard envelope.
    for bad in ["abc", "-1", "NaN", "inf", "1e16", ""] {
        let response = app(&test_config(), AppState::without_ch())
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/assets/native/price?min_volume_usd={bad}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "min_volume_usd={bad}"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], "invalid_query", "min_volume_usd={bad}");
    }
}
