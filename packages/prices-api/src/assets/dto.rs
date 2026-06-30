//! Response DTOs for the `/v1/assets` resource.

use serde::Serialize;
use utoipa::ToSchema;

/// `GET /assets/{id}/price` response (overview §4.2). All numeric fields are
/// decimal strings to preserve precision.
#[derive(Debug, Serialize, ToSchema)]
pub struct PriceResponse {
    /// Echoed natural identity (`native`, `CODE:ISSUER`, or a C… contract).
    pub asset: String,
    /// Current USD price.
    pub price_usd: String,
    /// XLM-quoted price. **Stub `"0"`** until task 0072 materializes it in the MV.
    pub price_xlm: String,
    /// 24h USD volume-weighted average price.
    pub vwap_24h: String,
    /// Trailing-24h USD volume.
    pub volume_24h_usd: String,
    /// 24h percentage change. **Stub `"0"`** until task 0072.
    pub change_24h_pct: String,
    /// Per-source (DEX) price/volume breakdown. **Stub `{}`** until task 0072.
    #[schema(value_type = Object)]
    pub sources: serde_json::Value,
    /// Timestamp of the snapshot (ISO-8601 UTC).
    pub updated_at: String,
}

/// `GET /assets/{id}` response (overview §4.1). The doc fixes only the request
/// forms; this is the chosen detail shape, resolved from `prices.assets`.
#[derive(Debug, Serialize, ToSchema)]
pub struct AssetDetail {
    /// Echoed natural identity.
    pub asset: String,
    /// Normalized kind: `native`, `credit`, or `contract`.
    pub asset_kind: String,
    /// Classic asset code (`""` for native/contract).
    pub code: String,
    /// Classic issuer G-strkey (`""` otherwise).
    pub issuer: String,
    /// Soroban contract C-strkey (`""` otherwise).
    pub contract: String,
    /// SEP-1 home domain, if known.
    pub home_domain: String,
    /// Whether the asset is currently tracked as active.
    pub is_active: bool,
}

/// One item in the `GET /assets` listing (overview §4.1).
#[derive(Debug, Serialize, ToSchema)]
pub struct AssetListItem {
    pub asset_code: String,
    /// `classic` or `soroban` (matches the `?type` filter vocabulary).
    pub asset_type: String,
    pub issuer_address: String,
    pub contract_address: String,
    pub home_domain: String,
    pub price_usd: String,
    /// **Stub `"0"`** until task 0072.
    pub change_24h_pct: String,
    /// **Stub `"0"`** until task 0072.
    pub change_7d_pct: String,
    pub volume_24h_usd: String,
    pub vwap_24h: String,
    /// Per-source breakdown. **Stub `{}`** until task 0072.
    #[schema(value_type = Object)]
    pub sources: serde_json::Value,
    pub updated_at: String,
}

/// `GET /assets` paginated response.
#[derive(Debug, Serialize, ToSchema)]
pub struct AssetListResponse {
    pub data: Vec<AssetListItem>,
    /// Opaque cursor for the next page (`null` on the last page).
    pub cursor: Option<String>,
    pub has_more: bool,
}
