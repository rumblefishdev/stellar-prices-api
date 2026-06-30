//! OpenAPI assembly via `utoipa` + `utoipa-axum`.
//!
//! [`register_routes`] is the single source of routes: the live app calls
//! `.with_state(state).split_for_parts()` to get `(Router, OpenApi)`, while
//! `bin/extract_openapi.rs` calls `.split_for_parts()` directly for the spec —
//! so the documented routes and the served routes can never drift. Each `/v1`
//! resource adds its `routes!(...)` here as it lands (Phases 2–3).

use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Top-level API metadata. Schema/path components are collected from the
/// `#[utoipa::path]` handlers wired in [`register_routes`].
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Stellar Prices API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Public read API for Stellar asset prices, OHLCV, oracle \
                       cross-reference, and backfill status."
    ),
    tags(
        (name = "ops", description = "Operational endpoints (health)")
    )
)]
pub struct ApiDoc;

/// Build the un-stated router carrying both the axum routes and their OpenAPI
/// paths. Resource routers nest under `/v1`; operational routes stay at the
/// root.
pub fn register_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi()).routes(routes!(crate::ops::health))
    // .nest("/v1", crate::assets::router())  // Phase 2/3
    // .nest("/v1", crate::oracles::router()) // Phase 2 … etc.
}
