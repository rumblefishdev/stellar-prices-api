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
//! read from the live ingest cursor (`prices.ingest_cursor`), which advances
//! every batch, falling back to the SDEX `target_ledger` only while that cursor
//! is unset. ⚠️ An earlier revision of this note said there was "no live
//! chain-tip table in the prices schema" and derived the tip from
//! `target_ledger` alone; that column freezes when the backfill stops, and on
//! 2026-09-08 it was 534,222 ledgers — 28 days — behind reality (task 0176).
//!
//! `newest_data_available` — the other end of the covered window — exists in
//! `prices.backfill_progress` but is deliberately not surfaced here yet: it is
//! neither selected in `queries_ch` nor carried on these DTOs.

use serde::Serialize;
use utoipa::ToSchema;

// Every ledger-sequence field below publishes `maximum = 4_294_967_295`.
//
// These are `u64` in Rust (ClickHouse hands back `UInt64`), but a Stellar ledger
// sequence is `uint32` in the protocol's `LedgerHeader`, so `u32::MAX` is the
// real ceiling — a domain fact, not a limit we impose. Stated so a client knows
// the value always fits in 32 bits. It has to be a literal on each field because
// `#[schema(...)]` is an attribute macro and cannot read a const.
//
// A const here could only assert things about itself, not about those literals
// (an earlier revision had `assert!(LEDGER_SEQ_MAX == 4_294_967_295)`, which is
// a tautology that stays green while a mistyped attribute publishes a wrong
// bound). The literals are checked where they become observable instead:
// `every_ledger_field_publishes_the_uint32_ceiling` in `tests/openapi.rs` reads
// them back out of the served document and compares against `u32::MAX`.

/// `GET /backfill/status` response.
#[derive(Debug, Serialize, ToSchema)]
pub struct BackfillStatus {
    /// Current chain tip — the newest ledger the live processor has durably
    /// committed (`prices.ingest_cursor`), which advances every batch.
    ///
    /// Falls back to the SDEX `target_ledger` only when that cursor is unset.
    /// It was previously read from `target_ledger` alone, which is not a tip:
    /// the backfill rewrites it when it pushes, so it freezes when the backfill
    /// stops.
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
    /// One of `running`, `completed`, `paused`, `failed`, or **`stalled`**.
    ///
    /// `stalled` is derived at read time, not stored: a stream still recorded
    /// as `running` whose last push is more than 7 days old is republished as
    /// `stalled`. Nothing writes a terminal state when a backfill run dies, so
    /// without this a crashed run advertises `running` indefinitely. Treat
    /// `stalled` as "not making progress" — it is not a value the writer can
    /// ever produce.
    pub status: String,
    #[schema(maximum = 4_294_967_295u64)]
    pub current_ledger: u64,
    #[schema(maximum = 4_294_967_295u64)]
    pub start_ledger: u64,
    #[schema(maximum = 4_294_967_295u64)]
    pub target_ledger: u64,
    /// `(target - current) / (target - start) * 100`, computed at read time.
    /// Backward: this stream's `current_ledger` is the OLDEST ledger reflected
    /// and descends toward `start_ledger` (genesis), so the covered span is
    /// `[current, target]`. See `handlers::progress_pct`.
    pub progress_pct: f64,
    /// `current_ledger - start_ledger`, computed at read time — how far the
    /// archive's floor still sits above genesis. Backward, for the same reason
    /// as `progress_pct`. Bounded by the ledger-sequence ceiling, being a
    /// difference of two of them.
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
    /// One of `running`, `completed`, `paused`, `failed`, or **`stalled`**.
    ///
    /// `stalled` is derived at read time, not stored: a stream still recorded
    /// as `running` whose last push is more than 7 days old is republished as
    /// `stalled`. Nothing writes a terminal state when a backfill run dies, so
    /// without this a crashed run advertises `running` indefinitely. Treat
    /// `stalled` as "not making progress" — it is not a value the writer can
    /// ever produce.
    pub status: String,
    pub last_push_at: Option<String>,
    /// When the most recent run to reach a terminal state finished; `null`
    /// while no run has.
    ///
    /// ⚠️ This may legitimately predate `last_push_at`. The two measure
    /// different things: `completed_at` belongs to the run that completed,
    /// `last_push_at` to the last write by **any** run. A stream walked in more
    /// than one pass — the SDEX archive completed a pass on 2026-07-27 and
    /// pushed again on 2026-08-11 — therefore reports the earlier completion
    /// beside the later push, and that ordering is not a defect.
    pub completed_at: Option<String>,
    /// Timestamp of the oldest OHLCV row this stream has landed; see the module
    /// note. `null` until the stream lands its first candle.
    pub earliest_data_available: Option<String>,
}
