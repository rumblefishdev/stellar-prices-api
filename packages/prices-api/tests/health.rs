//! Phase 0 smoke test: `GET /health` returns 200 end-to-end through the real
//! router, exercised in-process via tower's `oneshot` (no network, no Lambda
//! runtime). This is the same `app()` the Lambda entrypoint serves, so a green
//! test proves the router is wired correctly.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use prices_api::{AppState, app};
use tower::ServiceExt; // for `oneshot`

#[tokio::test]
async fn health_returns_200_ok() {
    let router = app(AppState::without_ch());

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
