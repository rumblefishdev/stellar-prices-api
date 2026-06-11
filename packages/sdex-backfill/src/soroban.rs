//! Single-pass Soroban extraction from the same `LedgerCloseMeta` the SDEX path
//! already parses. Produces AMM OHLCV ticks (Phoenix/Soroswap/Aquarius) and
//! oracle price samples (REFLECTOR/REDSTONE) — no dependency on a pre-populated
//! `soroban_events` table.
//!
//! Venue classification uses an incremental registry built from factory events
//! seen IN the indexed window (Soroswap `new_pair`, Aquarius `add_pool`, Phoenix
//! `create`/`liquidity_pool`). Pools created before the window are not resolved
//! (documented coverage limitation for task 0060); Soroswap additionally needs
//! the pool→tokens map for any resolution. Oracle events need no registry.

use std::collections::HashMap;

use rust_decimal::Decimal;
use serde_json::Value;
use stellar_xdr::curr::{LedgerCloseMeta, TransactionMeta};
use tracing::warn;

use extractors_core::{SorobanEventRow, TaggedValue, Venue, VenueRegistry};
use ledger_processor::dispatch::dispatch;
use phoenix_extractor::PhoenixPoolRegistry;
use soroswap_extractor::SoroswapPoolRegistry;
use xdr_parser::extract_events;
use xdr_parser::types::EventSource;

use crate::canonical::{AssetIdentity, AssetRegistry, canonicalise};
use crate::sink::OracleSample;
use crate::tick::TradeTick;

/// Reflector publishes prices already scaled to 14 decimals — matches the
/// `Decimal(38,14)` `oracle_prices.price_usd` column directly.
const ORACLE_SCALE: u32 = 14;
/// AMM token amounts are treated as 7-decimal (Stellar SAC convention). Token
/// decimals vary; this is a documented sizing-measurement approximation.
const AMM_AMOUNT_SCALE: u32 = 7;

/// Per-run registries, grown incrementally from in-window factory events.
pub struct Registries {
    pub venue: VenueRegistry,
    pub phoenix: PhoenixPoolRegistry,
    pub soroswap: SoroswapPoolRegistry,
    /// Oracle asset key (symbol or contract address) → synthetic asset_id.
    /// Kept in a separate id space (>= 1_000_000) from the trade AssetRegistry;
    /// oracle assets are not written to `prices.assets`.
    oracle_ids: HashMap<String, u32>,
    oracle_next: u32,
}

impl Default for Registries {
    fn default() -> Self {
        Self {
            venue: VenueRegistry::new(),
            phoenix: PhoenixPoolRegistry::new(),
            soroswap: SoroswapPoolRegistry::new(),
            oracle_ids: HashMap::new(),
            oracle_next: 1_000_000,
        }
    }
}

impl Registries {
    pub fn new() -> Self {
        Self::default()
    }

    fn oracle_id(&mut self, key: &str) -> u32 {
        if let Some(&id) = self.oracle_ids.get(key) {
            return id;
        }
        let id = self.oracle_next;
        self.oracle_next += 1;
        self.oracle_ids.insert(key.to_string(), id);
        id
    }

    pub fn pool_count(&self) -> usize {
        self.soroswap.pool_count() + self.phoenix.pool_count()
    }
}

