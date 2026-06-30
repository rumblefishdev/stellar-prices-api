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
    components(schemas(
        crate::assets::dto::PriceResponse,
        crate::assets::dto::AssetDetail,
        crate::assets::dto::AssetListItem,
        crate::assets::dto::AssetListResponse,
        crate::oracles::dto::OraclesResponse,
        crate::oracles::dto::OracleEntry,
        crate::backfill::dto::BackfillStatus,
        crate::backfill::dto::SdexStream,
        crate::backfill::dto::AmmStream,
        crate::batch::dto::BatchRequest,
        crate::batch::dto::BatchResponse,
    )),
    tags(
        (name = "ops", description = "Operational endpoints (health)"),
        (name = "assets", description = "Asset metadata"),
        (name = "prices", description = "Asset prices (current + batch)"),
        (name = "oracles", description = "Oracle cross-reference prices"),
        (name = "backfill", description = "Historical backfill progress")
    )
)]
pub struct ApiDoc;

/// Build the un-stated router carrying both the axum routes and their OpenAPI
/// paths. Resource routers nest under `/v1`; operational routes stay at the
/// root.
pub fn register_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(crate::ops::health))
        .nest("/v1", crate::assets::router())
        .nest("/v1", crate::oracles::router())
        .nest("/v1", crate::backfill::router())
        .nest("/v1", crate::batch::router())
}
