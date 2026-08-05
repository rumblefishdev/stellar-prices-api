//! Response DTOs for `/v1/backfill/status`.
//!
//! NOTE: `earliest_data_available` is the timestamp of the **oldest OHLCV row
//! this stream has actually landed** — the backfill writers merge it down from
//! each partition's minimum candle `minute_start` (`sdex-backfill`'s
//! `PartitionStats::earliest_minute` → `sink`'s monotonic `merge_min`), so it
//! tracks ingested depth and moves as the pre-Soroban tail walks backward. It
//! is NOT the floor of the public ledger archive. (An earlier revision of this
//! note said the opposite; the writer, `schema/init.sql`, and the schema
//! overview §3.5 all agree on the definition above.) `realtime_tip_ledger` is
//! derived from the SDEX stream's `target_ledger` (best available proxy; there
//! is no live chain-tip table in the prices schema).
//!
//! `newest_data_available` — the other end of the covered window — exists in
//! `prices.backfill_progress` but is deliberately not surfaced here yet: it is
//! neither selected in `queries_ch` nor carried on these DTOs.

use serde::Serialize;
use utoipa::ToSchema;

/// Upper bound published for every ledger-sequence field.
///
/// These are `u64` in Rust (ClickHouse hands back `UInt64`), but a Stellar
/// ledger sequence is `uint32` in the protocol's `LedgerHeader`, so `u32::MAX`
/// is the real ceiling — a domain fact, not a limit we impose. Stated so a
/// client knows the value always fits in 32 bits. Repeated as a literal on each
/// field because `#[schema(...)]` is an attribute macro and cannot read a const.
const LEDGER_SEQ_MAX: u64 = u32::MAX as u64;
const _: () = assert!(LEDGER_SEQ_MAX == 4_294_967_295);

/// `GET /backfill/status` response.
#[derive(Debug, Serialize, ToSchema)]
pub struct BackfillStatus {
    /// Approximate current chain tip (SDEX `target_ledger`).
    #[schema(maximum = 4_294_967_295u64)]
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
    #[schema(maximum = 4_294_967_295u64)]
    pub current_ledger: u64,
    #[schema(maximum = 4_294_967_295u64)]
    pub start_ledger: u64,
    #[schema(maximum = 4_294_967_295u64)]
    pub target_ledger: u64,
    /// `(current - start) / (target - start) * 100`, computed at read time.
    pub progress_pct: f64,
    /// `target_ledger - current_ledger`, computed at read time. Bounded by the
    /// same ledger-sequence ceiling, being a difference of two of them.
    #[schema(maximum = 4_294_967_295u64)]
    pub ledgers_remaining: u64,
    /// Most recent successful push (`null` until the first push).
    pub last_push_at: Option<String>,
    /// Timestamp of the oldest OHLCV row this stream has landed (see the module
    /// note). `null` until the stream lands its first candle.
    pub earliest_data_available: Option<String>,
}

/// Soroban AMM (one-shot) backfill progress.
#[derive(Debug, Serialize, ToSchema)]
pub struct AmmStream {
    pub status: String,
    pub last_push_at: Option<String>,
    pub completed_at: Option<String>,
    /// Timestamp of the oldest OHLCV row this stream has landed; see the module
    /// note. `null` until the stream lands its first candle.
    pub earliest_data_available: Option<String>,
}
