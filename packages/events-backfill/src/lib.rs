//! Events-sourced AMM backfill (task 0097) — reprice historical AMM candles from
//! BE's ClickHouse `default.soroban_events` through the shared live extraction
//! pipeline. A CH-to-CH reprice: no ledger archive is re-downloaded.
//!
//! The extraction itself is NOT reimplemented here — [`crate::run`] feeds
//! CH-sourced events into `prices_ingest_core::process_soroban_event_rows`, the
//! same `classify_amm_groups` → `dispatch` → `amm_trade_to_tick` chain the live
//! processor uses, so repriced candles are byte-identical to the live path.

pub mod cli;
pub mod error;
pub mod run;
pub mod source;
