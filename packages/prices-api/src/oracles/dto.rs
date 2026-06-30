//! Response DTOs for the `/v1/oracles` resource.

use serde::Serialize;
use utoipa::ToSchema;

/// One oracle's latest reading for the asset. Doubles as the CH row
/// (`clickhouse::Row` + `Deserialize`) and the JSON shape.
#[derive(Debug, Serialize, serde::Deserialize, clickhouse::Row, ToSchema)]
pub struct OracleEntry {
    /// Oracle name, e.g. `reflector`.
    pub name: String,
    /// Latest oracle USD price (decimal string).
    pub price_usd: String,
    /// Timestamp of that reading (ISO-8601 UTC).
    pub updated_at: String,
}

/// `GET /oracles/{id}` response.
#[derive(Debug, Serialize, ToSchema)]
pub struct OraclesResponse {
    /// Echoed natural identity.
    pub asset: String,
    /// Latest reading per oracle (empty when none recorded).
    pub oracles: Vec<OracleEntry>,
}
