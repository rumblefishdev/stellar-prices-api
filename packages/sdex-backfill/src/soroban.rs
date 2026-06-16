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

use crate::canonical::{AssetIdentity, AssetRegistry, USDC_ISSUER, USDT_ISSUER, canonicalise};
use crate::sink::OracleSample;
use crate::tick::TradeTick;

/// AMM token amounts are treated as 7-decimal (Stellar SAC convention). Token
/// decimals vary; this is a documented sizing-measurement approximation.
const AMM_AMOUNT_SCALE: u32 = 7;

/// Per-run registries, grown incrementally from in-window factory events.
///
/// Oracle assets are NOT kept here: they are resolved through the same
/// `AssetRegistry` as trades (task 0061 §5) so a Reflector `USDC`/`XLM` row
/// carries the identical canonical `asset_id` used as a candle's
/// `quote_asset_id`. The previous synthetic `>= 1_000_000` oracle id space made
/// the enrichment ASOF join (`o.asset_id = p.quote_asset_id`) match nothing for
/// backfilled data.
pub struct Registries {
    pub venue: VenueRegistry,
    pub phoenix: PhoenixPoolRegistry,
    pub soroswap: SoroswapPoolRegistry,
}

impl Default for Registries {
    fn default() -> Self {
        Self {
            venue: VenueRegistry::new(),
            phoenix: PhoenixPoolRegistry::new(),
            soroswap: SoroswapPoolRegistry::new(),
        }
    }
}

impl Registries {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pool_count(&self) -> usize {
        self.soroswap.pool_count() + self.phoenix.pool_count()
    }
}

