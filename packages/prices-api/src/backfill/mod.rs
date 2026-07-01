//! `/v1/backfill` resource (overview §4.5). Reflects the most recent push state
//! of the two canonical backfill streams (`sdex_archive`, `soroban_amm`).

pub mod dto;
pub mod handlers;
pub mod queries_ch;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Resource router (mounted under `/v1`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(handlers::get_status))
}
