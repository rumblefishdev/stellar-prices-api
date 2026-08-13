//! Portal safety-gate tests (task 0183).
//!
//! There is one environment and it is production, so an unfinished portal slice
//! is publicly reachable the moment it deploys. `PORTAL_ENABLED` is what keeps
//! it dark, and these tests pin the two properties that make the flag worth
//! having: closed really is closed, and closed is **indistinguishable from a
//! route that was never deployed**.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use prices_api::portal::CONFIG_PATH;
use prices_api::{AppConfig, AppState, app};
use tower::ServiceExt;

fn config(portal_enabled: bool) -> AppConfig {
    AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![],
        portal_enabled,
    }
}

async fn send(portal_enabled: bool, uri: &str) -> (StatusCode, Vec<u8>) {
    let resp = app(&config(portal_enabled), AppState::without_ch())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    (status, bytes.to_vec())
}

#[tokio::test]
async fn portal_is_closed_by_default() {
    // The flag defaults to false in `AppConfig::from_env`, and this is the
    // behaviour that default buys.
    let (status, _) = send(false, "/api-tokens/api/auth/login").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// The property the gate exists for. A `403` would confirm the route is there;
/// a `503` would promise it is coming. Both tell a stranger we are building
/// something. An empty `404` tells them nothing at all — and the router has no
/// `fallback`, so that is exactly what a genuinely absent path returns.
#[tokio::test]
async fn a_closed_portal_route_is_byte_identical_to_an_absent_one() {
    let (closed_status, closed_body) = send(false, "/api-tokens/api/key").await;
    let (absent_status, absent_body) = send(false, "/no-such-route-anywhere").await;

    assert_eq!(closed_status, absent_status);
    assert_eq!(closed_body, absent_body);
    assert!(
        closed_body.is_empty(),
        "a closed portal route must not carry a body — an ErrorEnvelope would \
         give the gate away, since that is what a real portal 404 will look \
         like once these routes exist"
    );
}

/// `/config` answers in both states: it *is* the question "is the portal open?",
/// so refusing to answer it while closed would be circular, and [[0185]]'s
/// bundle would have nothing to render its "not yet available" page from.
#[tokio::test]
async fn config_answers_while_closed_and_says_so() {
    let (status, body) = send(false, CONFIG_PATH).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["enabled"], serde_json::Value::Bool(false));
}

#[tokio::test]
async fn config_reports_open_when_the_flag_is_on() {
    let (status, body) = send(true, CONFIG_PATH).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["enabled"], serde_json::Value::Bool(true));
}

/// A stale `enabled: false` cached at a CDN or in a browser would keep the
/// portal dark for that viewer long after it opened, with nothing on screen to
/// explain why.
#[tokio::test]
async fn config_is_never_cached() {
    let resp = app(&config(false), AppState::without_ch())
        .oneshot(
            Request::builder()
                .uri(CONFIG_PATH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.headers()
            .get("cache-control")
            .map(|v| v.to_str().unwrap()),
        Some("no-store")
    );
}

/// With the flag on, the gate steps aside: a portal path that has no handler
/// yet reaches the router and 404s there instead. Same status as the closed
/// case by construction, which is the point — so this asserts the *open* path
/// is live by checking a route that does exist.
#[tokio::test]
async fn opening_the_flag_lets_portal_routes_through() {
    let (status, _) = send(true, CONFIG_PATH).await;
    assert_eq!(status, StatusCode::OK);
}

/// The bundle at `/api-tokens/*` is served by S3 and never reaches this Lambda.
/// The gate must not claim it: `/api-tokens/api/` is ours, `/api-tokens/` is not.
#[tokio::test]
async fn the_gate_does_not_reach_beyond_its_prefix() {
    // Not a route this Lambda serves in either state — but it must 404 as a
    // plain miss, not because the gate decided to own it.
    let (open, _) = send(true, "/api-tokens/dashboard").await;
    let (closed, _) = send(false, "/api-tokens/dashboard").await;
    assert_eq!(open, StatusCode::NOT_FOUND);
    assert_eq!(closed, StatusCode::NOT_FOUND);
}

/// Data routes must be untouched by the flag in both directions — the portal
/// gate is not allowed to become a switch for the partner API.
#[tokio::test]
async fn data_routes_are_unaffected_in_both_states() {
    for enabled in [false, true] {
        let (status, _) = send(enabled, "/health").await;
        assert_eq!(status, StatusCode::OK, "portal_enabled={enabled}");
    }
}