/// Resolve a Reflector `update`-event asset key to a canonical `AssetIdentity`.
///
/// Reflector keys are **ticker symbols**, not contract addresses — confirmed
/// against captured samples (`lore/4-notes/samples/soroban-events/REFLECTOR.jsonl`:
/// `XLM`, `USDC`, `USDT`, `EURC`, `BTC`, `EUR`, …). We resolve only the assets we
/// price *through*: the USD-pegged stables (`USDC`, `USDT`) and `XLM`. Everything
/// else returns `None` and the sample is dropped rather than minted into
/// `prices.assets`.
///
/// Two reasons a symbol is dropped, and they are not the same:
///   - **No Stellar identity** (`EUR`, `BTC`, `ETH`, `XAU`, …): pure FX/crypto
///     reference rates that aren't tradeable Stellar assets, so they could never
///     be a candle's quote and could never match the USD-close ASOF join.
///   - **Deliberately out of scope** (`EURC`, …): EURC *is* a tradeable Stellar
///     classic (Circle's Euro Coin) and *could* appear as a candle quote, but the
///     USD-close reference set is intentionally restricted to USD-pegged + XLM
///     quotes. We do not price through a EUR-denominated stable, so an
///     EURC-quoted candle is an unsupported quote (→ no `close_usd`), by design —
///     not a coverage gap. Pricing through EURC would require its own Reflector
///     reference arm here plus a EURC/USDC pivot in the peg-pivot tier.
fn reflector_key_to_identity(key: &str) -> Option<AssetIdentity> {
    match key {
        "XLM" | "native" => Some(AssetIdentity::Native),
        "USDC" => Some(AssetIdentity::Credit {
            code: "USDC".to_string(),
            issuer: USDC_ISSUER.to_string(),
        }),
        "USDT" => Some(AssetIdentity::Credit {
            code: "USDT".to_string(),
            issuer: USDT_ISSUER.to_string(),
        }),
        _ => None,
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

/// The string/symbol value of the topic at `idx`, if any.
fn topic_symbol(topics: &Value, idx: usize) -> Option<&str> {
    topics
        .as_array()?
        .get(idx)?
        .as_object()?
        .get("value")?
        .as_str()
}

/// First topic's symbol value, if any (the event "signature"). Note: some
/// factories (Soroswap) put a name String in topic[0] and the real action
/// symbol in topic[1] — see `learn_factory`.
fn signature(topics: &Value) -> Option<&str> {
    topic_symbol(topics, 0)
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
                Some("REFLECTOR") => decode_reflector(ev, assets, &mut out),
                Some("REDSTONE") => decode_redstone(ev, contract_id.as_str(), assets, &mut out),
                _ => {
                    // Factory events grow the registry; pool events are queued.
                    learn_factory(ev, reg);
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
///
/// The action symbol is not always in topic[0]: Aquarius (`add_pool`) and
/// Phoenix (`create`) emit it there, but Soroswap's factory event is
/// `[String("SoroswapFactory"), Symbol("new_pair")]`, so its action lives in
/// topic[1]. We therefore check both positions.
fn learn_factory(ev: &xdr_parser::types::ExtractedEvent, reg: &mut Registries) {
    let sig0 = signature(&ev.topics);
    let sig1 = topic_symbol(&ev.topics, 1);

    // Aquarius router: Symbol("add_pool"), data (pool_address, pool_type)
    if sig0 == Some("add_pool") {
        if let Some(pool) = first_address(&ev.data) {
            reg.venue.insert(pool, Venue::Aquarius);
        }
        return;
    }

    // Phoenix factory: [Symbol("create"), Symbol("liquidity_pool")], data Address(pool)
    if sig0 == Some("create") {
        if sig1 == Some("liquidity_pool") {
            if let Some(pool) = address_value(&ev.data) {
                reg.venue.insert(pool.clone(), Venue::Phoenix);
                reg.phoenix.register(pool, phoenix_extractor::POOL_TYPE_XYK);
            }
        }
        return;
    }

    // Soroswap factory: [String("SoroswapFactory"), Symbol("new_pair")],
    // data { token_0, token_1, pair, ... }. The action symbol is in topic[1].
    if sig1 == Some("new_pair") {
        if let TaggedValue::Map(m) = json_to_tagged(&ev.data) {
            let get = |k: &str| {
                m.iter()
                    .find(|(key, _)| key.as_str() == Some(k))
                    .and_then(|(_, v)| v.as_address().map(String::from))
            };
            if let (Some(pair), Some(t0), Some(t1)) = (get("pair"), get("token_0"), get("token_1"))
            {
                reg.soroswap.register(pair.clone(), t0, t1);
                reg.venue.insert(pair, Venue::Soroswap);
            }
        }
    }
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

/// Resolve an AMM token (contract address) to a canonical `AssetIdentity`: the
/// underlying classic asset if the address is a known SAC (§12.4), else the
/// `Contract` identity for a pure Soroban token.
fn resolve_amm_token(contract_addr: &str, assets: &AssetRegistry) -> AssetIdentity {
    assets
        .resolve_sac(contract_addr)
        .unwrap_or_else(|| AssetIdentity::Contract(contract_addr.to_string()))
}

/// Convert a venue `TradeRow` into a `TradeTick` for the candle accumulator.
fn amm_trade_to_tick(
    trade: &extractors_core::TradeRow,
    closed_at: i64,
    assets: &mut AssetRegistry,
) -> Option<TradeTick> {
    // Collapse a SAC token onto its underlying classic identity (§12.4) so
    // AMM-via-SAC and SDEX-classic share one asset_id; a pure Soroban token keeps
    // its contract-address identity.
    let sold = resolve_amm_token(&trade.token_in, assets);
    let bought = resolve_amm_token(&trade.token_out, assets);
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
    assets: &mut AssetRegistry,
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
                    // Resolve to the canonical asset_id (task 0061 §5). Only the
                    // USD-pegged stables + XLM resolve; every other symbol is
                    // dropped — either it has no Stellar identity (EUR, BTC, …) or
                    // it's a tradeable asset we deliberately don't price through
                    // (EURC). See `reflector_key_to_identity` for the distinction.
                    let Some(identity) = reflector_key_to_identity(&key) else {
                        continue;
                    };
                    let asset_id = assets.get_or_assign(&identity);
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
}

/// REDSTONE carries a base64 XDR `bytes` payload (updated_feeds map). Full XDR
/// decode is deferred; we capture one row per event with the raw payload so the
/// `oracle_prices` byte footprint is measured (price left 0).
///
/// REDSTONE does not decode a per-asset symbol, so there is no canonical asset to
/// resolve. We key it to the emitting oracle contract via a `Contract` identity —
/// keeping it in the canonical id space (task 0061 §5, no synthetic ids) and out
/// of the `reflector` ASOF join (price_usd = 0, oracle_name = 'redstone').
fn decode_redstone(
    ev: &xdr_parser::types::ExtractedEvent,
    contract_id: &str,
    assets: &mut AssetRegistry,
    out: &mut LedgerSoroban,
) {
    let raw = ev
        .data
        .as_object()
        .and_then(|o| o.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let asset_id = assets.get_or_assign(&AssetIdentity::Contract(contract_id.to_string()));
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

    #[test]
    fn soroswap_factory_action_symbol_is_in_topic1() {
        // Regression: the Soroswap factory event carries a name String in
        // topic[0] and the real action symbol ("new_pair") in topic[1].
        // learn_factory must key off topic[1] for Soroswap — keying off
        // topic[0] (signature) silently never registers any Soroswap pool, so
        // all Soroswap swaps drop.
        let topics = json!([
            {"type":"string","value":"SoroswapFactory"},
            {"type":"sym","value":"new_pair"}
        ]);
        assert_eq!(signature(&topics), Some("SoroswapFactory"));
        assert_eq!(topic_symbol(&topics, 1), Some("new_pair"));
    }

    #[test]
    fn reflector_resolves_quote_symbols_to_canonical_identities() {
        assert_eq!(
            reflector_key_to_identity("XLM"),
            Some(AssetIdentity::Native)
        );
        assert_eq!(
            reflector_key_to_identity("USDC"),
            Some(AssetIdentity::Credit {
                code: "USDC".to_string(),
                issuer: USDC_ISSUER.to_string(),
            })
        );
        assert_eq!(
            reflector_key_to_identity("USDT"),
            Some(AssetIdentity::Credit {
                code: "USDT".to_string(),
                issuer: USDT_ISSUER.to_string(),
            })
        );
    }

    #[test]
    fn reflector_drops_non_stellar_reference_symbols() {
        // FX/crypto references (real keys in the captured samples) have no
        // Stellar tradeable identity — they must not be minted into the asset
        // registry / prices.assets.
        for k in ["EUR", "GBP", "BTC", "ETH", "EURC", "XAU"] {
            assert_eq!(reflector_key_to_identity(k), None, "key {k} should drop");
        }
    }

    #[test]
    fn reflector_usdc_matches_trade_quote_id() {
        // The load-bearing guarantee (task 0061 §5): a Reflector USDC oracle row
        // and a candle whose quote is USDC must land on the SAME asset_id, so the
        // enrichment ASOF join `o.asset_id = p.quote_asset_id` matches.
        let mut assets = AssetRegistry::from_existing(vec![]);
        let usdc = AssetIdentity::Credit {
            code: "USDC".to_string(),
            issuer: USDC_ISSUER.to_string(),
        };
        // Trade path interns USDC as a quote.
        let trade_quote_id = assets.get_or_assign(&usdc);
        // Oracle path resolves the Reflector "USDC" symbol.
        let oracle_id = assets.get_or_assign(&reflector_key_to_identity("USDC").unwrap());
        assert_eq!(trade_quote_id, oracle_id);
    }

    #[test]
    fn phoenix_factory_action_symbols_resolve_by_position() {
        // Phoenix: [Symbol("create"), Symbol("liquidity_pool")].
        let topics = json!([
            {"type":"sym","value":"create"},
            {"type":"sym","value":"liquidity_pool"}
        ]);
        assert_eq!(signature(&topics), Some("create"));
        assert_eq!(topic_symbol(&topics, 1), Some("liquidity_pool"));
    }
}
