//! `/v1/prices/batch` resource (overview §4.3). Current prices for many assets
//! in one POST. Uncached (per §2.1).

pub mod dto;
pub mod handlers;

use axum::extract::DefaultBodyLimit;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Resource router (mounted under `/v1`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::post_batch))
        .layer(DefaultBodyLimit::max(dto::MAX_BATCH_BODY_BYTES))
}
