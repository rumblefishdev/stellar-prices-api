//! Axum handlers for `/v1/oracles`.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};

use crate::assets::queries_ch as assets_q;
use crate::common::errors::ErrorEnvelope;
use crate::common::extract::ValidatedPath;
use crate::common::{cache_control, errors};
use crate::identity::AssetIdentifier;
use crate::oracles::dto::OraclesResponse;
use crate::oracles::queries_ch;
use crate::state::AppState;

/// `GET /oracles/{asset_identifier}` — latest per-oracle readings for an asset.
///
/// 404s for an unknown asset; an asset that exists but has no oracle rows
/// returns 200 with an empty `oracles` array.
#[utoipa::path(
    get,
    path = "/oracles/{asset_identifier}",
    tag = "oracles",
    summary = "`GET /oracles/{asset_identifier}` — latest per-oracle readings for an asset.",
    description = "The most recent reading from each oracle that publishes a USD price for the asset, \
     one\nentry per oracle. An unknown asset is a 404; a known asset with no oracle \
     readings\nreturns 200 with an empty `oracles` array.",
    params(
        ("asset_identifier" = String, Path,
         description = "`native`, `CODE:ISSUER` (a classic asset's code and its issuer's `G…` public key) or \
          the `C…` address of a Soroban contract")
    ),
    responses(
        (status = 200, description = "Oracle readings", body = OraclesResponse),
        (status = 400, description = "Invalid asset identifier (`invalid_id`)", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid `x-api-key` (`unauthorized`)", body = ErrorEnvelope),
        (status = 403, description = "Rejected at the API gateway: `x-api-key` missing, unknown, or not enabled for this API"),
        (status = 404, description = "Unknown asset (`not_found`)", body = ErrorEnvelope),
        (status = 429, description = "Per-key rate limit or monthly quota exceeded"),
        (status = 500, description = "Database or upstream failure (`db_error`)", body = ErrorEnvelope),
    )
)]
pub async fn get_oracles(
    State(state): State<AppState>,
    ValidatedPath(raw): ValidatedPath<String>,
) -> Response {
    let id = match AssetIdentifier::parse(&raw) {
        Ok(id) => id,
        Err(e) => return errors::bad_request(errors::INVALID_ID, e.to_string()),
    };

    let asset_id = match assets_q::resolve_asset_id(state.ch(), &id).await {
        Ok(Some(aid)) => aid,
        Ok(None) => return errors::not_found("unknown asset"),
        Err(e) => return errors::db_error(&e, "asset lookup"),
    };

    match queries_ch::oracles_for_asset(state.ch(), asset_id).await {
        Ok(oracles) => {
            let body = OraclesResponse {
                asset: id.to_canonical(),
                oracles,
            };
            let mut resp = Json(body).into_response();
            cache_control::attach(&mut resp, cache_control::MEDIUM);
            resp
        }
        Err(e) => errors::db_error(&e, "oracle lookup"),
    }
}
