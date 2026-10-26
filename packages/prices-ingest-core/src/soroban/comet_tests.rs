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
const CLOSED_AT: i64 = 1_700_000_000;

fn usdc() -> AssetIdentity {
    AssetIdentity::Credit {
        code: "USDC".to_string(),
        issuer: USDC_ISSUER.to_string(),
    }
}

fn blnd() -> AssetIdentity {
    AssetIdentity::Credit {
        code: "BLND".to_string(),
        issuer: BLND_ISSUER.to_string(),
    }
}

/// The pool's two tokens, as already known. `from_existing` registers the SAC
/// of every known classic identity, so both SACs collapse onto them.
fn seeded_assets() -> AssetRegistry {
    AssetRegistry::from_existing(vec![usdc(), blnd()])
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
    assert_eq!(tick.base, blnd(), "BLND is the base");
    assert_eq!(tick.quote, usdc(), "USDC is the quote");
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
    assert_eq!((&tick.base, &tick.quote), (&blnd(), &usdc()));
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
    assets.set_decimals(TOKEN.to_string(), 7);
    let out = run(
        62_078_348,
        std::slice::from_ref(&trade),
        &mut reg,
        &mut assets,
    );

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

// The rest of the real samples: 13 distinct events in all. `first_swap` is the
// same event as `dust_usdc_to_blnd`, and `dust_out_blnd_to_usdc` the same as
// `dust_blnd_to_usdc`, so the duplicates are not repeated.

/// `typical_blnd_to_usdc_recent` — ledger 64,573,919, tx idx 233, op 0,
/// event 9. BLND → USDC, 1263056538 in / 6938342 out.
fn typical_blnd_to_usdc_recent() -> RawSorobanEvent {
    comet_event(
        64_573_919,
        233,
        9,
        json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
        json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "CBNVK5PE7JCL773P5SHWE3YUCHQVVVPVOG72SKCEFVTZOLLQWVTCPLZ3"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "1263056538"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "6938342"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}]}),
    )
}

/// `largest_blnd_to_usdc_pre_exploit` — ledger 61,341,361, tx idx 299, op 0,
/// event 1. BLND → USDC, 20754799754016 in / 836304108819 out.
fn largest_blnd_to_usdc_pre_exploit() -> RawSorobanEvent {
    comet_event(
        61_341_361,
        299,
        1,
        json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
        json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "CB3JAPDEIMA3OOSALUHLYRGM2QTXGVD3EASALPFMVEU2POLLULJBT2XN"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "20754799754016"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "836304108819"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}]}),
    )
}

/// `largest_usdc_to_blnd` — ledger 61,341,820, tx idx 348, op 0,
/// event 1. USDC → BLND, 200000000000 in / 5093532302262 out.
fn largest_usdc_to_blnd() -> RawSorobanEvent {
    comet_event(
        61_341_820,
        348,
        1,
        json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
        json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "CB3JAPDEIMA3OOSALUHLYRGM2QTXGVD3EASALPFMVEU2POLLULJBT2XN"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "200000000000"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "5093532302262"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}}]}),
    )
}

/// `dust_usdc_to_blnd` — ledger 51,500,460, tx idx 237, op 0,
/// event 0. USDC → BLND, 1000 in / 778906 out. Also tagged `first_swap`: the pool's first swap ever.
fn dust_usdc_to_blnd() -> RawSorobanEvent {
    comet_event(
        51_500_460,
        237,
        0,
        json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
        json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "GAZDHUMJW3QL6ITBWQRVR7FVB2H5ZTDPKSJLNUQOPMAX3G5ONIFOFSU5"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "1000"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "778906"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}}]}),
    )
}

/// `dust_blnd_to_usdc` — ledger 56,131,699, tx idx 226, op 0,
/// event 5. BLND → USDC, 152 in / 10 out. Also tagged `dust_out_blnd_to_usdc`.
fn dust_blnd_to_usdc() -> RawSorobanEvent {
    comet_event(
        56_131_699,
        226,
        5,
        json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
        json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "CCRUA3KR3QGCS5D5QDNITSRKQLIVHFBD4XUCZ3PC37PHN5U6BOKEGKTO"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "152"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "10"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}]}),
    )
}

