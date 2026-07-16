//! prices ingestion core — the shared ledger→OHLCV pipeline.
//!
//! This crate owns the *tested* decode → extract → canonicalise → bucket →
//! write pipeline that was first written for the SDEX historical backfill
//! (`sdex-backfill`) and is now reused verbatim by the live **Prices Ledger
//! Processor Lambda** (`prices-ledger-processor`, task 0038). Both writers go
//! through the same modules so live and backfill produce **identical**
//! `prices.price_ohlcv_1m` rows (same surrogate `asset_id`s via the
//! [`AssetRegistry`], same SAC→classic collapse, same preferred-quote
//! orientation, same `Decimal(38,14)` scaling, same `version`). Splitting this
//! into its own crate is what prevents the two paths from drifting.
//!
//! Layers, in pipeline order:
//! - [`filter`] — classic SDEX trades from `LedgerCloseMeta` operation results.
//! - [`soroban`] — Soroban AMM trades + oracle samples from contract events.
//! - [`canonical`] — asset identity, the [`AssetRegistry`] surrogate-id store,
//!   and `(base, quote)` canonicalisation.
//! - [`price`] / [`tick`] — per-trade price + the [`TradeTick`] the bucketer eats.
//! - [`bucket`] — 1-minute OHLCV accumulation ([`CandleAccumulator`]).
//! - [`writer`] — the transport-agnostic ClickHouse [`OhlcvWriter`] (works with a
//!   plaintext local client *or* the task-0052 mTLS client — both are a
//!   `clickhouse::Client`).
//! - [`decode`] — `*.xdr.zst` object bytes → `Vec<LedgerCloseMeta>`.

pub mod bucket;
pub mod canonical;
pub mod decode;
pub mod error;
pub mod filter;
pub mod price;
pub mod registry_io;
pub mod retry;
pub mod safe_log;
pub mod soroban;
pub mod tick;
pub mod writer;

pub use bucket::{CandleAccumulator, OhlcvCandle};
pub use canonical::{AssetIdentity, AssetRegistry, CanonicalPair, canonicalise};
pub use decode::{decode_object, ledger_sequence};
pub use error::IngestError;
pub use filter::{RawTrade, extract_trades};
pub use price::{compute_price, stroops_to_decimal};
pub use registry_io::PoolRegistryRow;
pub use retry::{DEFAULT_BACKOFF_MS, retry_with_backoff};
pub use safe_log::safe_response_token;
pub use soroban::{
    LedgerSoroban, RawSorobanEvent, Registries, UnresolvedPoolSwap, process_ledger,
    process_soroban_event_rows, reflector_key_to_identity,
};
pub use tick::{TradeTick, raw_trade_to_tick};
pub use writer::{AssetMetadata, OhlcvWriter, OracleSample, UnresolvedPool};
