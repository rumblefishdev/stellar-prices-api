//! Response DTOs for `/v1/backfill/status`.
//!
//! NOTE: `earliest_data_available` is the earliest ledger available *to
//! backfill* — the floor of the public ledger archive the backfill writers
//! record — NOT the earliest candle ingested. The SDEX pre-Soroban tail is
//! still backfilling toward it, so the ingested/queryable depth is shallower
//! (query the coarse `price_ohlcv_1d` table for that). `realtime_tip_ledger` is
//! derived from the SDEX stream's `target_ledger` (best available proxy; there
//! is no live chain-tip table in the prices schema).

use serde::Serialize;
use utoipa::ToSchema;

/// `GET /backfill/status` response.
#[derive(Debug, Serialize, ToSchema)]
pub struct BackfillStatus {
    /// Approximate current chain tip (SDEX `target_ledger`).
    pub realtime_tip_ledger: u64,
    /// SDEX archive stream (absent if its row is missing).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdex: Option<SdexStream>,
    /// Soroban AMM stream (absent if its row is missing).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soroban_amm: Option<AmmStream>,
}

/// SDEX archive backfill progress.
#[derive(Debug, Serialize, ToSchema)]
pub struct SdexStream {
    pub status: String,
    pub current_ledger: u64,
    pub start_ledger: u64,
    pub target_ledger: u64,
    /// `(current - start) / (target - start) * 100`, computed at read time.
    pub progress_pct: f64,
    /// `target_ledger - current_ledger`, computed at read time.
    pub ledgers_remaining: u64,
    /// Most recent successful push (`null` until the first push).
    pub last_push_at: Option<String>,
    /// Earliest ledger available *to backfill* (public-archive floor), NOT the
    /// earliest ingested candle (see the module note). `null` if unrecorded.
    pub earliest_data_available: Option<String>,
}

/// Soroban AMM (one-shot) backfill progress.
#[derive(Debug, Serialize, ToSchema)]
pub struct AmmStream {
    pub status: String,
    pub last_push_at: Option<String>,
    pub completed_at: Option<String>,
    /// Earliest ledger available *to backfill* (public-archive floor); see the
    /// module note. `null` if unrecorded.
    pub earliest_data_available: Option<String>,
}