/// `multi_swap_tx` — ledger 64,302,520, tx idx 151, op 0, events 10, 14, 19,
/// 23 and 27: five swaps against the pool in ONE transaction, in event order.
fn multi_swap_tx() -> Vec<RawSorobanEvent> {
    vec![
        // Event 10: USDC → BLND, 42270699754 in / 2606025902783 out.
        comet_event(
            64_302_520,
            151,
            10,
            json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
            json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "CAUA3I56LPLZUTWP43AF67DA3BSNSOLJ6M2CD46BGXCAEVI4IQMQ4EWX"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "42270699754"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "2606025902783"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}}]}),
        ),
        // Event 14: USDC → BLND, 66544251298 in / 961255886637 out.
        comet_event(
            64_302_520,
            151,
            14,
            json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
            json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "CAUA3I56LPLZUTWP43AF67DA3BSNSOLJ6M2CD46BGXCAEVI4IQMQ4EWX"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "66544251298"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "961255886637"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}}]}),
        ),
        // Event 19: BLND → USDC, 1416931973976 in / 81363391287 out.
        comet_event(
            64_302_520,
            151,
            19,
            json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
            json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "CAUA3I56LPLZUTWP43AF67DA3BSNSOLJ6M2CD46BGXCAEVI4IQMQ4EWX"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "1416931973976"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "81363391287"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}]}),
        ),
        // Event 23: BLND → USDC, 1889242631968 in / 25821262208 out.
        comet_event(
            64_302_520,
            151,
            23,
            json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
            json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "CAUA3I56LPLZUTWP43AF67DA3BSNSOLJ6M2CD46BGXCAEVI4IQMQ4EWX"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "1889242631968"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "25821262208"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}]}),
        ),
        // Event 27: BLND → USDC, 261107183476 in / 1520891050 out.
        comet_event(
            64_302_520,
            151,
            27,
            json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
            json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "CAUA3I56LPLZUTWP43AF67DA3BSNSOLJ6M2CD46BGXCAEVI4IQMQ4EWX"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "261107183476"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "1520891050"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}]}),
        ),
    ]
}

/// SYNTHETIC — no real non-swap payload was captured. The pool's liquidity
/// topics and its LP token's SEP-41 events in one transaction; the extractor
/// keys on topics only, so the data is a placeholder (a malformed one, which
/// must not matter).
fn liquidity_and_lp_events(ledger: u32, tx_index: u16) -> Vec<RawSorobanEvent> {
    let sym = |s: &str| json!({"type": "sym", "value": s});
    let addr = |s: &str| json!({"type": "address", "value": s});
    let data = json!({"type": "i128", "value": "1"});
    [
        json!([sym("POOL"), sym("deposit")]),
        json!([sym("POOL"), sym("join_pool")]),
        json!([sym("POOL"), sym("exit_pool")]),
        json!([sym("POOL"), sym("withdraw")]),
        json!([sym("transfer"), addr(COMET), addr(BLND_ISSUER)]),
        json!([sym("approve"), addr(COMET), addr(BLND_ISSUER)]),
        json!([sym("burn"), addr(COMET)]),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, topics)| comet_event(ledger, tx_index, i as u32, topics, data.clone()))
    .collect()
}

/// Run one real sample alone and return its single comet tick.
fn only_tick(event: RawSorobanEvent) -> TradeTick {
    let mut reg = comet_registry();
    let mut assets = seeded_assets();
    let ledger = event.ledger_sequence;
    let out = run(ledger, &[event], &mut reg, &mut assets);
    assert!(out.dispatch_errors.is_empty());
    assert!(out.unresolved.is_empty());
    assert_eq!(out.amm_ticks.len(), 1);
    let (source, tick) = out.amm_ticks.into_iter().next().unwrap();
    assert_eq!(source, "comet");
    assert_eq!((&tick.base, &tick.quote), (&blnd(), &usdc()));
    tick
}

/// AC2: BLND → USDC is priced USDC per BLND too — `amount_out / amount_in`.
#[test]
fn a_real_blnd_to_usdc_swap_is_priced_in_usdc_per_blnd() {
    let tick = only_tick(typical_blnd_to_usdc_recent());
    assert_eq!(tick.price, d(6_938_342) / d(1_263_056_538)); // ≈ 0.0054933
    assert_eq!(tick.volume_base, d(1_263_056_538));
    assert_eq!(tick.volume_quote, d(6_938_342));
    assert!(tick.price_forming);
    assert_eq!(tick.operation_index, 9);
}

