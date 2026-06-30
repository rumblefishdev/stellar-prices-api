//! Public Prices REST API — a single axum Lambda serving all route groups
//! (ADR 0008). Copied in skeleton from BE's `crates/api`, adapted to the
//! ClickHouse-only Prices data layer (`packages/prices-clickhouse`).
//!
//! The library exposes [`app`] (the fully-wired `axum::Router`) so the Lambda
//! entrypoint (`src/main.rs`) and integration tests build the *exact same*
//! router — the bin only adds the Lambda HTTP runtime adapter on top. This is
//! the BE pattern that keeps the deployed routes and the tested routes in
//! lockstep.

pub mod common;
pub mod config;
pub mod ops;
pub mod state;

use axum::Router;

pub use config::AppConfig;
pub use state::AppState;

/// Build the fully-wired application router from an already-constructed
/// [`AppState`].
///
/// Pure and side-effect free: the caller owns cold-start I/O (building the CH
/// client), so tests can inject a CH-less state via [`AppState::without_ch`] and
/// drive routes with `tower::ServiceExt::oneshot` — no network, no Lambda.
///
/// Route groups are mounted here. Phase 0 wires only `/health`; the versioned
/// `/v1` resource routers (assets, ohlcv/price, batch, oracles, backfill) land
/// in later phases, and the whole thing migrates to a utoipa `OpenApiRouter`
/// when OpenAPI is introduced (Phase 1).
pub fn app(state: AppState) -> Router {
    Router::new().merge(ops::router()).with_state(state)
}
