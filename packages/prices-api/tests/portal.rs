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
use prices_api::portal::{CONFIG_PATH, OPENAPI_PATH, PortalGate, gate_portal};
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
        // Discord endpoints are part of the config now, not read from the
        // process environment per router — see `AppConfig::portal_endpoints`.
        portal_endpoints: Default::default(),
        // Task 0187: the control-plane client for self-service keys. `None`
        // is what every non-portal test wants — with no client in the
        // config there is no code path here that can reach API Gateway.
        portal_keys: None,
        portal_eligibility: None,
        // Task 0188: what the dashboard states as the per-key rate limit.
        // `None` is the shape a deployment that was not told the limit has, and
        // the tests that care set it explicitly.
        portal_rate_limit: None,
        portal_web_origin: None,
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
/// `/api/key` observes an unrouted 404, not a refusal. Those
/// assertions stay green with the gate deleted. Driving [`gate_portal`] over a
/// route that really is there is what makes the difference observable, and it
/// is why [`PortalGate::new`] is public.
const PROBE: &str = "/api/__probe";

fn gated_probe(portal_enabled: bool) -> Router {
    Router::new().route(PROBE, get(|| async { "probe" })).layer(
        axum::middleware::from_fn_with_state(PortalGate::new(portal_enabled), gate_portal),
    )
}