/// AC2: the largest real swaps each way price direction-correctly.
#[test]
fn the_largest_real_swaps_price_in_usdc_per_blnd_both_ways() {
    let sell = only_tick(largest_blnd_to_usdc_pre_exploit());
    assert_eq!(sell.price, d(836_304_108_819) / d(20_754_799_754_016)); // ≈ 0.040294
    assert!(sell.price_forming);

    let buy = only_tick(largest_usdc_to_blnd());
    assert_eq!(buy.price, d(200_000_000_000) / d(5_093_532_302_262)); // ≈ 0.039265
    assert!(buy.price_forming);
}

/// AC2 / ADR 0287 §1: dust swaps tick with their volume but never price the
/// candle — the rounding bound fails on raw legs of 1000 and 10.
#[test]
fn real_dust_swaps_tick_without_forming_a_price() {
    let first = only_tick(dust_usdc_to_blnd());
    assert_eq!(first.price, d(1_000) / d(778_906));
    assert!(!first.price_forming);

    let tiny = only_tick(dust_blnd_to_usdc());
    assert_eq!(tiny.price, d(10) / d(152));
    assert!(!tiny.price_forming);
}

/// AC2: five swaps in one transaction are five ticks, one per event.
#[test]
fn five_real_swaps_in_one_transaction_are_five_ticks() {
    let mut reg = comet_registry();
    let mut assets = seeded_assets();
    let out = run(64_302_520, &multi_swap_tx(), &mut reg, &mut assets);

    assert!(out.dispatch_errors.is_empty());
    assert!(out.unresolved.is_empty());
    assert_eq!(out.amm_ticks.len(), 5);
    assert!(out.amm_ticks.iter().all(|(s, _)| *s == "comet"));
    let ops: Vec<u16> = out
        .amm_ticks
        .iter()
        .map(|(_, t)| t.operation_index)
        .collect();
    assert_eq!(ops, vec![10, 14, 19, 23, 27]);
    assert!(
        out.amm_ticks
            .iter()
            .all(|(_, t)| t.transaction_index == 151)
    );
    // First (USDC → BLND) and last (BLND → USDC), both USDC per BLND.
    assert_eq!(
        out.amm_ticks[0].1.price,
        d(42_270_699_754) / d(2_606_025_902_783)
    );
    assert_eq!(
        out.amm_ticks[4].1.price,
        d(1_520_891_050) / d(261_107_183_476)
    );
}

/// AC3: the pool's liquidity events and its LP token's SEP-41 events are not
/// trades — alone they give nothing, and beside a real swap only the swap
/// prices.
#[test]
fn liquidity_and_lp_token_events_neither_tick_nor_err() {
    let mut reg = comet_registry();
    let mut assets = seeded_assets();
    let out = run(
        64_570_597,
        &liquidity_and_lp_events(64_570_597, 627),
        &mut reg,
        &mut assets,
    );
    assert!(out.amm_ticks.is_empty());
    assert!(out.dispatch_errors.is_empty());
    assert!(out.unresolved.is_empty());

    let mut events = liquidity_and_lp_events(64_570_597, 627);
    events.push(typical_usdc_to_blnd_recent()); // event 21, after 0..=6
    let out = run(64_570_597, &events, &mut reg, &mut assets);
    assert_eq!(out.amm_ticks.len(), 1);
    assert!(out.dispatch_errors.is_empty());
}

/// D6: a registered Comet pool's malformed swap is a counted `comet` dispatch
/// error, not a silent loss — the swap filter knows Comet's `POOL/swap`.
#[test]
fn a_malformed_registered_comet_swap_is_a_counted_dispatch_error() {
    let mut malformed = typical_usdc_to_blnd_recent();
    malformed.data["value"]
        .as_array_mut()
        .unwrap()
        .retain(|e| e["key"]["value"] != "token_out");

    let mut reg = comet_registry();
    let mut assets = seeded_assets();
    let out = run(64_570_597, &[malformed], &mut reg, &mut assets);
    assert!(out.amm_ticks.is_empty());
    assert_eq!(out.dispatch_errors, vec![("comet", 1)]);

    // Liquidity traffic alone never counts, even with malformed data.
    let deposits: Vec<RawSorobanEvent> = liquidity_and_lp_events(64_570_597, 627)
        .into_iter()
        .take(1)
        .collect();
    let out = run(64_570_597, &deposits, &mut reg, &mut assets);
    assert!(out.dispatch_errors.is_empty());
}

