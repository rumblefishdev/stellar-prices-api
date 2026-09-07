//! **The operator's after-check for task 0268 — not a CI test.**
//!
//! Every test here is `#[ignore]`d and reads a REAL database named by
//! `CLICKHOUSE_URL` / `CH_DATABASE`, not a scratch one. They are expected to
//! FAIL until the production re-enrichment pass has run; that is their purpose.
//! Run them after the pass, from the runbook:
//!
//!   docs/runbooks/repair-coarse-usd-values.md, Appendix B, "After"
//!
//!   CLICKHOUSE_URL=... CH_DATABASE=prices \
//!     cargo test -p enrichment-worker --test post_run_0268_it -- --ignored
//!
//! ## The falsifier
//!
//! USDC closed at **0.9681** on 2023-03-11 (the SVB weekend depeg). Every
//! XLM/USDC candle from that day was stored as `close × $1.00` by the enrichment
//! peg tier, so `native`'s USD close read ~3.19% HIGH. After the pass it must
//! read ~3% BELOW its USDC-denominated close, on every granularity that holds
//! deep history.
//!
//! ## Why they fail loudly rather than skip
//!
//! A missing row is not a pass. If `native` has no candle for 2023-03-11 in a
//! given table, that is either a retention boundary the operator must know about
//! or a repair that silently dropped rows — both are findings. `fetch_optional`
//! plus an explicit panic says which; a `return` on empty would turn the whole
//! check into a green no-op, which is the exact shape of the false all-clear
//! that hid task 0182 for a month.
//!
//! ## Why a second date
//!
//! `usdc_is_back_at_par_a_few_days_later` exists so the first test cannot be
//! satisfied by pricing EVERYTHING ~3% low — a uniformly scaled table would pass
//! the depeg check and fail this one.

use clickhouse::Client;

/// 2023-03-11 00:00:00 UTC — the depeg day.
const DEPEG_DAY: u32 = 1_678_492_800;
/// 2023-03-15 00:00:00 UTC — recovered.
const RECOVERED_DAY: u32 = 1_678_838_400;

/// USDC's measured close on 2023-03-11.
const DEPEG_RATE: f64 = 0.9681;

/// The granularities that hold deep history. `_15m` has a 30-day retention and
/// `_1m` was largely dropped by the cleanup worker for 2025-02 → 2026-02, so
/// neither can carry a 2023 row; the repair driver refuses `_1m` outright.
const DEEP_TABLES: [&str; 5] = [
    "price_ohlcv_1h",
    "price_ohlcv_4h",
    "price_ohlcv_1d",
    "price_ohlcv_1w",
    "price_ohlcv_1M",
];

fn client() -> Client {
    Client::default()
        .with_url(
            std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".into()),
        )
        .with_database(std::env::var("CH_DATABASE").unwrap_or_else(|_| "prices".into()))
}

/// The volume-weighted implied rate `close_usd / close` for `native`'s
/// USDC-quoted candles covering `day`, in `table`. `None` when the day holds no
/// such candle at all.
///
/// Volume-weighted rather than a bare average because a bucket may merge several
/// sources; `argMax`-ing one of them would make the answer depend on which venue
/// happened to trade last. The `close > 0` guard keeps a zero close out of the
/// division.
async fn implied_rate(ch: &Client, table: &str, day: u32, span: u32) -> Option<f64> {
    ch.query(&format!(
        "SELECT sum(toFloat64(close_usd) * toFloat64(volume_base)) \
              / nullIf(sum(toFloat64(close) * toFloat64(volume_base)), 0) \
         FROM {table} AS p FINAL \
         INNER JOIN ( SELECT asset_id FROM assets FINAL \
                      WHERE asset_code = 'XLM' AND issuer_address = '' \
                        AND contract_address = '' ) AS x ON x.asset_id = p.asset_id \
         INNER JOIN ( SELECT asset_id FROM assets FINAL \
                      WHERE asset_code = 'USDC' AND issuer_address = ? \
                        AND contract_address = '' ) AS u ON u.asset_id = p.quote_asset_id \
         WHERE p.timestamp >= toDateTime(?) AND p.timestamp < toDateTime(?) \
           AND p.close > 0 AND p.close_usd > 0"
    ))
    .bind(prices_clickhouse::USDC_ISSUER)
    .bind(day)
    .bind(day + span)
    .fetch_optional::<f64>()
    .await
    .unwrap()
}

/// 🔑 **THE FALSIFIER FOR THE WHOLE TASK.** `native` on 2023-03-11 must publish a
/// USD close ~3% BELOW its USDC-denominated close, on every granularity that
/// holds deep history. Acceptance criterion 2.
///
/// An implied rate of exactly 1.0 means the peg tier still owns those rows and
/// the pass did not reach this table.
#[tokio::test]
#[ignore = "operator after-check: run against prod AFTER the 0268 re-enrichment pass"]
async fn native_on_the_depeg_day_is_priced_below_its_usdc_close() {
    let ch = client();
    // A monthly bucket starting 2023-03-01 covers the depeg day, so widen the
    // span per grain rather than pinning midnight-to-midnight.
    let spans = [86_400u32, 86_400, 86_400, 7 * 86_400, 31 * 86_400];
    let mut failures = Vec::new();

    for (table, span) in DEEP_TABLES.iter().zip(spans) {
        // A weekly/monthly bucket containing 2023-03-11 opens BEFORE it, so look
        // back a full bucket for those grains.
        let from = DEPEG_DAY.saturating_sub(span - 86_400);
        let Some(rate) = implied_rate(&ch, table, from, span).await else {
            failures.push(format!(
                "{table}: NO native/USDC candle covering 2023-03-11 at all. That is a \
                 finding, not a pass — either a retention boundary or a repair that \
                 dropped rows."
            ));
            continue;
        };
        if (rate - 1.0).abs() < 1e-9 {
            failures.push(format!(
                "{table}: implied rate is exactly 1.0 — the peg tier still owns these \
                 rows and the 0268 pass did not reach this table."
            ));
            continue;
        }
        // Tolerance: the daily close was 0.9681, but a bucket that merges an
        // intraday span (which fell as low as ~0.88) averages lower, and the
        // weekly/monthly buckets blend in recovered days above it. ±4 cents is
        // wide enough for all five grains and far too narrow to admit par.
        if (rate - DEPEG_RATE).abs() > 0.04 {
            failures.push(format!(
                "{table}: implied rate {rate:.6}, expected ~{DEPEG_RATE} (±0.04)"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "USDC closed at 0.9681 on 2023-03-11 and native must be priced from that \
         measurement, not from $1:\n  {}",
        failures.join("\n  ")
    );
}

/// The control. Four days later USDC was back within a few bps of par, so the
/// implied rate must be ~1.0 — which a table priced uniformly ~3% low could not
/// satisfy while also passing the test above.
#[tokio::test]
#[ignore = "operator after-check: run against prod AFTER the 0268 re-enrichment pass"]
async fn usdc_is_back_at_par_a_few_days_later() {
    let ch = client();
    let Some(rate) = implied_rate(&ch, "price_ohlcv_1d", RECOVERED_DAY, 86_400).await else {
        panic!("no native/USDC daily candle for 2023-03-15 — that is a finding, not a pass");
    };
    assert!(
        (rate - 1.0).abs() < 0.005,
        "2023-03-15 implied rate {rate:.6}, expected ~1.0 (±0.005). A table priced \
         uniformly low would pass the depeg check and fail here — which is what this \
         control exists to catch."
    );
}
