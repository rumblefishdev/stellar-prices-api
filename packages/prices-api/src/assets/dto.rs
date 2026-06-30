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
