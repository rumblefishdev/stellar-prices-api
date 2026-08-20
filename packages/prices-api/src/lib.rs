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
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::get;

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
    let router = router.route(
        "/api-docs-json",
        get(move || {
            let spec_json = spec_json.clone();
            async move {
                let mut resp =
                    ([(CONTENT_TYPE, "application/json")], (*spec_json).clone()).into_response();
                common::cache_control::attach(&mut resp, common::cache_control::DEPLOY_STATIC);
                resp
            }
        }),
    );

    // Portal routes before the key gate, and exempt from it: a visitor signing
    // in has no API key by definition (task 0183). The gate inside `portal`
    // decides whether they are served at all.
    let router = portal::apply(router, config);

    auth::apply(router, config)
}
