//! Task 0300 — the Comet weighted pool (Blend's backstop, BLND/USDC) as the AMM
//! venue `comet`, pinned end to end on REAL production payloads.
//!
//! Every fixture below is a verbatim `topics_xdr` / `data_xdr` pair read from
//! BE's `default.soroban_events` (typed-JSON SCVal, the shape
//! `process_soroban_event_rows` consumes), cited by ledger / transaction index /
//! operation / event index. Numbers and addresses are copied, never retyped.
//!
//! A child module of `soroban` so it can reach the private
//! `classify_amm_groups`, `amm_trade_to_tick` and `unregistered_pool_venue`
//! through `super::`, without growing `soroban.rs` by fifteen JSON fixtures.

use super::*;
use serde_json::json;

/// The Comet BLND/USDC pool. NOT the XLM SAC `CAS3J7GY…` many other tests use.
const COMET: &str = "CAS3FL6TLZKDGGSISDBWGGPXT3NRR4DYTZD7YOD3HMYO6LTJUVGRVEAM";
/// BLND's issuer (also the account that deployed the Comet pool).
const BLND_ISSUER: &str = "GDJEHTBE6ZHUXSWFI642DCGLUOECLHPF3KSXHPXTSTJ7E3JF6MQ5EZYY";
const BLND_ID: u32 = 225;
const USDC_ID: u32 = 3;
const CLOSED_AT: i64 = 1_700_000_000;

/// Production's asset ids for the pool's two tokens. `from_existing` registers
/// the SAC of every known classic identity, so both SACs collapse onto them.
fn seeded_assets() -> AssetRegistry {
    AssetRegistry::from_existing(vec![
        (
            USDC_ID,
            AssetIdentity::Credit {
                code: "USDC".to_string(),
                issuer: USDC_ISSUER.to_string(),
            },
        ),
        (
            BLND_ID,
            AssetIdentity::Credit {
                code: "BLND".to_string(),
                issuer: BLND_ISSUER.to_string(),
            },
        ),
    ])
}

/// The registry the preload builds once `pool_registry` holds the Comet row.
fn comet_registry() -> Registries {
    let mut reg = Registries::new();
    reg.venue.insert(COMET.to_string(), Venue::Comet);
    reg
}

/// A 7-dp raw amount as the Decimal `amm_trade_to_tick` builds from it.
fn d(raw: i128) -> Decimal {
    Decimal::from_i128_with_scale(raw, 7)
}

/// One Comet event as the events-backfill reads it. `transaction_id` uses the
/// live form `"<ledger>:<tx idx>"`.
fn comet_event(
    ledger: u32,
    tx_index: u16,
    event_index: u32,
    topics: Value,
    data: Value,
) -> RawSorobanEvent {
    RawSorobanEvent {
        contract_id: COMET.to_string(),
        transaction_id: format!("{ledger}:{tx_index}"),
        transaction_index: tx_index,
        ledger_sequence: ledger,
        event_index,
        topics,
        data,
    }
}

/// Run one ledger's events through the backfill seam.
fn run(
    ledger: u32,
    events: &[RawSorobanEvent],
    reg: &mut Registries,
    assets: &mut AssetRegistry,
) -> LedgerSoroban {
    let mut out = LedgerSoroban::default();
    process_soroban_event_rows(ledger, CLOSED_AT, events, reg, assets, &mut out);
    out
}

/// `typical_usdc_to_blnd_recent` — ledger 64,570,597, tx idx 627, op 0,
/// event 21. USDC → BLND, 3454229 in / 621636466 out.
fn typical_usdc_to_blnd_recent() -> RawSorobanEvent {
    comet_event(
        64_570_597,
        627,
        21,
        json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
        json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "CBNVK5PE7JCL773P5SHWE3YUCHQVVVPVOG72SKCEFVTZOLLQWVTCPLZ3"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "3454229"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "621636466"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}}]}),
    )
}

/// The tracer (task 0300 D5): a registered Comet pool's real `POOL/swap` is a
/// `"comet"` tick, BLND based and USDC quoted, priced in USDC per BLND.
#[test]
fn a_registered_comet_pool_swap_becomes_a_comet_tick() {
    let mut reg = comet_registry();
    let mut assets = seeded_assets();
    let out = run(
        64_570_597,
        &[typical_usdc_to_blnd_recent()],
        &mut reg,
        &mut assets,
    );

    assert_eq!(out.amm_ticks.len(), 1, "one swap, one tick");
    let (source, tick) = &out.amm_ticks[0];
    assert_eq!(*source, "comet");
    assert_eq!(tick.base_id, BLND_ID, "BLND is the base");
    assert_eq!(tick.quote_id, USDC_ID, "USDC is the quote");
    // USDC → BLND canonicalises inverted: price = USDC in / BLND out.
    assert_eq!(tick.price, d(3_454_229) / d(621_636_466));
    assert_eq!(tick.volume_base, d(621_636_466));
    assert_eq!(tick.volume_quote, d(3_454_229));
    assert!(tick.price_forming);
    assert_eq!(tick.operation_index, 21);
    assert_eq!(tick.transaction_index, 627);
    assert!(out.dispatch_errors.is_empty());
    assert!(out.unresolved.is_empty());
}