/// SYNTHETIC amounts on the real `typical_usdc_to_blnd_recent` payload: an
/// i128 string that does not parse, or a negative one, is a counted `comet`
/// dispatch error — never a silent 0 and never a tick.
#[test]
fn an_unparseable_or_negative_comet_amount_is_a_counted_dispatch_error() {
    for (key, bad) in [
        ("token_amount_in", "not-a-number"),
        ("token_amount_out", "12x"),
        ("token_amount_in", "-3454229"),
        ("token_amount_out", "-621636466"),
    ] {
        let mut event = typical_usdc_to_blnd_recent();
        for entry in event.data["value"].as_array_mut().unwrap() {
            if entry["key"]["value"] == key {
                entry["value"]["value"] = json!(bad);
            }
        }

        let mut reg = comet_registry();
        let mut assets = seeded_assets();
        let out = run(64_570_597, &[event], &mut reg, &mut assets);
        assert!(out.amm_ticks.is_empty(), "{key} = {bad:?} must not tick");
        assert_eq!(
            out.dispatch_errors,
            vec![("comet", 1)],
            "{key} = {bad:?} must be a counted comet dispatch error"
        );
    }
}

/// AC5: with the pool NOT registered, its real swap is counted as a `comet`
/// trade the registry drops.
#[test]
fn an_unregistered_comet_swap_is_counted_as_comet() {
    let mut reg = Registries::new();
    let mut assets = seeded_assets();
    let out = run(
        64_570_597,
        &[typical_usdc_to_blnd_recent()],
        &mut reg,
        &mut assets,
    );
    assert!(out.amm_ticks.is_empty());
    assert_eq!(out.unregistered_pool_events, vec![("comet", 1)]);
    assert_eq!(out.unregistered_pool_contracts, vec![COMET.to_string()]);
}

/// AC6 (D3a): from an EMPTY `pool_registry`, the committed static list alone
/// routes the pool — no table row, no manual venue insert.
#[test]
fn the_static_list_routes_comet_from_an_empty_registry() {
    let mut reg = Registries::new();
    reg.merge_static_pools();
    let mut assets = seeded_assets();
    let out = run(
        64_570_597,
        &[typical_usdc_to_blnd_recent()],
        &mut reg,
        &mut assets,
    );
    assert_eq!(out.amm_ticks.len(), 1);
    let (source, tick) = &out.amm_ticks[0];
    assert_eq!(*source, "comet");
    assert_eq!((&tick.base, &tick.quote), (&blnd(), &usdc()));
    assert!(out.unregistered_pool_events.is_empty());
}

/// Blend v2.1's Comet pool (task 0334), wasm `d735c339…` (CometDEX v1.1.0).
const COMET_V1_1: &str = "CASYAKK3SHDN6S2E6IUOBVRXT3KKPVQCVQO4GNBDTVE6XZQQMW4UMIHE";

/// `v1_1_blnd_to_usdc` — pool `CASYAKK3…`, ledger 64,875,167, tx idx 199,
/// op 0, event 8. BLND → USDC, 16805005302 in / 94368304 out.
fn v1_1_blnd_to_usdc() -> RawSorobanEvent {
    RawSorobanEvent {
        contract_id: COMET_V1_1.to_string(),
        ..comet_event(
            64_875_167,
            199,
            8,
            json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
            json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": "GDCRZPZYBZ24RHRO3WBPJGFDL7NDFKUQBS3ZDB6YGBJB3TGKMFYBQ3LD"}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": "16805005302"}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": "94368304"}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}}]}),
        )
    }
}