/// Output of processing one ledger's Soroban events.
#[derive(Default)]
pub struct LedgerSoroban {
    /// (source, tick) pairs; source ∈ {phoenix, soroswap, aquarius}.
    pub amm_ticks: Vec<(&'static str, TradeTick)>,
    pub oracle: Vec<OracleSample>,
}

fn collect_tx_metas(lcm: &LedgerCloseMeta) -> Vec<&TransactionMeta> {
    match lcm {
        LedgerCloseMeta::V0(v) => v
            .tx_processing
            .iter()
            .map(|p| &p.tx_apply_processing)
            .collect(),
        LedgerCloseMeta::V1(v) => v
            .tx_processing
            .iter()
            .map(|p| &p.tx_apply_processing)
            .collect(),
        LedgerCloseMeta::V2(v) => v
            .tx_processing
            .iter()
            .map(|p| &p.tx_apply_processing)
            .collect(),
    }
}

fn ledger_header(lcm: &LedgerCloseMeta) -> (u32, i64) {
    match lcm {
        LedgerCloseMeta::V0(v) => (
            v.ledger_header.header.ledger_seq,
            v.ledger_header.header.scp_value.close_time.0 as i64,
        ),
        LedgerCloseMeta::V1(v) => (
            v.ledger_header.header.ledger_seq,
            v.ledger_header.header.scp_value.close_time.0 as i64,
        ),
        LedgerCloseMeta::V2(v) => (
            v.ledger_header.header.ledger_seq,
            v.ledger_header.header.scp_value.close_time.0 as i64,
        ),
    }
}

/// Convert an xdr-parser typed-JSON SCVal into an `extractors_core::TaggedValue`.
fn json_to_tagged(v: &Value) -> TaggedValue {
    let Some(obj) = v.as_object() else {
        return TaggedValue::Null;
    };
    let ty = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let val = obj.get("value");
    match ty {
        "sym" => TaggedValue::Symbol(val.and_then(|v| v.as_str()).unwrap_or_default().to_string()),
        "string" | "str" => {
            TaggedValue::String(val.and_then(|v| v.as_str()).unwrap_or_default().to_string())
        }
        "address" => {
            TaggedValue::Address(val.and_then(|v| v.as_str()).unwrap_or_default().to_string())
        }
        "i128" | "u128" | "i64" | "u64" | "i32" | "u32" | "i256" | "u256" => {
            TaggedValue::I128(json_int(val))
        }
        "map" => {
            let entries = val.and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let pairs = entries
                .iter()
                .filter_map(|e| {
                    let o = e.as_object()?;
                    Some((
                        json_to_tagged(o.get("key")?),
                        json_to_tagged(o.get("value")?),
                    ))
                })
                .collect();
            TaggedValue::Map(pairs)
        }
        "vec" => {
            let items = val.and_then(|v| v.as_array()).cloned().unwrap_or_default();
            TaggedValue::Vec(items.iter().map(json_to_tagged).collect())
        }
        _ => TaggedValue::Null,
    }
}

/// Parse an SCVal numeric `value` (string or JSON number) into i128. Oversized
/// values (u256 hex, etc.) fall back to 0 — they are not used for AMM amounts.
fn json_int(val: Option<&Value>) -> i128 {
    match val {
        Some(Value::String(s)) => s.parse::<i128>().unwrap_or(0),
        Some(Value::Number(n)) => n.as_i64().map(i128::from).unwrap_or(0),
        _ => 0,
    }
}

/// First topic's symbol value, if any (the event "signature").
fn signature(topics: &Value) -> Option<&str> {
    topics
        .as_array()?
        .first()?
        .as_object()?
        .get("value")?
        .as_str()
}

/// Process all Soroban events in one ledger.
pub fn process_ledger(
    lcm: &LedgerCloseMeta,
    reg: &mut Registries,
    assets: &mut AssetRegistry,
) -> LedgerSoroban {
    let (ledger_seq, closed_at) = ledger_header(lcm);
    let tx_metas = collect_tx_metas(lcm);
    let mut out = LedgerSoroban::default();

    for (tx_index, tx_meta) in tx_metas.iter().enumerate() {
        let tx_id = format!("{ledger_seq}:{tx_index}");
        let events = extract_events(tx_meta, &tx_id, ledger_seq, closed_at);

        // Grouped AMM event rows by emitting contract, order preserved.
        let mut amm_groups: HashMap<String, Vec<SorobanEventRow>> = HashMap::new();

        for ev in &events {
            if ev.source == EventSource::Diagnostic {
                continue; // drop diagnostic mirrors
            }
            let Some(contract_id) = ev.contract_id.clone() else {
                continue;
            };
            let sig = signature(&ev.topics);

            match sig {
                Some("REFLECTOR") => decode_reflector(ev, reg, &mut out),
                Some("REDSTONE") => decode_redstone(ev, contract_id.as_str(), reg, &mut out),
                _ => {
                    // Factory events grow the registry; pool events are queued.
                    learn_factory(&contract_id, ev, reg);
                    let row = SorobanEventRow {
                        contract_id: contract_id.clone(),
                        transaction_id: tx_id.clone(),
                        ledger_sequence: ledger_seq as u64,
                        event_index: ev.event_index,
                        topics: topics_to_tagged(&ev.topics),
                        data: json_to_tagged(&ev.data),
                    };
                    amm_groups.entry(contract_id).or_default().push(row);
                }
            }
        }

        for (contract_id, rows) in amm_groups {
            let venue = match reg.venue.get(&contract_id) {
                Some(v) => v.clone(),
                None => continue, // unknown pool/contract — skip
            };
            let source = venue_source(&venue);
            match dispatch(&rows, &reg.venue, &reg.phoenix, &reg.soroswap) {
                Ok(trades) => {
                    for t in trades {
                        if let Some(tick) = amm_trade_to_tick(&t, closed_at, assets) {
                            out.amm_ticks.push((source, tick));
                        }
                    }
                }
                Err(e) => warn!(contract_id, error = %e, "amm dispatch error"),
            }
        }
    }

    out
}

fn topics_to_tagged(topics: &Value) -> Vec<TaggedValue> {
    topics
        .as_array()
        .map(|a| a.iter().map(json_to_tagged).collect())
        .unwrap_or_default()
}

fn venue_source(v: &Venue) -> &'static str {
    match v {
        Venue::Phoenix => "phoenix",
        Venue::Soroswap => "soroswap",
        Venue::Aquarius => "aquarius",
    }
}

