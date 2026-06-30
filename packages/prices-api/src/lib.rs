//! Public Prices REST API — a single axum Lambda serving all route groups
//! (ADR 0008). Copied in skeleton from BE's `crates/api`, adapted to the
//! ClickHouse-only Prices data layer (`packages/prices-clickhouse`).
//!
//! The library exposes [`app`] (the fully-wired `axum::Router`) so the Lambda
//! entrypoint (`src/main.rs`) and integration tests build the *exact same*
//! router — the bin only adds the Lambda HTTP runtime adapter on top. This is
//! the BE pattern that keeps the deployed routes and the tested routes in
//! lockstep.

pub mod auth;
pub mod cache;
pub mod common;
pub mod config;
pub mod identity;
pub mod openapi;
pub mod ops;
pub mod state;

use std::sync::Arc;

use axum::Router;
use axum::http::header::CONTENT_TYPE;
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

    if let Some(base) = &config.base_url {
        spec.servers = Some(vec![utoipa::openapi::Server::new(base)]);
    }

    // Serialize once at startup; serve the cached string (no per-request work).
    let spec_json = Arc::new(spec.to_json().unwrap_or_else(|_| "{}".to_string()));
    let router = router.route(
        "/api-docs-json",
        get(move || {
            let spec_json = spec_json.clone();
            async move { ([(CONTENT_TYPE, "application/json")], (*spec_json).clone()) }
        }),
    );

    auth::apply(router, config)
}
