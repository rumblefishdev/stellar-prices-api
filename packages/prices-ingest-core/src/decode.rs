//! `*.xdr.zst` object bytes → `Vec<LedgerCloseMeta>`.
//!
//! Wraps BE's `xdr-parser` (`decompress_zstd` + `deserialize_batch`) — the same
//! two calls the SDEX backfill makes per ledger file (`sdex-backfill::ingest`).
//! A Galexie object is a zstd-compressed `LedgerCloseMetaBatch`; with
//! `ledgers_per_file = 1` the returned vec is usually a single ledger, but the
//! batch shape is honoured so a multi-ledger file decodes correctly too.

use stellar_xdr::LedgerCloseMeta;

use crate::error::IngestError;

/// Decompress + deserialize one Galexie `*.xdr.zst` object into its ledgers.
pub fn decode_object(compressed: &[u8]) -> Result<Vec<LedgerCloseMeta>, IngestError> {
    let xdr_bytes = xdr_parser::decompress_zstd(compressed)?;
    let batch = xdr_parser::deserialize_batch(&xdr_bytes)?;
    Ok(batch.ledger_close_metas.to_vec())
}

/// The ledger sequence number of a `LedgerCloseMeta` (all protocol versions).
/// The live Lambda uses this to advance its doorbell cursor to the highest
/// ledger actually processed in a run.
pub fn ledger_sequence(lcm: &LedgerCloseMeta) -> u32 {
    match lcm {
        LedgerCloseMeta::V0(v) => v.ledger_header.header.ledger_seq,
        LedgerCloseMeta::V1(v) => v.ledger_header.header.ledger_seq,
        LedgerCloseMeta::V2(v) => v.ledger_header.header.ledger_seq,
    }
}

/// The close time of a `LedgerCloseMeta`, unix seconds (all protocol versions).
///
/// This is the same field the trade extractor stamps onto every tick as
/// `closed_at` (`filter.rs::ledger_header`), so bucketing it with
/// `close_time / 60 * 60` yields exactly the `minute_start` the
/// [`crate::CandleAccumulator`] keys on. The live reconcile loop uses that to
/// tell a minute it has seen the END of from one still being filled — see
/// task 0282.
pub fn ledger_close_time(lcm: &LedgerCloseMeta) -> i64 {
    match lcm {
        LedgerCloseMeta::V0(v) => v.ledger_header.header.scp_value.close_time.0 as i64,
        LedgerCloseMeta::V1(v) => v.ledger_header.header.scp_value.close_time.0 as i64,
        LedgerCloseMeta::V2(v) => v.ledger_header.header.scp_value.close_time.0 as i64,
    }
}
