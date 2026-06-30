//! `/v1/assets` resource. Phase 2 lands `GET /assets/{id}/price`; asset detail
//! (`GET /assets/{id}`), listing (`GET /assets`), and OHLCV
//! (`GET /assets/{id}/ohlcv`) follow in later phases.

pub mod dto;
pub mod handlers;
pub mod queries_ch;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Resource router (mounted under `/v1` by [`crate::openapi::register_routes`]).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::get_assets))
        .routes(routes!(handlers::get_price))
        .routes(routes!(handlers::get_asset))
}