/// Task 0334: the v1.1 pool's real swap, unregistered before and routed by
/// the static list after, from an empty registry.
#[test]
fn the_static_list_routes_the_blend_v2_1_comet_pool() {
    let mut assets = seeded_assets();
    let out = run(
        64_875_167,
        &[v1_1_blnd_to_usdc()],
        &mut Registries::new(),
        &mut assets,
    );
    assert!(out.amm_ticks.is_empty());
    assert_eq!(out.unregistered_pool_events, vec![("comet", 1)]);
    assert_eq!(
        out.unregistered_pool_contracts,
        vec![COMET_V1_1.to_string()]
    );

    let mut reg = Registries::new();
    reg.merge_static_pools();
    let out = run(64_875_167, &[v1_1_blnd_to_usdc()], &mut reg, &mut assets);
    assert_eq!(out.amm_ticks.len(), 1);
    let (source, tick) = &out.amm_ticks[0];
    assert_eq!(*source, "comet");
    assert_eq!((&tick.base, &tick.quote), (&blnd(), &usdc()));
    // BLND → USDC: price = USDC out / BLND in (≈ 0.005616).
    assert_eq!(tick.price, d(94_368_304) / d(16_805_005_302));
    assert_eq!(tick.volume_base, d(16_805_005_302));
    assert!(tick.price_forming);
    assert!(out.unregistered_pool_events.is_empty());
    assert!(out.dispatch_errors.is_empty());
}

// ---- task 0337: three more factory-less Comet v1.1 pools -------------------
//
// Fixtures from StellarExpert's copy of each event (`/contract/<id>/events`),
// decoded from its `bodyXdr`; ledger and tx application order are the event
// id's TOID (`ledger << 32 | tx << 12 | op`), event index its suffix.

const BLNT_USDC: &str = "CDQMIKIHKA5UY3FQJ2UDTLNX3GOPD3GPNZWSPTOBVOHEUMQJQXOPC7Y3";
const BLNT_XLM: &str = "CA5HX3D47PQZWC2E7VWXCKH4534P2NOJEOIXAWU47MI4QV2FQUYY3FLC";
const BLND_USDC_SMALL: &str = "CCNDNFA7LEVUJRYWTSWVAA2ESZAKFO6MYOPI6E5AZT3V2UBP2MNV7QFZ";
/// BLNT's issuer; its SAC is `CCBNWP5L…`.
const BLNT_ISSUER: &str = "GAP2ACGH3O6TRFAERGM4USJ6XWU3U2BFIK224JPV6NI3CUNEEEGTBTFB";

fn blnt() -> AssetIdentity {
    AssetIdentity::Credit {
        code: "BLNT".to_string(),
        issuer: BLNT_ISSUER.to_string(),
    }
}

/// One `POOL/swap` of `pool`, its payload map built from the five fields.
fn v1_1_swap(
    pool: &str,
    (ledger, tx_index, event_index): (u32, u16, u32),
    caller: &str,
    (token_in, amount_in): (&str, &str),
    (token_out, amount_out): (&str, &str),
) -> RawSorobanEvent {
    RawSorobanEvent {
        contract_id: pool.to_string(),
        ..comet_event(
            ledger,
            tx_index,
            event_index,
            json!([{"type": "sym", "value": "POOL"}, {"type": "sym", "value": "swap"}]),
            json!({"type": "map", "value": [{"key": {"type": "sym", "value": "caller"}, "value": {"type": "address", "value": caller}}, {"key": {"type": "sym", "value": "token_amount_in"}, "value": {"type": "i128", "value": amount_in}}, {"key": {"type": "sym", "value": "token_amount_out"}, "value": {"type": "i128", "value": amount_out}}, {"key": {"type": "sym", "value": "token_in"}, "value": {"type": "address", "value": token_in}}, {"key": {"type": "sym", "value": "token_out"}, "value": {"type": "address", "value": token_out}}]}),
        )
    }
}

const USDC_SAC: &str = "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75";
const BLND_SAC: &str = "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY";
const BLNT_SAC: &str = "CCBNWP5L7ZSXA3DBWCMFAVKFVRW3ZHWO7B5J557W42O64DDZNPJP7VDS";
const XLM_SAC: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
/// The router behind every BLNT swap so far (USDC → BLNT → XLM in one tx).
const BLNT_ROUTER: &str = "CB2DT56GJPXZG7SXGPJOW3F3WNF6RQ7RT5HZLAD2XW4TJ7NUN2YWUZLW";