/// Recognise factory events and register the created pool. Detected by event
/// signature (only factories emit these), independent of emitter address.
fn learn_factory(contract_id: &str, ev: &xdr_parser::types::ExtractedEvent, reg: &mut Registries) {
    let topics = ev.topics.as_array();
    let sig = signature(&ev.topics);
    match sig {
        // Aquarius router: Symbol("add_pool"), data (pool_address, pool_type)
        Some("add_pool") => {
            if let Some(pool) = first_address(&ev.data) {
                reg.venue.insert(pool, Venue::Aquarius);
            }
        }
        // Phoenix factory: [Symbol("create"), Symbol("liquidity_pool")], data Address(pool)
        Some("create") => {
            let is_lp = topics
                .and_then(|t| t.get(1))
                .and_then(|t| t.as_object())
                .and_then(|o| o.get("value"))
                .and_then(|v| v.as_str())
                == Some("liquidity_pool");
            if is_lp {
                if let Some(pool) = address_value(&ev.data) {
                    reg.venue.insert(pool.clone(), Venue::Phoenix);
                    reg.phoenix.register(pool, phoenix_extractor::POOL_TYPE_XYK);
                }
            }
        }
        // Soroswap factory: [String("SoroswapFactory"), Symbol("new_pair")],
        // data { token_0, token_1, pair, ... }
        Some("new_pair") => {
            if let TaggedValue::Map(m) = json_to_tagged(&ev.data) {
                let get = |k: &str| {
                    m.iter()
                        .find(|(key, _)| key.as_str() == Some(k))
                        .and_then(|(_, v)| v.as_address().map(String::from))
                };
                if let (Some(pair), Some(t0), Some(t1)) =
                    (get("pair"), get("token_0"), get("token_1"))
                {
                    reg.soroswap.register(pair.clone(), t0, t1);
                    reg.venue.insert(pair, Venue::Soroswap);
                }
            }
        }
        _ => {}
    }
    let _ = contract_id;
}

fn address_value(v: &Value) -> Option<String> {
    let o = v.as_object()?;
    if o.get("type")?.as_str()? == "address" {
        return o.get("value")?.as_str().map(String::from);
    }
    None
}

fn first_address(v: &Value) -> Option<String> {
    match json_to_tagged(v) {
        TaggedValue::Vec(items) => items.iter().find_map(|t| t.as_address().map(String::from)),
        TaggedValue::Address(a) => Some(a),
        _ => None,
    }
}

