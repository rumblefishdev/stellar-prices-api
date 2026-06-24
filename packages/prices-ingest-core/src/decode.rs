//! `*.xdr.zst` object bytes → `Vec<LedgerCloseMeta>`.
//!
//! Wraps BE's `xdr-parser` (`decompress_zstd` + `deserialize_batch`) — the same
//! two calls the SDEX backfill makes per ledger file (`sdex-backfill::ingest`).
//! A Galexie object is a zstd-compressed `LedgerCloseMetaBatch`; with
//! `ledgers_per_file = 1` the returned vec is usually a single ledger, but the
//! batch shape is honoured so a multi-ledger file decodes correctly too.

use stellar_xdr::curr::LedgerCloseMeta;

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
