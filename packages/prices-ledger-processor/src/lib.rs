//! Prices Ledger Processor — live ingestion of Stellar ledgers into
//! `prices.price_ohlcv_1m` (task 0038).
//!
//! Shape mirrors BE's production indexer: an SQS **doorbell** triggers a
//! **doorbell-cursor reconcile loop** ([`reconcile`]) that walks contiguous
//! ledgers from S3, decodes + extracts + buckets them, writes OHLCV candles to
//! the shared Hetzner ClickHouse over mTLS, and advances its cursor last.
//!
//! The decode → extract → canonicalise → bucket → write pipeline is **not**
//! reimplemented here: it is `prices_ingest_core`, the same tested code the SDEX
//! backfill uses, so live and backfill rows are identical (same surrogate
//! `asset_id`s, SAC collapse, orientation, `Decimal`/`version`). This crate owns
//! only the *transport* seams:
//! - [`object_fetcher`] — local-disk (fixtures/tests) vs S3 (`lambda` feature).
//! - [`cursor`] — the ledger-sequence checkpoint.
//! - [`sink`] — the ClickHouse writer (plaintext local vs `aws-mtls` remote).
//! - [`galexie_key`] / [`retry`] — key derivation, backoff. (Log redaction now
//!   lives at the error source in `prices_ingest_core::safe_log`.)

pub mod cursor;
pub mod galexie_key;
pub mod object_fetcher;
pub mod reconcile;
pub mod sink;
