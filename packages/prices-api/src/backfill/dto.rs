//! Response DTOs for `/v1/backfill/status`.
//!
//! NOTE: `earliest_data_available` from overview §4.5 is intentionally omitted —
//! `prices.backfill_progress` has no such column yet (it's the backfill writers'
//! to record). `realtime_tip_ledger` is derived from the SDEX stream's
//! `target_ledger` (best available proxy; there is no live chain-tip table in
//! the prices schema).

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
    /// `(target - current) / (target - start) * 100`, computed at read time.
    pub progress_pct: f64,
    /// `current_ledger - start_ledger`, computed at read time.
    pub ledgers_remaining: u64,
    /// Most recent successful push (`null` until the first push).
    pub last_push_at: Option<String>,
}

/// Soroban AMM (one-shot) backfill progress.
#[derive(Debug, Serialize, ToSchema)]
pub struct AmmStream {
    pub status: String,
    pub last_push_at: Option<String>,
    pub completed_at: Option<String>,
}