#[tokio::test]
async fn portal_is_closed_by_default() {
    // The flag defaults to false in `AppConfig::from_env`, and this is the
    // behaviour that default buys.
    let (status, _) = send(false, "/api/auth/login").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// The property the gate exists for. A `403` would confirm the route is there;
/// a `503` would promise it is coming. Both tell a stranger we are building
/// something. An empty `404` tells them nothing at all — and the router has no
/// `fallback`, so that is exactly what a genuinely absent path returns.
#[tokio::test]
async fn a_closed_portal_route_is_byte_identical_to_an_absent_one() {
    let (closed_status, closed_body) = send(false, "/api/key").await;
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

    let (portal_status, portal_body) = drive(armed(), "/api/key").await;
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

/// The dashboard's rate-limit line comes from here, not from a literal in the
/// bundle (task 0188).
///
/// `pricingApiFreePlanRateLimit` is a per-env config value that
/// `api-gateway-stack.ts` hands to `addUsagePlan` and `compute-stack.ts` hands
/// to this Lambda as `PORTAL_RATE_LIMIT`. Raising it and deploying must change
/// what the page says, which is only true while the page reads it from here.
#[tokio::test]
async fn config_carries_the_rate_limit_the_gateway_enforces() {
    let mut settings = config(true);
    settings.portal_rate_limit = Some(5);

    let (status, body) = drive(app(&settings, AppState::without_ch()), CONFIG_PATH).await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["rate_limit_per_second"], serde_json::json!(5));
}

/// A deployment that was not told the limit says nothing about it rather than
/// defaulting to a number — a stale default IS the drift this field exists to
/// remove, one layer down.
#[tokio::test]
async fn config_omits_the_rate_limit_when_it_was_not_configured() {
    let (status, body) = send(true, CONFIG_PATH).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json.get("rate_limit_per_second").is_none(),
        "an unconfigured limit must be absent, not a guess: {json}"
    );
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
    let (unrouted, unrouted_body) = drive(gated_probe(false), "/api/never-built").await;
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

/// The bundle shares `/api/*` with the backend and is carved out to S3 at the
/// CDN, so its paths should never reach this Lambda. If one does, it is a plain
/// `404` in both states — the gate owning it changes nothing a caller can see.
#[tokio::test]
async fn a_bundle_path_that_reaches_the_lambda_is_a_plain_miss() {
    let (open, _) = send(true, "/api/dashboard").await;
    let (closed, _) = send(false, "/api/dashboard").await;
    assert_eq!(open, StatusCode::NOT_FOUND);
    assert_eq!(closed, StatusCode::NOT_FOUND);
}

/// The OpenAPI alias under the prefix answers in both states, like the root
/// copy and like `/config` — public documentation is not part of the portal
/// being open or closed (task 0194).
#[tokio::test]
async fn the_openapi_alias_answers_in_both_states() {
    for enabled in [false, true] {
        let (status, body) = send(enabled, "/api/api-docs-json").await;
        assert_eq!(status, StatusCode::OK, "portal_enabled={enabled}");
        let (_, root_body) = send(enabled, "/api-docs-json").await;
        assert_eq!(
            body, root_body,
            "the alias must serve the root document byte for byte"
        );
    }
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

// ---------------------------------------------------------------------------
// CORS — the bundle on a host of its own (task 0194)
// ---------------------------------------------------------------------------

const WEB_ORIGIN: &str = "https://sorobanscan.example";

fn config_with_origin(origin: Option<&str>) -> AppConfig {
    AppConfig {
        portal_web_origin: origin.map(str::to_string),
        ..config(true)
    }
}

async fn send_with(
    config: &AppConfig,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
) -> axum::http::Response<Body> {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app(config, AppState::without_ch())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// With the bundle's origin configured, a portal answer names it — exactly
/// it, with credentials — and any other origin gets no CORS header at all.
/// Reflecting the request's `Origin` would hand every site a readable,
/// cookie-bearing `GET /api/key`, which is why the second half is asserted
/// as hard as the first.
#[tokio::test]
async fn portal_answers_name_the_one_configured_origin_and_no_other() {
    let config = config_with_origin(Some(WEB_ORIGIN));

    let ours = send_with(&config, "GET", "/api/config", &[("origin", WEB_ORIGIN)]).await;
    assert_eq!(ours.status(), StatusCode::OK);
    assert_eq!(ours.headers()["access-control-allow-origin"], WEB_ORIGIN);
    assert_eq!(ours.headers()["access-control-allow-credentials"], "true");
    assert!(
        ours.headers()
            .get("vary")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.to_ascii_lowercase().contains("origin")),
        "a per-origin answer must vary on Origin"
    );

    for other in [
        "https://evil.example",
        "https://sorobanscan.example.evil",
        "http://sorobanscan.example",
    ] {
        let theirs = send_with(&config, "GET", "/api/config", &[("origin", other)]).await;
        assert_eq!(theirs.status(), StatusCode::OK, "{other}");
        assert!(
            theirs
                .headers()
                .get("access-control-allow-origin")
                .is_none(),
            "{other} must not be allowed"
        );
    }
}

/// The same-origin deployment — nothing configured — emits no CORS header
/// even to a request that names the origin production would allow.
#[tokio::test]
async fn without_a_configured_origin_no_answer_carries_a_cors_header() {
    let config = config_with_origin(None);
    let reply = send_with(&config, "GET", "/api/config", &[("origin", WEB_ORIGIN)]).await;
    assert_eq!(reply.status(), StatusCode::OK);
    assert!(reply.headers().get("access-control-allow-origin").is_none());
    assert!(
        reply
            .headers()
            .get("access-control-allow-credentials")
            .is_none()
    );
}

/// The revoke's preflight: `POST` with the marker header, from the bundle's
/// origin. Answered locally by the layer (in production by the gateway's
/// MOCK), and the answer has to allow the marker or the one write the portal
/// has can never be sent.
#[tokio::test]
async fn the_revokes_preflight_allows_the_marker_header_from_the_configured_origin() {
    use prices_api::portal::keys::{PORTAL_REQUEST_HEADER, REWORK_PATH};
    let config = config_with_origin(Some(WEB_ORIGIN));
    let reply = send_with(
        &config,
        "OPTIONS",
        REWORK_PATH,
        &[
            ("origin", WEB_ORIGIN),
            ("access-control-request-method", "POST"),
            ("access-control-request-headers", PORTAL_REQUEST_HEADER.0),
        ],
    )
    .await;
    assert!(reply.status().is_success(), "{}", reply.status());
    assert_eq!(reply.headers()["access-control-allow-origin"], WEB_ORIGIN);
    assert_eq!(reply.headers()["access-control-allow-credentials"], "true");
    let methods = reply.headers()["access-control-allow-methods"]
        .to_str()
        .unwrap()
        .to_ascii_uppercase();
    assert!(methods.contains("POST"), "{methods}");
    let allowed = reply.headers()["access-control-allow-headers"]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(allowed.contains(PORTAL_REQUEST_HEADER.0), "{allowed}");
    assert!(reply.headers().contains_key("access-control-max-age"));
}
/// The PORTAL's credentialed layer covers the portal's routes and nothing else.
///
/// ⚠️ **Rewritten twice in one day, by two tasks that changed its premise from
/// opposite sides — read both before amending it again.**
///
/// It was originally observed on the ALLOW-ORIGIN header: nothing outside the
/// portal carried one. That is no longer true of anything it checks.
/// [[0195]] gave both OpenAPI copies `Access-Control-Allow-Origin: *` so the
/// API reference page can fetch the document cross-origin, and [[0126]] gave
/// the data routes and `/health` the same `*` via `data_cors_layer`, which is
/// what makes `/v1` callable from a browser at all. **"No allow-origin" now
/// describes nothing this repo serves.**
///
/// What survives both, and is the whole point of running two CORS policies on
/// one service: the portal's SINGLE ORIGIN and its CREDENTIALS must never
/// appear anywhere else. `*` with credentials is refused by every browser, and
/// the portal's origin on a data route would hand one site a readable answer
/// while telling every other site it was not allowed. Those are the two
/// assertions below, and they hold no matter which way the wildcard spreads.
#[tokio::test]
async fn the_portals_credentialed_layer_stops_at_the_portal_prefix() {
    let config = config_with_origin(Some(WEB_ORIGIN));
    // `/v1/…/price` with a malformed identifier: a `400` from the handler's
    // own validation, before any database call, so the data API is covered
    // here without this suite needing ClickHouse (task 0195).
    for uri in [
        "/api-docs-json",
        OPENAPI_PATH,
        "/health",
        "/v1/assets/not-an-asset/price",
    ] {
        let reply = send_with(&config, "GET", uri, &[("origin", WEB_ORIGIN)]).await;
        let headers = reply.headers();
        assert!(
            headers.get("access-control-allow-credentials").is_none(),
            "{uri} is outside the portal's credentialed CORS answer"
        );
        let origin = headers
            .get("access-control-allow-origin")
            .map(|v| v.to_str().unwrap().to_string());
        assert_ne!(
            origin.as_deref(),
            Some(WEB_ORIGIN),
            "{uri} must not name the portal's origin"
        );
    }
}

/// The data routes' own answer: `*`, and NOTHING that would make a browser
/// refuse it (task 0126).
///
/// 🔑 **This is the half the gateway cannot provide.** `addCorsPreflight` in
/// `api-gateway-stack.ts` answers the preflight from a MOCK, but a preflight
/// only buys permission to SEND the request — the browser then reads the real
/// response's own allow-origin, and with a Lambda proxy integration that can
/// only come from this handler. Shipping the preflight alone leaves every
/// browser call failing exactly as before, while `curl` passes: that near-miss
/// is why this test exists.
///
/// ⚠️ `/health` is in here because it needs no ClickHouse — but it is a gateway
/// MOCK in production and never reaches this code there, so on its own it pins
/// the layer without pinning a route the layer actually has to serve. The `/v1`
/// path alongside it is the real subject: a malformed identifier is rejected by
/// the handler's own validation before any database call (task 0195 found this
/// route), and
/// [`a_real_v1_route_carries_the_wildcard_on_its_own_response`] covers the
/// `POST` side.
#[tokio::test]
async fn data_routes_answer_a_wildcard_origin_without_credentials() {
    let config = config_with_origin(Some(WEB_ORIGIN));
    // Two unrelated origins: a constant `*` must not vary with the caller, or
    // the gateway's stage cache would serve one caller's answer to the next
    // (the shape task 0118 measured on production).
    for origin in ["https://evil.example", "https://someone-else.example"] {
        for uri in ["/health", "/v1/assets/not-an-asset/price"] {
            let reply = send_with(&config, "GET", uri, &[("origin", origin)]).await;
            let headers = reply.headers();
            assert_eq!(
                headers
                    .get("access-control-allow-origin")
                    .map(|v| v.to_str().unwrap()),
                Some("*"),
                "{uri} answers every origin with the same wildcard"
            );
            assert!(
                headers.get("access-control-allow-credentials").is_none(),
                "`*` and credentials together are refused by every browser"
            );
        }
    }
}

/// The same wildcard on a **real `/v1` route**, proven to have been ROUTED
/// there (task 0126).
///
/// 🔑 **Why this exists next to the `/health` test above.** Every other CORS
/// assertion in this file rides on `/health`, which production answers from a
/// gateway MOCK — this handler never sees it. So `/health` can only show that
/// `data_cors_layer` is attached, not that it covers the surface the task is
/// about. Anything that leaves `/health` inside the layer while a data route
/// moves out of it — a router re-nested after the `.layer()` call in `app()`,
/// or simply a new `/v1` route registered below it — keeps the `/health` test
/// green while browser calls go back to being blocked and `curl` stays green
/// too: the exact half-shipped mechanism this task nearly released.
///
/// ⚠️ **A CORS header alone would NOT prove that.** The layer wraps the
/// fallback too, so an unrouted path answers `404` carrying the same `*`, and
/// a test that only read the header would pass against a route that no longer
/// exists. Hence the status and the error code: `{"assets": []}` is rejected by
/// `post_batch` itself, before `state.ch()` is reached, so a `400`
/// `invalid_query` is the handler's own signature — reachable with
/// [`AppState::without_ch`] and reachable no other way.
#[tokio::test]
async fn a_real_v1_route_carries_the_wildcard_on_its_own_response() {
    let config = config_with_origin(Some(WEB_ORIGIN));
    let reply = app(&config, AppState::without_ch())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/prices/batch")
                .header("origin", "https://evil.example")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"assets": []}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    let origin = reply
        .headers()
        .get("access-control-allow-origin")
        .map(|v| v.to_str().unwrap().to_string());
    let credentials = reply
        .headers()
        .get("access-control-allow-credentials")
        .is_some();
    let status = reply.status();
    let body = axum::body::to_bytes(reply.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let body = String::from_utf8_lossy(&body);

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the batch handler must have run and rejected the empty list; got {status} {body}"
    );
    assert!(
        body.contains("invalid_query"),
        "a 400 from anywhere else would not prove the route was reached: {body}"
    );
    assert_eq!(
        origin.as_deref(),
        Some("*"),
        "a /v1 response carries the wildcard the gateway preflight advertises"
    );
    assert!(
        !credentials,
        "`*` and credentials together are refused by every browser"
    );
}

/// Both copies of the OpenAPI document are readable from ANY origin (task
/// 0195): the portal's Swagger UI fetches the spec from the API's own hostname,
/// cross-origin, and so may a partner's tooling. `*` — constant, so the
/// gateway's stage cache cannot serve one caller's origin to the next — and
/// present whether or not the request carried an `Origin` at all, and whether
/// or not a portal origin is configured: it is a property of the document,
/// not of the portal.
#[tokio::test]
async fn the_openapi_document_is_readable_from_any_origin() {
    for config in [
        config_with_origin(Some(WEB_ORIGIN)),
        config_with_origin(None),
    ] {
        for uri in ["/api-docs-json", OPENAPI_PATH] {
            for headers in [&[][..], &[("origin", "https://partner.example")][..]] {
                let reply = send_with(&config, "GET", uri, headers).await;
                assert_eq!(reply.status(), StatusCode::OK, "{uri}");
                assert_eq!(
                    reply
                        .headers()
                        .get("access-control-allow-origin")
                        .map(|v| v.to_str().unwrap()),
                    Some("*"),
                    "{uri} with {headers:?}"
                );
            }
        }
    }
}

/// A closed portal answers a preflight like everything else under the prefix:
/// the gate's empty `404`, no CORS header. The browser then reports a CORS
/// failure, which is the honest reading of a door that is not open.
#[tokio::test]
async fn a_closed_portal_answers_a_preflight_with_the_gates_404() {
    use prices_api::portal::keys::REWORK_PATH;
    let config = AppConfig {
        portal_web_origin: Some(WEB_ORIGIN.to_string()),
        ..config(false)
    };
    let reply = send_with(
        &config,
        "OPTIONS",
        REWORK_PATH,
        &[
            ("origin", WEB_ORIGIN),
            ("access-control-request-method", "POST"),
        ],
    )
    .await;
    assert_eq!(reply.status(), StatusCode::NOT_FOUND);
    assert!(reply.headers().get("access-control-allow-origin").is_none());
}
