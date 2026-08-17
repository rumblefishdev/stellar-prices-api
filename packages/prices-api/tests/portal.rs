//! Portal safety-gate tests (task 0183).
//!
//! There is one environment and it is production, so an unfinished portal slice
//! is publicly reachable the moment it deploys. `PORTAL_ENABLED` is what keeps
//! it dark, and these tests pin the two properties that make the flag worth
//! having: closed really is closed, and closed is **indistinguishable from a
//! route that was never deployed**.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use prices_api::portal::{CONFIG_PATH, PortalGate, gate_portal};
use prices_api::{AppConfig, AppState, app};
use tower::ServiceExt;

fn config(portal_enabled: bool) -> AppConfig {
    config_with_keys(portal_enabled, vec![])
}

fn config_with_keys(portal_enabled: bool, api_keys: Vec<String>) -> AppConfig {
    AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys,
        portal_enabled,
        // Sign-in credentials are loaded asynchronously from Secrets Manager
        // (task 0186) and are never part of the environment; `None` is the shape
        // every non-portal test wants.
        portal_oauth: None,
    }
}

async fn drive(router: Router, uri: &str) -> (StatusCode, Vec<u8>) {
    let resp = router
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    (status, bytes.to_vec())
}

async fn send(portal_enabled: bool, uri: &str) -> (StatusCode, Vec<u8>) {
    drive(app(&config(portal_enabled), AppState::without_ch()), uri).await
}

/// A route under the portal prefix that exists only for these tests.
///
/// The suite needs one, and the reason is worth stating: for as long as
/// [`CONFIG_PATH`] is the *only* registered route under the prefix — and the
/// gate deliberately skips it — every "closed" assertion against a path like
/// `/api-tokens/api/key` observes an unrouted 404, not a refusal. Those
/// assertions stay green with the gate deleted. Driving [`gate_portal`] over a
/// route that really is there is what makes the difference observable, and it
/// is why [`PortalGate::new`] is public.
const PROBE: &str = "/api-tokens/api/__probe";

fn gated_probe(portal_enabled: bool) -> Router {
    Router::new().route(PROBE, get(|| async { "probe" })).layer(
        axum::middleware::from_fn_with_state(PortalGate::new(portal_enabled), gate_portal),
    )
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

/// The same property, but with the in-app key gate **armed** — the
/// configuration `config.rs` documents as the expected end state and which
/// `compute-stack.ts` has simply not reached yet.
///
/// This is the case an unconditional prefix exemption in `auth::is_exempt`
/// got wrong: portal paths answered an empty `404` while every other unknown
/// path answered `401` with an `ErrorEnvelope`, making the portal the only
/// unauthenticated surface on the service and so uniquely fingerprintable —
/// precisely the disclosure the gate exists to prevent. The exemption is now
/// conditional on the portal being open.
#[tokio::test]
async fn a_closed_portal_stays_indistinguishable_when_api_keys_are_armed() {
    let armed = || {
        app(
            &config_with_keys(false, vec!["a-key".to_string()]),
            AppState::without_ch(),
        )
    };

    let (portal_status, portal_body) = drive(armed(), "/api-tokens/api/key").await;
    let (absent_status, absent_body) = drive(armed(), "/no-such-route-anywhere").await;

    assert_eq!(
        portal_status, absent_status,
        "with keys armed, a closed portal path must answer exactly as any other \
         unknown path does — otherwise the prefix advertises itself"
    );
    assert_eq!(portal_body, absent_body);
}

/// And with the portal **open**, its routes must be reachable without a key
/// even though the gate is armed — a visitor signing in has none by
/// definition. Distinguishable from an unknown path here, which is fine: an
/// open portal is public.
#[tokio::test]
async fn an_open_portal_is_anonymous_even_with_api_keys_armed() {
    let armed_open = app(
        &config_with_keys(true, vec!["a-key".to_string()]),
        AppState::without_ch(),
    );
    let (status, _) = drive(armed_open, CONFIG_PATH).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the portal's own endpoints cannot sit behind the partner API key"
    );
}

/// `/config` has to survive **both** gates in **all four** combinations of
/// (portal open/closed) × (keys armed/disarmed), and the closed+armed cell is
/// the one that matters: it is where production sits for the whole build.
///
/// Regression guard. Making the auth exemption conditional on the portal being
/// open — the fix for the fingerprinting issue — initially caught `/config` in
/// the same net, so an armed, closed service answered the bundle `401` instead
/// of `{"enabled": false}` and [[0185]]'s page could not tell it was closed.
#[tokio::test]
async fn config_answers_in_all_four_gate_combinations() {
    for portal_enabled in [false, true] {
        for keys in [vec![], vec!["a-key".to_string()]] {
            let armed = !keys.is_empty();
            let (status, body) = drive(
                app(
                    &config_with_keys(portal_enabled, keys),
                    AppState::without_ch(),
                ),
                CONFIG_PATH,
            )
            .await;
            assert_eq!(
                status,
                StatusCode::OK,
                "portal_enabled={portal_enabled}, api_keys_armed={armed}"
            );
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(json["enabled"], serde_json::Value::Bool(portal_enabled));
        }
    }
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

/// **The test that fails if the gate is deleted.** Everything else in this file
/// would survive `gate_portal` being replaced by `next.run(req).await`.
#[tokio::test]
async fn the_gate_refuses_a_route_that_really_exists() {
    let (closed, body) = drive(gated_probe(false), PROBE).await;
    assert_eq!(
        closed,
        StatusCode::NOT_FOUND,
        "a registered portal route must be refused while the portal is closed \
         — if this returns 200 the gate is not doing anything"
    );
    assert!(body.is_empty(), "a refusal must not carry a body");

    let (open, body) = drive(gated_probe(true), PROBE).await;
    assert_eq!(open, StatusCode::OK);
    assert_eq!(body, b"probe");
}

/// The refusal above must also be indistinguishable from an unrouted path
/// *within the same router*, not just in the assembled app.
#[tokio::test]
async fn the_gates_refusal_matches_an_unrouted_path_in_the_same_router() {
    let (refused, refused_body) = drive(gated_probe(false), PROBE).await;
    let (unrouted, unrouted_body) = drive(gated_probe(false), "/api-tokens/api/never-built").await;
    assert_eq!(refused, unrouted);
    assert_eq!(refused_body, unrouted_body);
}

/// `CONFIG_PATH` is the one portal path the gate waves through in both
/// directions. Pinned so a later slice does not "tidy" the exemption away and
/// leave [[0185]]'s bundle unable to ask whether the portal is open.
#[tokio::test]
async fn config_is_exempt_from_the_gate_in_both_directions() {
    for enabled in [false, true] {
        let (status, _) = send(enabled, CONFIG_PATH).await;
        assert_eq!(status, StatusCode::OK, "portal_enabled={enabled}");
    }
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
