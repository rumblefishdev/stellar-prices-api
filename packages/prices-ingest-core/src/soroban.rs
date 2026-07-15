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
use stellar_xdr::{LedgerCloseMeta, TransactionMeta};
use tracing::warn;

use extractors_core::{SorobanEventRow, TaggedValue, Venue, VenueRegistry};
use ledger_processor::dispatch::dispatch;
use phoenix_extractor::PhoenixPoolRegistry;
use soroswap_extractor::SoroswapPoolRegistry;
use xdr_parser::extract_events;
use xdr_parser::types::EventSource;

use crate::canonical::{AssetIdentity, AssetRegistry, USDC_ISSUER, USDT_ISSUER, canonicalise};
use crate::tick::TradeTick;
use crate::writer::OracleSample;

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
pub fn reflector_key_to_identity(key: &str) -> Option<AssetIdentity> {
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

/// A `swap`-shaped Soroban event decoded for a contract that is absent from the
/// venue registry, so it could not be classified to a venue/pool and its volume
/// was dropped.
///
/// On a clean forward-discovery run (the AMM window starting exactly at Soroban
/// activation) this never happens: every pool's factory-create event is decoded
/// before any of its swaps, so the pool is always registered by swap time. A
/// populated record therefore means either an extractor gap (an event class we
/// don't yet recognise) or a pool created before the window — `sample_topics`
/// carries the event shape for the post-run re-check. Non-swap events on
/// unknown contracts are correctly ignored (not every contract is an AMM pool).
#[derive(Debug, Clone)]
pub struct UnresolvedPoolSwap {
    pub contract_id: String,
    pub ledger_sequence: u32,
    /// How many `swap` events this contract dropped in this ledger.
    pub swap_count: u32,
    /// Debug rendering of the first dropped swap's topics — the diagnostic hint.
    pub sample_topics: String,
}

/// Output of processing one ledger's Soroban events.
#[derive(Default)]
pub struct LedgerSoroban {
    /// (source, tick) pairs; source ∈ {phoenix, soroswap, aquarius}.
    pub amm_ticks: Vec<(&'static str, TradeTick)>,
    pub oracle: Vec<OracleSample>,
    /// Contracts that emitted a `swap` but were not in the venue registry.
    pub unresolved: Vec<UnresolvedPoolSwap>,
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
                Some("REDSTONE") => decode_redstone(ev, &mut out),
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

        classify_amm_groups(amm_groups, reg, assets, ledger_seq, closed_at, &mut out);
    }

    out
}

/// Classify each contract's queued AMM event rows against the current registry.
///
/// This is the resolvability seam **task 0078** protects. A pool present in
/// `reg.venue` is dispatched to price ticks (appended to `out.amm_ticks`); a
/// pool that is *absent* but emitted a `swap` is recorded in `out.unresolved`
/// so its dropped volume is visible to the post-run re-check (guard from task
/// 0053 decision #3). With the live processor now preloading `pool_registry`
/// (0078), a pre-existing pool is seeded before its first live swap and lands in
/// `amm_ticks` instead of `unresolved` — the empty-registry regression this fix
/// closes. Kept a standalone fn so that seeded-vs-unseeded behaviour is
/// unit-testable without a full XDR `LedgerCloseMeta` AMM fixture (none exist).
fn classify_amm_groups(
    amm_groups: HashMap<String, Vec<SorobanEventRow>>,
    reg: &Registries,
    assets: &mut AssetRegistry,
    ledger_seq: u32,
    closed_at: i64,
    out: &mut LedgerSoroban,
) {
    for (contract_id, rows) in amm_groups {
        // Pool-level `swap` events for this contract — the volume-bearing events
        // whose loss must never be silent. Computed once and reused by both the
        // unknown-venue branch and the venue-known-but-unpriced fallback below.
        //
        // Exception (task 0087): the Aquarius *router* emits a `swap` *summary*
        // (address `Vec` at topic[1]) wrapping the pool-level `trade`. It is
        // deliberately ignored for pricing — matching it would double-count the
        // pool `trade` — and must NOT be flagged as a gap, or it fatal-trips the
        // unresolved-pools guard. A genuine pool `swap` carries no address `Vec`
        // at topic[1] and is still counted here.
        let swaps: Vec<&SorobanEventRow> = rows
            .iter()
            .filter(|r| r.topics.first().and_then(|t| t.as_str()) == Some("swap"))
            .filter(|r| !is_aquarius_router_swap(r))
            .collect();

        let venue = match reg.venue.get(&contract_id) {
            Some(v) => v.clone(),
            None => {
                // Unknown contract. Most are not AMM pools and are correctly
                // ignored — but if this one emitted a pool-level `swap`, its
                // volume is being dropped. Record it for the post-run re-check
                // instead of silently skipping (guard from task 0053 decision #3).
                if let Some(rec) = unresolved_from_swaps(contract_id, &swaps, ledger_seq) {
                    out.unresolved.push(rec);
                }
                continue;
            }
        };

        let source = venue.as_source();
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

        // The one silent-drop task 0096 closes: a Soroswap pool that is
        // venue-known but ABSENT from `reg.soroswap` (its pair tokens were not
        // seeded before this run). A Soroswap `swap` omits the token pair, so
        // `dispatch` cannot price it and returns `Ok(vec![])`; because the pool
        // IS venue-known it never reached the unknown-venue branch, so its swap
        // volume vanished with neither a candle NOR an `unresolved_pools` row.
        // Record it once so the dropped volume is visible. Seeding the registry
        // makes it resolvable, so `run.rs`'s re-check keeps `still_unresolved`
        // 0 — recorded, recoverable, not a fatal genuine-gap.
        //
        // Scoped narrowly to this pair-resolution miss (checked directly against
        // `reg.soroswap`, NOT inferred from a zero tick count): a *resolvable*
        // pool that merely produced no tick — a zero-amount swap, an unmappable
        // asset, or a Phoenix dispatch error — is NOT a dropped-pool gap and is
        // deliberately not recorded here. Inferring the gap from "no tick" would
        // flood `unresolved_pools` with false positives on healthy pools and
        // grow the run's in-memory `unresolved` unboundedly.
        if matches!(venue, Venue::Soroswap) && !reg.soroswap.contains(&contract_id) {
            if let Some(rec) = unresolved_from_swaps(contract_id, &swaps, ledger_seq) {
                out.unresolved.push(rec);
            }
        }
    }
}

/// Build an [`UnresolvedPoolSwap`] from a contract's pool-level `swap` events, or
/// `None` if it emitted none. Shared by the unknown-venue branch and the
/// venue-known-but-unpriced fallback in [`classify_amm_groups`] so both record
/// the dropped volume identically (task 0096).
fn unresolved_from_swaps(
    contract_id: String,
    swaps: &[&SorobanEventRow],
    ledger_seq: u32,
) -> Option<UnresolvedPoolSwap> {
    let first = swaps.first()?;
    Some(UnresolvedPoolSwap {
        contract_id,
        ledger_sequence: ledger_seq,
        swap_count: swaps.len() as u32,
        sample_topics: format!("{:?}", first.topics),
    })
}

/// Recognise the Aquarius-router `swap` *summary* event by its topic shape.
///
///   topics = [ Symbol("swap"),
///              Vec([ Address(tokenA), Address(tokenB) ]),
///              Address(<swapper>) ]
///
/// The address `Vec` at topic[1] is the router signature: pool-level `swap`
/// events (the genuine-gap case the guard must still catch) never carry it.
/// See the router sample in `lore/4-notes/samples/soroban-events/swap.jsonl`
/// and the emitter note in `aquarius-extractor/src/lib.rs` (task 0087).
fn is_aquarius_router_swap(row: &SorobanEventRow) -> bool {
    row.topics.first().and_then(|t| t.as_str()) == Some("swap")
        && matches!(row.topics.get(1), Some(TaggedValue::Vec(_)))
}

fn topics_to_tagged(topics: &Value) -> Vec<TaggedValue> {
    topics
        .as_array()
        .map(|a| a.iter().map(json_to_tagged).collect())
        .unwrap_or_default()
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

/// Reserved `asset_id` for an oracle feed that is not a tradeable asset (e.g. the
/// REDSTONE emitter contract). Never assigned by the [`AssetRegistry`] (its ids
/// start at 1), so it maps to no `prices.assets` row — the feed stays out of the
/// asset read surface while its `oracle_prices` row is still recorded.
const ORACLE_FEED_NO_ASSET_ID: u32 = 0;

/// REDSTONE carries a base64 XDR `bytes` payload (updated_feeds map). Full XDR
/// decode is deferred; we capture one row per event with the raw payload so the
/// `oracle_prices` byte footprint is measured (price left 0).
///
/// REDSTONE does not decode a per-asset symbol, so there is no tradeable asset to
/// resolve. We deliberately do NOT intern the emitting oracle contract into the
/// `AssetRegistry`: an oracle feed is not an asset, and interning it would persist
/// it to `prices.assets` and leak it into the contract-keyed read surface
/// (`identity_by_contract`, `current_price_usd`), where a consumer resolving a
/// pool-leg contract address could match an oracle feed as if it were a token.
/// Instead the row carries the reserved [`ORACLE_FEED_NO_ASSET_ID`] sentinel — its
/// `asset_id` is functionally dead anyway (price_usd = 0, oracle_name =
/// 'redstone'; never read by the `reflector` ASOF join), and the raw payload is
/// preserved for the byte-footprint measurement.
fn decode_redstone(ev: &xdr_parser::types::ExtractedEvent, out: &mut LedgerSoroban) {
    let raw = ev
        .data
        .as_object()
        .and_then(|o| o.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    out.oracle.push(OracleSample {
        timestamp: ev.created_at.max(0) as u32,
        asset_id: ORACLE_FEED_NO_ASSET_ID,
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
    fn oracle_feed_sentinel_never_collides_with_a_real_asset_id() {
        // The REDSTONE sentinel (task 0061 review #2) must be disjoint from every
        // id the registry assigns, so an oracle-feed `oracle_prices` row maps to
        // NO `prices.assets` row and cannot leak into the contract read surface
        // (`identity_by_contract` / `current_price_usd`). Registry ids start at 1,
        // leaving 0 reserved.
        let mut reg = AssetRegistry::from_existing(vec![]);
        let native = reg.get_or_assign(&AssetIdentity::Native);
        assert_ne!(native, ORACLE_FEED_NO_ASSET_ID);
        assert!(native >= 1, "registry ids must start at 1");
        // A Contract identity — what REDSTONE used to intern — also never hits 0.
        let contract = reg.get_or_assign(&AssetIdentity::Contract("CORACLEFEED".to_string()));
        assert_ne!(contract, ORACLE_FEED_NO_ASSET_ID);
    }

    #[test]
    fn seeded_pool_swap_prices_where_unseeded_falls_to_unresolved() {
        // Task 0078 resolvability guarantee at the classify seam: an AMM swap for
        // a pre-existing pool must PRICE when the pool_registry is seeded (as the
        // live processor's cold-start preload now provides) and only fall to
        // `unresolved` when the registry is empty (the regression this fix
        // closes). Uses the shared Phoenix xyk swap fixture — no XDR ledger
        // fixture with AMM activity exists to drive the full `process_ledger`.
        use phoenix_extractor::test_fixtures::{
            XLM_USDC_POOL, common_xyk_wasm_hash, make_phoenix_xyk_events,
        };

        const SEQ: u32 = 62_460_522;
        const CLOSED_AT: i64 = 1_700_000_000;
        let group = || {
            let mut g: HashMap<String, Vec<SorobanEventRow>> = HashMap::new();
            g.insert(
                XLM_USDC_POOL.to_string(),
                make_phoenix_xyk_events(XLM_USDC_POOL, 0),
            );
            g
        };

        // Unseeded registry → swap volume is dropped to `unresolved` (the bug).
        let empty = Registries::new();
        let mut assets = AssetRegistry::from_existing(vec![]);
        let mut out = LedgerSoroban::default();
        classify_amm_groups(group(), &empty, &mut assets, SEQ, CLOSED_AT, &mut out);
        assert!(out.amm_ticks.is_empty(), "unseeded pool must not price");
        assert_eq!(
            out.unresolved.len(),
            1,
            "unseeded swap recorded as unresolved"
        );
        assert_eq!(out.unresolved[0].contract_id, XLM_USDC_POOL);

        // Seeded registry (preloaded pool_registry) → the same swap prices.
        let mut seeded = Registries::new();
        seeded
            .venue
            .insert(XLM_USDC_POOL.to_string(), Venue::Phoenix);
        seeded
            .phoenix
            .register_with_wasm(XLM_USDC_POOL.to_string(), 0, common_xyk_wasm_hash());
        let mut assets = AssetRegistry::from_existing(vec![]);
        let mut out = LedgerSoroban::default();
        classify_amm_groups(group(), &seeded, &mut assets, SEQ, CLOSED_AT, &mut out);
        assert!(
            out.unresolved.is_empty(),
            "seeded pool must not fall to unresolved"
        );
        assert_eq!(out.amm_ticks.len(), 1, "seeded swap prices to a tick");
        assert_eq!(
            out.amm_ticks[0].0, "phoenix",
            "tick tagged with the phoenix source"
        );
    }

    #[test]
    fn venue_known_but_unresolvable_soroswap_pool_is_recorded_not_silent() {
        // Task 0096: a Soroswap pool present in `reg.venue` but ABSENT from
        // `reg.soroswap` (its pair tokens were not seeded before this run) makes
        // `dispatch` return `Ok(vec![])` — no trade, no error. Because the pool
        // IS venue-known it never reaches the unknown-contract branch, so before
        // this fix it produced NEITHER a candle NOR an `unresolved_pools` row:
        // the swap volume vanished silently. It must now be recorded so the gap
        // is visible. This is exactly the prod Soroswap gap (registry seeded
        // 2026-07-14, after the Soroban backfill run).
        const SEQ: u32 = 50_600_000;
        const CLOSED_AT: i64 = 1_700_000_000;
        const SOROSWAP_POOL: &str = "CCR2CH4GQVCZHG7CHFVMNANCK45CU5DVKXZIIITDZQAU3CEJZ7RQH2MQ";

        let pool_swap = SorobanEventRow {
            contract_id: SOROSWAP_POOL.to_string(),
            transaction_id: "tx-soroswap".to_string(),
            ledger_sequence: SEQ as u64,
            event_index: 4,
            // Single `swap` topic (no address Vec) — a genuine pool swap, not the
            // Aquarius router summary. Data shape is irrelevant here: `dispatch`
            // returns Ok(vec![]) at the `reg.soroswap` miss, before any decode.
            topics: vec![TaggedValue::Symbol("swap".to_string())],
            data: TaggedValue::Vec(vec![]),
        };

        let mut groups: HashMap<String, Vec<SorobanEventRow>> = HashMap::new();
        groups.insert(SOROSWAP_POOL.to_string(), vec![pool_swap]);

        // Venue-known as Soroswap, but the pair is NOT in `reg.soroswap`.
        let mut reg = Registries::new();
        reg.venue.insert(SOROSWAP_POOL.to_string(), Venue::Soroswap);

        let mut assets = AssetRegistry::from_existing(vec![]);
        let mut out = LedgerSoroban::default();
        classify_amm_groups(groups, &reg, &mut assets, SEQ, CLOSED_AT, &mut out);

        assert!(
            out.amm_ticks.is_empty(),
            "an unresolvable soroswap pool must not price"
        );
        assert_eq!(
            out.unresolved.len(),
            1,
            "a venue-known but unresolvable soroswap swap must be recorded, not dropped"
        );
        assert_eq!(out.unresolved[0].contract_id, SOROSWAP_POOL);
        assert_eq!(out.unresolved[0].swap_count, 1);
    }

    #[test]
    fn soroswap_pair_swap_prices_through_classify() {
        // End-to-end at the classify seam — the layer where the prod "0 soroswap
        // candles" originated (task 0096). A real SoroswapPair-shaped swap
        // (`topics=[String("SoroswapPair"), Symbol("swap")]`, action in topic[1])
        // for a venue-known + `reg.soroswap`-seeded pool must now dispatch through
        // the fixed extractor and produce an `amm_tick` tagged `soroswap`.
        const SEQ: u32 = 50_704_650;
        const CLOSED_AT: i64 = 1_700_000_000;
        const POOL: &str = "CDBBBNMCWRMWEIFHUD5BXBCRTW6QM33ZEXIOBGKKQNDSH3WEF7WVBGMI";
        const T0: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
        const T1: &str = "CAUIKL3IYGMERDRUN6YSCLWVAKIFG5Q4YJHUKM4S4NJZQIA3BAS6OJPK";

        let swap = SorobanEventRow {
            contract_id: POOL.to_string(),
            transaction_id: "tx".to_string(),
            ledger_sequence: SEQ as u64,
            event_index: 5,
            topics: vec![
                TaggedValue::String("SoroswapPair".to_string()),
                TaggedValue::Symbol("swap".to_string()),
            ],
            data: TaggedValue::Map(vec![
                (
                    TaggedValue::Symbol("amount_0_in".into()),
                    TaggedValue::I128(1_000_000),
                ),
                (
                    TaggedValue::Symbol("amount_0_out".into()),
                    TaggedValue::I128(0),
                ),
                (
                    TaggedValue::Symbol("amount_1_in".into()),
                    TaggedValue::I128(0),
                ),
                (
                    TaggedValue::Symbol("amount_1_out".into()),
                    TaggedValue::I128(914_145),
                ),
            ]),
        };

        let mut groups: HashMap<String, Vec<SorobanEventRow>> = HashMap::new();
        groups.insert(POOL.to_string(), vec![swap]);

        let mut reg = Registries::new();
        reg.venue.insert(POOL.to_string(), Venue::Soroswap);
        reg.soroswap
            .register(POOL.to_string(), T0.to_string(), T1.to_string());

        let mut assets = AssetRegistry::from_existing(vec![]);
        let mut out = LedgerSoroban::default();
        classify_amm_groups(groups, &reg, &mut assets, SEQ, CLOSED_AT, &mut out);

        assert_eq!(
            out.amm_ticks.len(),
            1,
            "a SoroswapPair-shaped swap must now price (was 0 before the fix)"
        );
        assert_eq!(out.amm_ticks[0].0, "soroswap", "tick tagged soroswap");
        assert!(
            out.unresolved.is_empty(),
            "a priced pool is not recorded as unresolved"
        );
    }

    #[test]
    fn resolvable_soroswap_pool_with_no_priced_tick_is_not_recorded() {
        // Task 0096 (review follow-up): the silent-drop fix must fire ONLY for a
        // pair-resolution miss, never for any zero-tick outcome. A Soroswap pool
        // that IS in `reg.soroswap` but emits a zero-amount swap resolves fine —
        // `amm_trade_to_tick` returns None (zero volume). It must NOT be recorded
        // as an unresolved gap: that would be a false positive on a healthy pool
        // and grow the run's in-memory `unresolved` list unboundedly.
        const SEQ: u32 = 50_600_000;
        const CLOSED_AT: i64 = 1_700_000_000;
        const POOL: &str = "CCR2CH4GQVCZHG7CHFVMNANCK45CU5DVKXZIIITDZQAU3CEJZ7RQH2MQ";
        const TOKEN0: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
        const TOKEN1: &str = "CAUIKL3IYGMERDRUN6YSCLWVAKIFG5Q4YJHUKM4S4NJZQIA3BAS6OJPK";

        // A uniswap-v2-shaped swap with all-zero amounts → decodes to a trade
        // with amount_in == amount_out == 0 → `amm_trade_to_tick` returns None.
        let zero_swap = SorobanEventRow {
            contract_id: POOL.to_string(),
            transaction_id: "tx-zero".to_string(),
            ledger_sequence: SEQ as u64,
            event_index: 4,
            topics: vec![TaggedValue::Symbol("swap".to_string())],
            data: TaggedValue::Map(vec![
                (
                    TaggedValue::Symbol("amount_0_in".to_string()),
                    TaggedValue::I128(0),
                ),
                (
                    TaggedValue::Symbol("amount_1_in".to_string()),
                    TaggedValue::I128(0),
                ),
                (
                    TaggedValue::Symbol("amount_0_out".to_string()),
                    TaggedValue::I128(0),
                ),
                (
                    TaggedValue::Symbol("amount_1_out".to_string()),
                    TaggedValue::I128(0),
                ),
            ]),
        };

        let mut groups: HashMap<String, Vec<SorobanEventRow>> = HashMap::new();
        groups.insert(POOL.to_string(), vec![zero_swap]);

        // Venue-known AND resolvable — the pair IS present in `reg.soroswap`.
        let mut reg = Registries::new();
        reg.venue.insert(POOL.to_string(), Venue::Soroswap);
        reg.soroswap
            .register(POOL.to_string(), TOKEN0.to_string(), TOKEN1.to_string());

        let mut assets = AssetRegistry::from_existing(vec![]);
        let mut out = LedgerSoroban::default();
        classify_amm_groups(groups, &reg, &mut assets, SEQ, CLOSED_AT, &mut out);

        assert!(
            out.amm_ticks.is_empty(),
            "a zero-amount swap produces no tick"
        );
        assert!(
            out.unresolved.is_empty(),
            "a resolvable pool with a zero-amount swap must NOT be recorded as unresolved"
        );
    }

    #[test]
    fn router_swap_is_not_flagged_but_genuine_pool_swap_still_is() {
        // Task 0087: the Aquarius router's `swap` summary (address `Vec` at
        // topic[1]) is a deliberately-ignored wrapper over the pool `trade`. It
        // reaches the unknown-contract branch (routers are never registered) but
        // must NOT be recorded as an unresolved-pool gap — else it fatal-trips
        // the guard on a clean backfill. A genuine unknown-pool `swap` (no Vec
        // topic) must still be flagged, keeping the safety net intact. Neither
        // may produce a candle (the router event must not double-count).
        const SEQ: u32 = 50_639_018;
        const CLOSED_AT: i64 = 1_700_000_000;
        const ROUTER: &str = "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK";
        const UNKNOWN_POOL: &str = "CCR2CH4GQVCZHG7CHFVMNANCK45CU5DVKXZIIITDZQAU3CEJZ7RQH2MQ";
        const TOKEN_A: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
        const TOKEN_B: &str = "CAUIKL3IYGMERDRUN6YSCLWVAKIFG5Q4YJHUKM4S4NJZQIA3BAS6OJPK";

        let router_swap = SorobanEventRow {
            contract_id: ROUTER.to_string(),
            transaction_id: "tx-router".to_string(),
            ledger_sequence: SEQ as u64,
            event_index: 3,
            // [ Symbol("swap"), Vec([Address, Address]), Address(swapper) ]
            topics: vec![
                TaggedValue::Symbol("swap".to_string()),
                TaggedValue::Vec(vec![
                    TaggedValue::Address(TOKEN_A.to_string()),
                    TaggedValue::Address(TOKEN_B.to_string()),
                ]),
                TaggedValue::Address(
                    "GBCC4DK4KJQTQ6CZVNOZTZYDHMFKZMRSXLE2JN3NWBBGEAUKZHUTDDXA".to_string(),
                ),
            ],
            data: TaggedValue::Vec(vec![]),
        };
        // Genuine unknown-pool swap: single `swap` topic, no address Vec.
        let pool_swap = SorobanEventRow {
            contract_id: UNKNOWN_POOL.to_string(),
            transaction_id: "tx-pool".to_string(),
            ledger_sequence: SEQ as u64,
            event_index: 4,
            topics: vec![TaggedValue::Symbol("swap".to_string())],
            data: TaggedValue::Vec(vec![]),
        };

        let mut groups: HashMap<String, Vec<SorobanEventRow>> = HashMap::new();
        groups.insert(ROUTER.to_string(), vec![router_swap]);
        groups.insert(UNKNOWN_POOL.to_string(), vec![pool_swap]);

        let reg = Registries::new(); // neither contract registered
        let mut assets = AssetRegistry::from_existing(vec![]);
        let mut out = LedgerSoroban::default();
        classify_amm_groups(groups, &reg, &mut assets, SEQ, CLOSED_AT, &mut out);

        // Router swap must NOT double-count and must NOT be flagged.
        assert!(out.amm_ticks.is_empty(), "router swap must not price");
        assert_eq!(
            out.unresolved.len(),
            1,
            "only the genuine unknown-pool swap is flagged"
        );
        assert_eq!(
            out.unresolved[0].contract_id, UNKNOWN_POOL,
            "the flagged gap is the real unknown pool, not the router"
        );
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
