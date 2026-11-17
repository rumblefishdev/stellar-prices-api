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

#[cfg(test)]
mod tests {
    use stellar_xdr::{
        Hash, LedgerCloseMetaBatch, LedgerCloseMetaV2, LedgerCloseValueSignature, LedgerHeader,
        LedgerHeaderHistoryEntry, Limits, NodeId, PublicKey, Signature, StellarValue,
        StellarValueExt, StellarValueProposedMsValue, StellarValueSignedMsValue, TimePoint,
        TimePointMs, Uint256, WriteXdr,
    };

    use super::*;

    const SEQ: u32 = 70_000_000;
    const CLOSE_TIME: u64 = 1_793_000_000;

    fn signature() -> LedgerCloseValueSignature {
        LedgerCloseValueSignature {
            node_id: NodeId(PublicKey::PublicKeyTypeEd25519(Uint256([1; 32]))),
            signature: Signature(vec![2; 64].try_into().unwrap()),
        }
    }

    /// One ledger with `ext`, packed as Galexie writes it: a zstd
    /// `LedgerCloseMetaBatch`.
    fn object(ext: StellarValueExt) -> Vec<u8> {
        let lcm = LedgerCloseMeta::V2(LedgerCloseMetaV2 {
            ledger_header: LedgerHeaderHistoryEntry {
                header: LedgerHeader {
                    ledger_seq: SEQ,
                    scp_value: StellarValue {
                        close_time: TimePoint(CLOSE_TIME),
                        ext,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        });
        let batch = LedgerCloseMetaBatch {
            start_sequence: SEQ,
            end_sequence: SEQ,
            ledger_close_metas: vec![lcm].try_into().unwrap(),
        };
        let xdr = batch.to_xdr(Limits::none()).unwrap();
        zstd::encode_all(&xdr[..], 0).unwrap()
    }

    /// Protocol 30 (CAP-0088) puts one of the two millisecond arms on every
    /// ledger after the vote; `stellar-xdr` 28 rejects both. `close_time`
    /// stays whole seconds (`closeTime == closeTimeMs / 1000`).
    #[test]
    fn protocol_30_millisecond_arms_decode_with_close_time_in_seconds() {
        let close_time_ms = TimePointMs(CLOSE_TIME * 1000 + 250);
        let cases = [
            (
                "SignedMs",
                StellarValueExt::SignedMs(StellarValueSignedMsValue {
                    close_time_ms: close_time_ms.clone(),
                    lc_value_signature: signature(),
                }),
            ),
            (
                "EmptyTxSetMs",
                StellarValueExt::EmptyTxSetMs(StellarValueProposedMsValue {
                    close_time_ms,
                    tx_set_hash: Hash([3; 32]),
                    previous_ledger_hash: Hash([4; 32]),
                    previous_ledger_version: 29,
                    lc_value_signature: signature(),
                }),
            ),
        ];
        for (arm, ext) in cases {
            let lcms = decode_object(&object(ext)).unwrap_or_else(|e| panic!("{arm}: {e}"));
            assert_eq!(lcms.len(), 1, "{arm}");
            assert_eq!(ledger_sequence(&lcms[0]), SEQ, "{arm}");
            assert_eq!(ledger_close_time(&lcms[0]), CLOSE_TIME as i64, "{arm}");
        }
    }
}
