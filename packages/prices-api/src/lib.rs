//! Public Prices REST API — a single axum Lambda serving all route groups
//! (ADR 0008). Copied in skeleton from BE's `crates/api`, adapted to the
//! ClickHouse-only Prices data layer (`packages/prices-clickhouse`).
//!
//! The library exposes [`app`] (the fully-wired `axum::Router`) so the Lambda
//! entrypoint (`src/main.rs`) and integration tests build the *exact same*
//! router — the bin only adds the Lambda HTTP runtime adapter on top. This is
//! the BE pattern that keeps the deployed routes and the tested routes in
//! lockstep.

pub mod assets;
pub mod auth;
pub mod backfill;
pub mod batch;
pub mod common;
pub mod config;
pub mod identity;
pub mod openapi;
pub mod ops;
pub mod oracles;
pub mod portal;
pub mod state;
pub mod telemetry;

use std::sync::Arc;

use axum::Router;
use axum::http::header::{ACCEPT, ACCESS_CONTROL_ALLOW_ORIGIN, CONTENT_TYPE};
use axum::http::{HeaderName, HeaderValue, Method};
use axum::response::IntoResponse;
use axum::routing::get;
use tower_http::cors::{AllowOrigin, CorsLayer};

pub use config::AppConfig;
pub use state::AppState;

