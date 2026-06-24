//! XDR ledger decode + Soroban event walk.
//!
//! Wraps BE's `xdr_parser` crate: `decompress_zstd` → deserialize
//! `LedgerCloseMetaBatch` → per-ledger `extract_ledger` + per-tx
//! `extract_events`. The adapter converts xdr-parser's tagged JSON
//! representation of ScVal into the kernel's `TaggedValue` enum.
//!
//! Filter policy: only `EventSource::TxLevel` and `EventSource::PerOp`
//! (Protocol 23+) events are kept. Diagnostic events are dropped —
//! they can include byte-identical Contract-typed mirrors of consensus
//! events (BE task 0182), which would double-count.

use std::collections::HashMap;

use extractors_core::{SorobanEventRow, TaggedValue};
use stellar_xdr::curr::{LedgerCloseMeta, LedgerCloseMetaBatch, Limits, ReadXdr, TransactionMeta};
use xdr_parser::{decompress_zstd, extract_events, extract_ledger, types::EventSource};

use crate::reconcile::{DecodedLedger, LedgerDecoder};

pub struct XdrLedgerDecoder;

impl LedgerDecoder for XdrLedgerDecoder {
    async fn decode(&self, bytes: &[u8]) -> Result<Vec<DecodedLedger>, String> {
        let decompressed = decompress_zstd(bytes).map_err(|e| format!("decompress: {e:?}"))?;

        let batch = LedgerCloseMetaBatch::from_xdr(decompressed.as_slice(), Limits::none())
            .map_err(|e| format!("deserialize batch: {e}"))?;

        let mut out = Vec::with_capacity(batch.ledger_close_metas.len());
        for meta in batch.ledger_close_metas.iter() {
            let header = extract_ledger(meta);
            let event_groups = walk_ledger_events(meta, header.sequence, header.closed_at);
            out.push(DecodedLedger {
                ledger_sequence: header.sequence as u64,
                closed_at_unix_seconds: header.closed_at,
                event_groups,
            });
        }
        Ok(out)
    }
}

fn walk_ledger_events(
    meta: &LedgerCloseMeta,
    ledger_seq: u32,
    closed_at: i64,
) -> Vec<Vec<SorobanEventRow>> {
    let mut by_group: HashMap<(String, String), Vec<SorobanEventRow>> = HashMap::new();

    for (tx_hash, tx_meta) in iter_tx_metas(meta) {
        for evt in extract_events(tx_meta, &tx_hash, ledger_seq, closed_at) {
            if !matches!(evt.source, EventSource::TxLevel | EventSource::PerOp) {
                continue;
            }
            let Some(contract_id) = evt.contract_id.clone() else {
                continue;
            };
            let topics = match evt.topics {
                serde_json::Value::Array(arr) => arr.iter().map(json_to_tagged).collect(),
                _ => Vec::new(),
            };
            let row = SorobanEventRow {
                contract_id: contract_id.clone(),
                transaction_id: evt.transaction_hash.clone(),
                ledger_sequence: ledger_seq as u64,
                event_index: evt.event_index,
                topics,
                data: json_to_tagged(&evt.data),
            };
            by_group
                .entry((evt.transaction_hash, contract_id))
                .or_default()
                .push(row);
        }
    }

    // Stable order within each group: by event_index. Order across
    // groups is HashMap-iteration order, which is acceptable because
    // dispatch is per-group and the bucketer is commutative for
    // distinct (timestamp, asset_id, source) keys.
    let mut groups: Vec<Vec<SorobanEventRow>> = by_group.into_values().collect();
    for g in groups.iter_mut() {
        g.sort_by_key(|r| r.event_index);
    }
    groups
}

fn iter_tx_metas(meta: &LedgerCloseMeta) -> Vec<(String, &TransactionMeta)> {
    match meta {
        LedgerCloseMeta::V0(v) => v
            .tx_processing
            .iter()
            .map(|p| {
                (
                    hex::encode(p.result.transaction_hash.0),
                    &p.tx_apply_processing,
                )
            })
            .collect(),
        LedgerCloseMeta::V1(v) => v
            .tx_processing
            .iter()
            .map(|p| {
                (
                    hex::encode(p.result.transaction_hash.0),
                    &p.tx_apply_processing,
                )
            })
            .collect(),
        LedgerCloseMeta::V2(v) => v
            .tx_processing
            .iter()
            .map(|p| {
                (
                    hex::encode(p.result.transaction_hash.0),
                    &p.tx_apply_processing,
                )
            })
            .collect(),
    }
}