/// `self_swap_usdc` — ledger 64,112,340, tx idx 367, op 0, event 4. The
/// 2026-08-25 exploit's USDC → USDC swap (caller `CA27AABN…`).
fn self_swap_usdc() -> RawSorobanEvent {
    comet_event(
        64_112_340,
        367,
        4,
        json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
        json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "CA27AABNFZJRDEPW4AGB5IEC4VNYCSWYOZJNZ6GI6GLQALFUWLTLJHYQ"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "2594103172416"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "1941196544582"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}]}),
    )
}

/// `exploit_cycle_dump` — ledger 64,112,340, tx idx 367, op 0, event 20. The
/// same transaction's BLND → USDC dump, 80150040873159 in / 4251768634888 out.
fn exploit_cycle_dump() -> RawSorobanEvent {
    comet_event(
        64_112_340,
        367,
        20,
        json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
        json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "CA27AABNFZJRDEPW4AGB5IEC4VNYCSWYOZJNZ6GI6GLQALFUWLTLJHYQ"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "80150040873159"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "4251768634888"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}]}),
    )
}

/// Task 0300 D1: a real Comet self-swap prices nothing and is not an error.
#[test]
fn a_real_comet_self_swap_yields_no_tick_and_no_error() {
    let mut reg = comet_registry();
    let mut assets = seeded_assets();
    let before = assets.assets().count();
    let out = run(64_112_340, &[self_swap_usdc()], &mut reg, &mut assets);

    assert!(out.amm_ticks.is_empty());
    assert!(
        out.dispatch_errors.is_empty(),
        "a self-swap is not an error"
    );
    assert!(out.unresolved.is_empty());
    assert_eq!(assets.assets().count(), before);
}

/// Task 0300 D1 + D2: in the exploit transaction the self-swap drops, and the
/// dump beside it is indexed as it is — no caller or transaction exclusion.
#[test]
fn the_exploit_dump_ticks_as_is_beside_its_dropped_self_swap() {
    let mut reg = comet_registry();
    let mut assets = seeded_assets();
    let out = run(
        64_112_340,
        &[self_swap_usdc(), exploit_cycle_dump()],
        &mut reg,
        &mut assets,
    );

    assert_eq!(out.amm_ticks.len(), 1, "only the dump prices");
    let (source, tick) = &out.amm_ticks[0];
    assert_eq!(*source, "comet");
    assert_eq!((tick.base_id, tick.quote_id), (BLND_ID, USDC_ID));
    // BLND → USDC: price = USDC out / BLND in (≈ 0.053048).
    assert_eq!(tick.price, d(4_251_768_634_888) / d(80_150_040_873_159));
    assert_eq!(tick.operation_index, 20);
    assert!(tick.price_forming);
    assert!(out.dispatch_errors.is_empty());
    assert!(out.unresolved.is_empty());
}

/// Task 0300 D1 holds for a second venue through the same seam: a registered
/// Aquarius pool's `trade` of a token for itself (SYNTHETIC — the shape of the
/// real Aquarius `trade`, with sold == bought) gives no tick and no error.
#[test]
fn an_aquarius_self_trade_yields_no_tick_and_no_error() {
    const AQUA: &str = "CDE57N6XTUPBKYYDGQMXX7E7SLNOLFY3JEQB4MULSMR2AKTSAENGX2HC";
    const TOKEN: &str = "CAUIKL3IYGMERDRUN6YSCLWVAKIFG5Q4YJHUKM4S4NJZQIA3BAS6OJPK";
    let trade = RawSorobanEvent {
        contract_id: AQUA.to_string(),
        transaction_id: "62078348:1".to_string(),
        transaction_index: 1,
        ledger_sequence: 62_078_348,
        event_index: 5,
        topics: json!([
            {"type": "sym", "value": "trade"},
            {"type": "address", "value": TOKEN},
            {"type": "address", "value": TOKEN},
            {"type": "address", "value": "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK"}
        ]),
        data: json!({"type": "vec", "value": [
            {"type": "i128", "value": "930000000"},
            {"type": "i128", "value": "423899086439"},
            {"type": "i128", "value": "465000"}
        ]}),
    };
    let mut reg = Registries::new();
    reg.venue.insert(AQUA.to_string(), Venue::Aquarius);
    let mut assets = AssetRegistry::from_existing(vec![]);
    let out = run(62_078_348, &[trade.clone()], &mut reg, &mut assets);

    assert!(out.amm_ticks.is_empty());
    assert!(out.dispatch_errors.is_empty());
    assert!(out.unresolved.is_empty());

    // The same event with distinct tokens does price — the guard, not the
    // fixture, is what drops it.
    let mut distinct = trade;
    distinct.topics[2]["value"] = json!("CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA");
    let out = run(62_078_348, &[distinct], &mut reg, &mut assets);
    assert_eq!(out.amm_ticks.len(), 1);
    assert_eq!(out.amm_ticks[0].0, "aquarius");
}
