//! Axum handlers for the `/v1/assets` resource.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::assets::dto::{AssetDetail, AssetListItem, AssetListResponse, PriceResponse};
use crate::assets::queries_ch::{self, ListArgs, Order, SortCol, TypeFilter};
use crate::common::{cache_control, cursor, errors};
use crate::identity::AssetIdentifier;
use crate::state::AppState;

/// Default / maximum page size for `GET /assets` (overview §4.1).
const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

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

/// `GET /assets/{asset_identifier}` — single-asset metadata.
#[utoipa::path(
    get,
    path = "/assets/{asset_identifier}",
    tag = "assets",
    params(
        ("asset_identifier" = String, Path,
         description = "native, CODE:ISSUER, or a C… contract address")
    ),
    responses(
        (status = 200, description = "Asset detail", body = AssetDetail),
        (status = 400, description = "Invalid asset identifier"),
        (status = 404, description = "Unknown asset"),
    )
)]
pub async fn get_asset(State(state): State<AppState>, Path(raw): Path<String>) -> Response {
    let id = match AssetIdentifier::parse(&raw) {
        Ok(id) => id,
        Err(e) => return errors::bad_request(errors::INVALID_ID, e.to_string()),
    };

    match queries_ch::asset_detail(state.ch(), &id).await {
        Ok(Some(row)) => {
            let asset_kind = if !row.contract_address.is_empty() {
                "contract"
            } else if row.asset_code == "XLM" && row.issuer_address.is_empty() {
                "native"
            } else {
                "credit"
            };
            let body = AssetDetail {
                asset: id.to_canonical(),
                asset_kind: asset_kind.to_string(),
                code: row.asset_code,
                issuer: row.issuer_address,
                contract: row.contract_address,
                home_domain: row.home_domain,
                is_active: row.is_active != 0,
            };
            let mut resp = Json(body).into_response();
            cache_control::attach(&mut resp, cache_control::MEDIUM);
            resp
        }
        Ok(None) => errors::not_found("unknown asset"),
        Err(e) => {
            tracing::error!(error = %e, "asset_detail query failed");
            errors::internal_error(errors::DB_ERROR, "asset lookup failed")
        }
    }
}

/// Query parameters for `GET /assets`.
#[derive(Debug, Deserialize)]
pub struct ListParams {
    #[serde(rename = "type")]
    pub asset_type: Option<String>,
    pub search: Option<String>,
    pub sort: Option<String>,
    pub order: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

/// `GET /assets` — paginated, sortable, filterable list of tracked assets.
#[utoipa::path(
    get,
    path = "/assets",
    tag = "assets",
    params(
        ("type" = Option<String>, Query, description = "classic | soroban | all (default all)"),
        ("search" = Option<String>, Query, description = "asset code prefix match"),
        ("sort" = Option<String>, Query,
         description = "price | volume_24h | change_24h | code (default volume_24h)"),
        ("order" = Option<String>, Query, description = "asc | desc (default desc)"),
        ("cursor" = Option<String>, Query, description = "opaque pagination cursor"),
        ("limit" = Option<u32>, Query, description = "1..=200 (default 50)"),
    ),
    responses(
        (status = 200, description = "Asset list page", body = AssetListResponse),
        (status = 400, description = "Invalid query parameter"),
    )
)]
pub async fn get_assets(State(state): State<AppState>, Query(p): Query<ListParams>) -> Response {
    let Some(sort) = SortCol::parse(p.sort.as_deref()) else {
        return errors::bad_request(errors::INVALID_QUERY, "invalid sort");
    };
    let Some(order) = Order::parse(p.order.as_deref()) else {
        return errors::bad_request(errors::INVALID_QUERY, "invalid order");
    };
    let Some(type_filter) = TypeFilter::parse(p.asset_type.as_deref()) else {
        return errors::bad_request(errors::INVALID_QUERY, "invalid type");
    };
    let limit = p.limit.unwrap_or(DEFAULT_LIMIT);
    if limit == 0 || limit > MAX_LIMIT {
        return errors::bad_request(errors::INVALID_QUERY, "limit must be 1..=200");
    }
    let cursor = match p.cursor.as_deref() {
        Some(tok) => match cursor::decode(tok) {
            Some(c) => Some(c),
            None => return errors::bad_request(errors::INVALID_QUERY, "invalid cursor"),
        },
        None => None,
    };
    let search = p.search.filter(|s| !s.is_empty());

    let args = ListArgs {
        sort,
        order,
        type_filter,
        search,
        cursor,
        fetch_limit: limit as u64 + 1,
    };
    let mut rows = match queries_ch::list_assets(state.ch(), args).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "list_assets query failed");
            return errors::internal_error(errors::DB_ERROR, "asset list failed");
        }
    };

    let has_more = rows.len() as u64 > limit as u64;
    if has_more {
        rows.truncate(limit as usize);
    }
    let next_cursor = if has_more {
        rows.last().map(|r| cursor::encode(&r.sort_key, r.asset_id))
    } else {
        None
    };

    let data = rows
        .into_iter()
        .map(|r| AssetListItem {
            asset_type: if r.contract_address.is_empty() {
                "classic".to_string()
            } else {
                "soroban".to_string()
            },
            asset_code: r.asset_code,
            issuer_address: r.issuer_address,
            contract_address: r.contract_address,
            home_domain: r.home_domain,
            price_usd: r.price_usd,
            change_24h_pct: r.change_24h_pct,
            change_7d_pct: r.change_7d_pct,
            volume_24h_usd: r.volume_24h_usd,
            vwap_24h: r.vwap_24h,
            sources: json!({}),
            updated_at: r.updated_at,
        })
        .collect();

    let mut resp = Json(AssetListResponse {
        data,
        cursor: next_cursor,
        has_more,
    })
    .into_response();
    cache_control::attach(&mut resp, cache_control::MEDIUM);
    resp
}
