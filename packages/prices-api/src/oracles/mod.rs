//! `/v1/oracles` resource (overview §4.4). Oracle cross-reference prices,
//! exposed for reference only — they do NOT feed `price_usd` anywhere else.

pub mod dto;
pub mod handlers;
pub mod queries_ch;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Resource router (mounted under `/v1`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(handlers::get_oracles))
}