/// Event `279761757207027713-0001` — `CDQMIKIH…`, ledger 65,137,110, tx 262,
/// event 1. USDC → BLNT, 500000000 in / 103337956008 out.
fn blnt_usdc_swap() -> RawSorobanEvent {
    v1_1_swap(
        BLNT_USDC,
        (65_137_110, 262, 1),
        BLNT_ROUTER,
        (USDC_SAC, "500000000"),
        (BLNT_SAC, "103337956008"),
    )
}

/// Event `279761757207027713-0005` — `CA5HX3D4…`, the same transaction's
/// second hop. BLNT → XLM, 103337956008 in / 3711330029 out.
fn blnt_xlm_swap() -> RawSorobanEvent {
    v1_1_swap(
        BLNT_XLM,
        (65_137_110, 262, 5),
        BLNT_ROUTER,
        (BLNT_SAC, "103337956008"),
        (XLM_SAC, "3711330029"),
    )
}

/// Event `279599050962505729-0009` — `CCNDNFA7…`, ledger 65,099,227, tx 641,
/// event 9, the pool's first swap. BLND → USDC, 42983166 in / 231757 out.
fn blnd_usdc_small_swap() -> RawSorobanEvent {
    v1_1_swap(
        BLND_USDC_SMALL,
        (65_099_227, 641, 9),
        "CDYYYMHP3RKMPDY7VARR5J65RBLUFBHMARTHZM5NZ774D4CMOV7VHRR2",
        (BLND_SAC, "42983166"),
        (USDC_SAC, "231757"),
    )
}

/// Run `event` from an empty registry, then from one with the static list
/// merged: unregistered before, exactly one `"comet"` tick after.
fn routed_by_the_static_list(event: RawSorobanEvent) -> TradeTick {
    let mut assets = AssetRegistry::from_existing(vec![usdc(), blnd(), blnt()]);
    let ledger = event.ledger_sequence;
    let pool = event.contract_id.clone();

    let out = run(
        ledger,
        std::slice::from_ref(&event),
        &mut Registries::new(),
        &mut assets,
    );
    assert!(out.amm_ticks.is_empty());
    assert_eq!(out.unregistered_pool_events, vec![("comet", 1)]);
    assert_eq!(out.unregistered_pool_contracts, vec![pool]);

    let mut reg = Registries::new();
    reg.merge_static_pools();
    let out = run(ledger, &[event], &mut reg, &mut assets);
    assert!(out.unregistered_pool_events.is_empty());
    assert!(out.dispatch_errors.is_empty());
    assert!(out.unresolved.is_empty());
    assert_eq!(out.amm_ticks.len(), 1);
    let (source, tick) = out.amm_ticks.into_iter().next().unwrap();
    assert_eq!(source, "comet");
    assert!(tick.price_forming);
    tick
}

#[test]
fn the_static_list_routes_the_blnt_usdc_pool() {
    let tick = routed_by_the_static_list(blnt_usdc_swap());
    assert_eq!((&tick.base, &tick.quote), (&blnt(), &usdc()));
    // USDC → BLNT: price = USDC in / BLNT out (≈ 0.00484).
    assert_eq!(tick.price, d(500_000_000) / d(103_337_956_008));
    assert_eq!(tick.volume_base, d(103_337_956_008));
}

#[test]
fn the_static_list_routes_the_blnt_xlm_pool() {
    let tick = routed_by_the_static_list(blnt_xlm_swap());
    assert_eq!((&tick.base, &tick.quote), (&blnt(), &AssetIdentity::Native));
    // BLNT → XLM: price = XLM out / BLNT in (≈ 0.0359).
    assert_eq!(tick.price, d(3_711_330_029) / d(103_337_956_008));
    assert_eq!(tick.volume_base, d(103_337_956_008));
}

#[test]
fn the_static_list_routes_the_small_blnd_usdc_pool() {
    let tick = routed_by_the_static_list(blnd_usdc_small_swap());
    assert_eq!((&tick.base, &tick.quote), (&blnd(), &usdc()));
    // BLND → USDC: price = USDC out / BLND in (≈ 0.00539).
    assert_eq!(tick.price, d(231_757) / d(42_983_166));
    assert_eq!(tick.volume_base, d(42_983_166));
}
