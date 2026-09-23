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
