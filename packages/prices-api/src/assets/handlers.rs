//! Axum handlers for the `/v1/assets` resource.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::assets::dto::PriceResponse;
use crate::assets::queries_ch;
use crate::common::{cache_control, errors};
use crate::identity::AssetIdentifier;
use crate::state::AppState;

/// `GET /assets/{asset_identifier}/price` — current price for one asset.
///
/// Parsing/validation runs before any DB call, so a malformed identifier 400s
/// without touching ClickHouse. `price_xlm`, `change_24h_pct`, and `sources` are
/// v1 stubs (task 0072 fills them producer-side).
#[utoipa::path(
    get,
    path = "/assets/{asset_identifier}/price",
    tag = "prices",
    params(
        ("asset_identifier" = String, Path,
         description = "native, CODE:ISSUER, or a C… contract address")
    ),
    responses(
        (status = 200, description = "Current price", body = PriceResponse),
        (status = 400, description = "Invalid asset identifier"),
        (status = 404, description = "No current price for the asset"),
    )
)]
pub async fn get_price(State(state): State<AppState>, Path(raw): Path<String>) -> Response {
    let id = match AssetIdentifier::parse(&raw) {
        Ok(id) => id,
        Err(e) => return errors::bad_request(errors::INVALID_ID, e.to_string()),
    };

    match queries_ch::current_price(state.ch(), &id).await {
        Ok(Some(row)) => {
            let body = PriceResponse {
                asset: id.to_canonical(),
                price_usd: row.price_usd,
                price_xlm: "0".to_string(),
                vwap_24h: row.vwap_24h,
                volume_24h_usd: row.volume_24h_usd,
                change_24h_pct: "0".to_string(),
                sources: json!({}),
                updated_at: row.updated_at,
            };
            let mut resp = Json(body).into_response();
            cache_control::attach(&mut resp, cache_control::SHORT);
            resp
        }
        Ok(None) => errors::not_found("no current price for the asset"),
        Err(e) => {
            // Log the detail; never leak the raw CH error to the client.
            tracing::error!(error = %e, "current_price query failed");
            errors::internal_error(errors::DB_ERROR, "price lookup failed")
        }
    }
}