/// Convert one `{"type": "...", "value": ...}` tagged JSON node into a
/// `TaggedValue`. Types we don't yet handle (bool, u32, bytes, error, …)
/// collapse to `Null` — the kernel's Phoenix XYK extractor only inspects
/// sym/address/i128/vec/map shapes, so this is sufficient for the
/// extractors wired in by task 0037. Unsupported types become visible
/// to future extractors as `Null` and will need adapter extensions.
pub(crate) fn json_to_tagged(v: &serde_json::Value) -> TaggedValue {
    let Some(obj) = v.as_object() else {
        return TaggedValue::Null;
    };
    let type_name = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let value = obj.get("value").unwrap_or(&serde_json::Value::Null);
    match type_name {
        "sym" => value
            .as_str()
            .map(|s| TaggedValue::Symbol(s.to_string()))
            .unwrap_or(TaggedValue::Null),
        "string" => value
            .as_str()
            .map(|s| TaggedValue::String(s.to_string()))
            .unwrap_or(TaggedValue::Null),
        "address" => value
            .as_str()
            .map(|s| TaggedValue::Address(s.to_string()))
            .unwrap_or(TaggedValue::Null),
        "i128" => value
            .as_str()
            .and_then(|s| s.parse::<i128>().ok())
            .map(TaggedValue::I128)
            .unwrap_or(TaggedValue::Null),
        "u128" => value
            .as_str()
            .and_then(|s| s.parse::<u128>().ok())
            .and_then(|u| i128::try_from(u).ok())
            .map(TaggedValue::I128)
            .unwrap_or(TaggedValue::Null),
        "vec" => match value.as_array() {
            Some(arr) => TaggedValue::Vec(arr.iter().map(json_to_tagged).collect()),
            None => TaggedValue::Null,
        },
        "map" => match value.as_array() {
            Some(arr) => TaggedValue::Map(
                arr.iter()
                    .filter_map(|e| {
                        let k = e.get("key")?;
                        let v = e.get("value")?;
                        Some((json_to_tagged(k), json_to_tagged(v)))
                    })
                    .collect(),
            ),
            None => TaggedValue::Null,
        },
        _ => TaggedValue::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sym_address_i128_roundtrip() {
        assert_eq!(
            json_to_tagged(&json!({"type": "sym", "value": "swap"})),
            TaggedValue::Symbol("swap".into())
        );
        assert_eq!(
            json_to_tagged(&json!({"type": "address", "value": "CABCDEF"})),
            TaggedValue::Address("CABCDEF".into())
        );
        assert_eq!(
            json_to_tagged(&json!({"type": "i128", "value": "-12345"})),
            TaggedValue::I128(-12_345)
        );
        assert_eq!(
            json_to_tagged(&json!({"type": "u128", "value": "12345"})),
            TaggedValue::I128(12_345)
        );
    }

    #[test]
    fn vec_recursively_adapts() {
        let v = json!({
            "type": "vec",
            "value": [
                {"type": "sym", "value": "swap"},
                {"type": "address", "value": "CPOOL"},
                {"type": "address", "value": "CTRADER"},
            ],
        });
        match json_to_tagged(&v) {
            TaggedValue::Vec(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], TaggedValue::Symbol("swap".into()));
                assert_eq!(items[1], TaggedValue::Address("CPOOL".into()));
            }
            other => panic!("expected Vec, got {other:?}"),
        }
    }

    #[test]
    fn map_adapts_key_value_pairs() {
        let v = json!({
            "type": "map",
            "value": [
                {"key": {"type": "sym", "value": "token_in"},
                 "value": {"type": "address", "value": "CXLM"}},
                {"key": {"type": "sym", "value": "amount_in"},
                 "value": {"type": "i128", "value": "1000"}},
            ],
        });
        match json_to_tagged(&v) {
            TaggedValue::Map(pairs) => {
                assert_eq!(pairs.len(), 2);
                assert_eq!(pairs[0].0, TaggedValue::Symbol("token_in".into()));
                assert_eq!(pairs[0].1, TaggedValue::Address("CXLM".into()));
                assert_eq!(pairs[1].1, TaggedValue::I128(1000));
            }
            other => panic!("expected Map, got {other:?}"),
        }
    }

    #[test]
    fn nested_map_in_vec() {
        let v = json!({
            "type": "vec",
            "value": [{
                "type": "map",
                "value": [{
                    "key": {"type": "sym", "value": "k"},
                    "value": {"type": "i128", "value": "1"},
                }],
            }],
        });
        let out = json_to_tagged(&v);
        if let TaggedValue::Vec(items) = out {
            assert!(matches!(items[0], TaggedValue::Map(_)));
        } else {
            panic!("expected outer Vec");
        }
    }

    #[test]
    fn unsupported_type_falls_back_to_null() {
        assert_eq!(
            json_to_tagged(&json!({"type": "bool", "value": true})),
            TaggedValue::Null
        );
        assert_eq!(
            json_to_tagged(&json!({"type": "bytes", "value": "deadbeef"})),
            TaggedValue::Null
        );
    }

    #[test]
    fn malformed_i128_falls_back_to_null() {
        assert_eq!(
            json_to_tagged(&json!({"type": "i128", "value": "not a number"})),
            TaggedValue::Null
        );
    }

    #[test]
    fn missing_type_field_is_null() {
        assert_eq!(json_to_tagged(&json!({"value": "x"})), TaggedValue::Null);
        assert_eq!(json_to_tagged(&json!(null)), TaggedValue::Null);
        assert_eq!(json_to_tagged(&json!("bare-string")), TaggedValue::Null);
    }
}