/// Build the fully-wired application router.
///
/// Pure apart from serializing the OpenAPI spec once at startup: the caller owns
/// cold-start I/O (building the CH client), so tests inject a CH-less state via
/// [`AppState::without_ch`] and drive routes with `tower::ServiceExt::oneshot` —
/// no network, no Lambda.
///
/// Wiring order:
/// 1. [`openapi::register_routes`] → `(Router, OpenApi)` (routes + their spec).
/// 2. Stamp `servers` from `config.base_url`, expose the spec at
///    `GET /api-docs-json`.
/// 3. Layer the in-app API-key gate (armed only when `API_KEYS` is set).
pub fn app(config: &AppConfig, state: AppState) -> Router {
    let (router, mut spec) = openapi::register_routes()
        .with_state(state)
        .split_for_parts();

    openapi::stamp_servers(&mut spec, config);

    // CORS for the data routes (task 0126). The gateway answers the PREFLIGHT
    // from a MOCK and never invokes this Lambda for it — but a preflight only
    // buys permission to send the real request, and the browser then checks the
    // REAL response for its own `Access-Control-Allow-Origin`. With a Lambda
    // PROXY integration that header can only come from here: API Gateway
    // returns what this handler returns, verbatim.
    //
    // ⚠️ **Shipping the gateway preflight without this layer changes nothing a
    // user can see.** The preflight answers `204`, the browser sends the `GET`,
    // the response arrives with no allow-origin, and the browser blocks it with
    // the same opaque `TypeError: Failed to fetch` as before — while `curl`
    // keeps working perfectly. That is the exact shape of defect this task
    // exists to close, and it was nearly re-shipped inside the fix for it.
    // Caught in review of PR #277; the two halves are declared together here
    // and in `api-gateway-stack.ts` so neither can travel alone.
    let router = router.layer(data_cors_layer());

    // Serialize once at startup; serve the cached string (no per-request work).
    // `DEPLOY_STATIC` is the client-facing tier for this route — the document
    // only changes when a new build ships (task 0124).
    //
    // Panics rather than falling back to `{}`. The fallback was silent in the
    // worst possible way: a syntactically valid, empty document served as
    // `200 OK`, with no log and no metric, then cached for 300s by the client
    // and 3600s by the gateway. Every partner running a generator against it in
    // that window would get a client with zero endpoints and no indication
    // anything was wrong. Failing to start is louder and shorter: the deploy
    // fails and the alarm fires. Matches `extract_openapi`, which already
    // `.expect`s the same serialization.
    let spec_json = Arc::new(
        spec.to_json()
            .expect("the OpenAPI document must serialize; it is built at startup"),
    );
    // Mounted twice: at the root, where partners and the OpenAPI `servers`
    // block expect it, and under the portal prefix (`portal::OPENAPI_PATH`),
    // which is the only place the portal's own "API reference" link can reach
    // on a host whose root belongs to another application (task 0194).
    //
    // `Access-Control-Allow-Origin: *`, on both copies (task 0195). The
    // portal's Swagger UI is a page on `sorobanscan.rumblefish.dev` fetching
    // this document from `prices-api.sorobanscan.rumblefish.dev` — cross-origin,
    // and read by `fetch`, not by navigation, so without an allow header the
    // browser refuses to hand the bytes to the page. `*` rather than the
    // portal's origin, deliberately: the document is anonymous, public and
    // carries no cookie and no key, so there is nothing an origin restriction
    // would protect and everything it would cost — a partner's Postman, a
    // client generator run from a browser, somebody else's Swagger. And the
    // value is a CONSTANT, which is what makes it safe under the gateway's
    // 3600 s stage cache: a reflected origin cached there would be served to
    // the next caller. The portal's own routes keep their single credentialed
    // origin (`portal::cors_layer`); this header never says `*` on anything
    // that reads a cookie.
    let serve_spec = move || {
        let spec_json = spec_json.clone();
        async move {
            let mut resp = (
                [
                    (CONTENT_TYPE, HeaderValue::from_static("application/json")),
                    (ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*")),
                ],
                (*spec_json).clone(),
            )
                .into_response();
            common::cache_control::attach(&mut resp, common::cache_control::DEPLOY_STATIC);
            resp
        }
    };
    let router = router
        .route("/api-docs-json", get(serve_spec.clone()))
        .route(portal::OPENAPI_PATH, get(serve_spec));

    // Portal routes before the key gate, and exempt from it: a visitor signing
    // in has no API key by definition (task 0183). The gate inside `portal`
    // decides whether they are served at all.
    let router = portal::apply(router, config);

    auth::apply(router, config)
}

/// The `/v1` data routes' CORS answer — `*`, no credentials (task 0126).
///
/// Deliberately NOT `portal::cors_layer`, and the difference is forced rather
/// than chosen: `Access-Control-Allow-Origin: *` cannot be combined with
/// credentials — browsers reject that pairing outright — so the portal, whose
/// calls carry the session cookie, must name exactly one origin. These routes
/// carry no cookie and no session (auth is an `x-api-key` header the caller
/// supplies deliberately), so `*` is available and costs nothing: a hostile
/// page calling `/v1` gets exactly what `curl` gets.
///
/// 🔑 **The value is a CONSTANT, and that is load-bearing under the gateway's
/// stage cache.** A reflected `Origin` cached there would be served to the next
/// caller as if their origin had been granted — the same cross-caller bleed
/// task 0118 measured on production with an undeclared cache-key parameter.
/// `AllowOrigin::any()` writes one fixed `*` for every caller, so there is
/// nothing per-caller to cache wrongly.
///
/// Mirrors the gateway's preflight answer in
/// `infra/src/lib/stacks/api-gateway-stack.ts` (`DATA_CORS_ALLOW_HEADERS`,
/// `DATA_PREFLIGHT_MAX_AGE`). The two must agree: the gateway states what a
/// browser MAY send, this states what it may READ, and a mismatch fails in the
/// browser only — never in `curl`, and never in a test that checks one side.
///
/// ⚠️ **What this does NOT cover.** It wraps the data router, so it is inside
/// two things that can reject a request before it: API Gateway's own error
/// responses (task 0255 — those still carry the PORTAL's single origin on
/// `/v1`, which a browser reads as a mismatch), and the in-app key gate, which
/// `auth::apply` layers outside every route. The gate is armed only when
/// `API_KEYS` is set and the gateway rejects a keyless caller first, so its
/// `401` is reachable only by a key the gateway accepted and this service does
/// not know — but that `401` reaches a browser without an allow-origin header
/// and reads as a dead network. Both are error paths; neither blocks a working
/// call. Tracked with 0255 rather than fixed here, because moving this layer
/// outside `auth::apply` would put it over the PORTAL's routes too.
///
/// ⚠️ **An earlier draft of this comment said that would emit TWO allow-origin
/// headers. It does not — measured 2026-09-02.** `CorsLayer` OVERWRITES the
/// header rather than appending, so an overlap is invisible in the response:
/// exactly one value comes back either way, and no test that reads the header
/// with `HeaderMap::get` could tell. The real consequence is worse for being
/// quiet — the portal's single credentialed origin is silently REPLACED by `*`,
/// and its `Access-Control-Allow-Credentials` disappears with it, which breaks
/// every cookie-bearing portal call. Four tests in `tests/portal.rs` do catch
/// that, so the constraint is pinned; it is the reasoning that was wrong.
///
/// The same measurement retires a worry recorded on PR #277: the OpenAPI
/// routes carry their own `*` from task 0195's handler, and this layer is
/// declared BEFORE they are registered — but even if it were not, the two
/// would not stack.
fn data_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::any())
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([CONTENT_TYPE, ACCEPT, HeaderName::from_static("x-api-key")])
        .max_age(std::time::Duration::from_secs(3600))
}