/// Convert a venue `TradeRow` into a `TradeTick` for the candle accumulator.
fn amm_trade_to_tick(
    trade: &extractors_core::TradeRow,
    closed_at: i64,
    assets: &mut AssetRegistry,
) -> Option<TradeTick> {
    let sold = AssetIdentity::Contract(trade.token_in.clone());
    let bought = AssetIdentity::Contract(trade.token_out.clone());
    let pair = canonicalise(&sold, &bought, assets);

    let amount_in = Decimal::try_from_i128_with_scale(trade.amount_in, AMM_AMOUNT_SCALE).ok()?;
    let amount_out = Decimal::try_from_i128_with_scale(trade.amount_out, AMM_AMOUNT_SCALE).ok()?;
    if amount_in.is_zero() || amount_out.is_zero() {
        return None;
    }

    let (price, volume_base, volume_quote) = if pair.inverted {
        (amount_in / amount_out, amount_out, amount_in)
    } else {
        (amount_out / amount_in, amount_in, amount_out)
    };

    Some(TradeTick {
        ledger_sequence: trade.ledger_sequence as u32,
        closed_at,
        operation_index: (trade.first_event_index & 0xFFFF) as u16,
        claim_index: 0,
        base_id: pair.base_id,
        quote_id: pair.quote_id,
        price,
        volume_base,
        volume_quote,
    })
}

/// Decode a REFLECTOR `update` event: data.update_data is a vec of
/// [asset_key, price_i128@1e14] pairs; topic[2] is the u64 ms timestamp.
fn decode_reflector(
    ev: &xdr_parser::types::ExtractedEvent,
    reg: &mut Registries,
    out: &mut LedgerSoroban,
) {
    let ts_ms = ev
        .topics
        .as_array()
        .and_then(|t| t.get(2))
        .and_then(|t| t.as_object())
        .and_then(|o| o.get("value"))
        .and_then(|v| v.as_u64())
        .unwrap_or((ev.created_at.max(0) as u64) * 1000);
    let timestamp = (ts_ms / 1000) as u32;

    let TaggedValue::Map(m) = json_to_tagged(&ev.data) else {
        return;
    };
    let Some((_, update_data)) = m.iter().find(|(k, _)| k.as_str() == Some("update_data")) else {
        return;
    };
    let TaggedValue::Vec(entries) = update_data else {
        return;
    };
    for entry in entries {
        if let TaggedValue::Vec(kv) = entry {
            if kv.len() >= 2 {
                let key = kv[0].as_str().map(String::from);
                let price = kv[1].as_i128();
                if let (Some(key), Some(price)) = (key, price) {
                    let asset_id = reg.oracle_id(&key);
                    out.oracle.push(OracleSample {
                        timestamp,
                        asset_id,
                        oracle_name: "reflector".to_string(),
                        price_usd: price, // already 1e14-scaled
                        raw_data: format!("{{\"asset\":\"{key}\"}}"),
                    });
                }
            }
        }
    }
    let _ = ORACLE_SCALE;
}

/// REDSTONE carries a base64 XDR `bytes` payload (updated_feeds map). Full XDR
/// decode is deferred; we capture one row per event with the raw payload so the
/// `oracle_prices` byte footprint is measured (price left 0).
fn decode_redstone(
    ev: &xdr_parser::types::ExtractedEvent,
    contract_id: &str,
    reg: &mut Registries,
    out: &mut LedgerSoroban,
) {
    let raw = ev
        .data
        .as_object()
        .and_then(|o| o.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let asset_id = reg.oracle_id(contract_id);
    out.oracle.push(OracleSample {
        timestamp: ev.created_at.max(0) as u32,
        asset_id,
        oracle_name: "redstone".to_string(),
        price_usd: 0,
        raw_data: raw,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_to_tagged_handles_map_and_i128() {
        let v = json!({"type":"map","value":[
            {"key":{"type":"sym","value":"amount0"},"value":{"type":"i128","value":"123"}}
        ]});
        let t = json_to_tagged(&v);
        match t {
            TaggedValue::Map(m) => {
                assert_eq!(m[0].0.as_str(), Some("amount0"));
                assert_eq!(m[0].1.as_i128(), Some(123));
            }
            _ => panic!("expected map"),
        }
    }

    #[test]
    fn signature_reads_topic0_symbol() {
        let topics = json!([{"type":"sym","value":"REFLECTOR"},{"type":"sym","value":"update"}]);
        assert_eq!(signature(&topics), Some("REFLECTOR"));
    }
}
