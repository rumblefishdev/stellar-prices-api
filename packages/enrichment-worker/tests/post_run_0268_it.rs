//! **The operator's after-check for task 0268 — not a CI test.**
//!
//! Every `#[tokio::test]` here is `#[ignore]`d and reads a REAL database named
//! by `CLICKHOUSE_URL` / `CH_DATABASE`, not a scratch one. They are expected to
//! FAIL until the production re-enrichment pass has run; that is their purpose.
//! Run them after the pass, from the runbook:
//!
//!   docs/runbooks/repair-coarse-usd-values.md, Appendix B, "After"
//!
//!   CLICKHOUSE_URL=... CH_DATABASE=prices \
//!     cargo test -p enrichment-worker --test post_run_0268_it -- --ignored
//!
//! The JUDGEMENT — what counts as repaired — is a pure function ([`judge`]) with
//! plain unit tests that CI does run, so the harness cannot rot silently the way
//! its first version did (review WR-01/WR-02: it could not deserialise its own
//! query, and its tolerance band admitted the very value it exists to reject).
//!
//! ## The falsifier
//!
//! USDC closed at **0.9681** on 2023-03-11 (the SVB weekend depeg). Every
//! XLM/USDC candle from that day was stored as `close × $1.00` by the enrichment
//! peg tier, so `native`'s USD close read ~3.19% HIGH. After the pass it must
//! read BELOW par by more than any grain's blending can explain, on every
//! granularity that holds deep history.
//!
//! ## Why the bound is directional, not a band around 0.9681
//!
//! A band `0.9681 ± 0.04` contains 1.0. Par is then rejected only by an exact
//! equality test, which any PARTIAL repair defeats: reach 0.1% of the rows and
//! the volume-weighted rate lands at ~0.99997 — not exactly 1.0, comfortably
//! inside the band — and an essentially unrepaired table reports green. So each
//! grain gets a ceiling the pass must get UNDER, chosen from what its bucket
//! blends: a day-or-shorter bucket is the depeg day itself; a weekly bucket
//! blends six recovered days; a monthly one blends thirty.
//!
//! ## Why they fail loudly rather than skip
//!
//! A missing row is not a pass. If `native` has no candle covering 2023-03-11 in
//! a given table, that is either a retention boundary the operator must know
//! about or a repair that silently dropped rows — both are findings. And a row
//! at `close_usd = 0` is the task 0182 outcome (reset, never refilled), which the
//! rate alone cannot see because a zero contributes nothing to the sum — so the
//! zeros are counted separately and any of them is a failure.
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

/// USDC's measured close on 2023-03-11. The number this task exists for; the
/// ceilings below are derived from it and from the intraday low (~0.88).
const DEPEG_RATE: f64 = 0.9681;

/// Below this the table is not "repaired", it is broken: nothing in the
/// depeg weekend traded USDC under ~0.88, so a bucket-average under 0.85 means a
/// wrong rate (or a wrong column) was applied, not the measured one.
const SANITY_FLOOR: f64 = 0.85;

/// One granularity's expectation. `span` is the bucket width the lookback
/// covers; `max_rate` is the ceiling the post-repair implied rate must be UNDER
/// — strictly below par by more than the grain's own blending can explain.
struct Grain {
    table: &'static str,
    span: u32,
    max_rate: f64,
}

/// The granularities that hold deep history. `_15m` has a 30-day retention and
/// `_1m` was largely dropped by the cleanup worker for 2025-02 → 2026-02, so
/// neither can carry a 2023 row; the repair driver refuses `_1m` outright.
///
/// Table and span travel together (review IN-07): two arrays zipped by position
/// silently mis-pair when one is reordered.
const DEEP: [Grain; 5] = [
    // A daily-or-shorter bucket IS the depeg day: the daily close was 0.9681
    // and the intraday buckets average lower still.
    Grain {
        table: "price_ohlcv_1h",
        span: 86_400,
        max_rate: 0.99,
    },
    Grain {
        table: "price_ohlcv_4h",
        span: 86_400,
        max_rate: 0.99,
    },
    Grain {
        table: "price_ohlcv_1d",
        span: 86_400,
        max_rate: 0.99,
    },
    // A weekly bucket blends six recovered days at ~1.0 with one at 0.9681.
    Grain {
        table: "price_ohlcv_1w",
        span: 7 * 86_400,
        max_rate: 0.999,
    },
    // A monthly bucket blends thirty; the honest value is ~0.998.
    Grain {
        table: "price_ohlcv_1M",
        span: 31 * 86_400,
        max_rate: 0.9995,
    },
];

/// What one table says about `native`/USDC over a span. `rate` is `None` when
/// no candle in the span has a positive `close_usd` — distinct from `rows == 0`,
/// which is "no candle at all".
#[derive(clickhouse::Row, serde::Deserialize, Debug, PartialEq)]
struct Measurement {
    rate: Option<f64>,
    rows: u64,
    zeros: u64,
}

fn client() -> Client {
    Client::default()
        .with_url(
            std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".into()),
        )
        .with_database(std::env::var("CH_DATABASE").unwrap_or_else(|_| "prices".into()))
}

/// The volume-weighted implied rate `close_usd / close` for `native`'s
/// USDC-quoted candles in `[from, from + span)`, in `table`, plus the row
/// count and the count of rows at `close_usd = 0`.
///
/// Volume-weighted rather than a bare average because a bucket may merge several
/// sources; `argMax`-ing one of them would make the answer depend on which venue
/// happened to trade last. The `close > 0` guard keeps a zero close out of the
/// division; the `close_usd > 0` condition is INSIDE the sums, not in the
/// `WHERE`, so a zeroed row still counts toward `rows` and `zeros`.
///
/// One ungrouped aggregate always returns exactly one row — over an empty match
/// set it returns `NULL`s and zero counts, never zero rows — so this is
/// `fetch_one` into a typed row with a `Nullable` column, and emptiness is read
/// from `rows`, not from the absence of a result (review WR-01).
async fn measure(ch: &Client, table: &str, from: u32, span: u32) -> Measurement {
    ch.query(&format!(
        "SELECT \
             sumIf(toFloat64(close_usd) * toFloat64(volume_base), close_usd > 0) \
               / nullIf(sumIf(toFloat64(close) * toFloat64(volume_base), close_usd > 0), 0) AS rate, \
             count() AS rows, \
             countIf(close_usd = 0) AS zeros \
         FROM {table} AS p FINAL \
         INNER JOIN ( SELECT asset_id FROM assets FINAL \
                      WHERE asset_code = 'XLM' AND issuer_address = '' \
                        AND contract_address = '' ) AS x ON x.asset_id = p.asset_id \
         INNER JOIN ( SELECT asset_id FROM assets FINAL \
                      WHERE asset_code = 'USDC' AND issuer_address = ? \
                        AND contract_address = '' ) AS u ON u.asset_id = p.quote_asset_id \
         WHERE p.timestamp >= toDateTime(?) AND p.timestamp < toDateTime(?) \
           AND p.close > 0"
    ))
    .bind(prices_clickhouse::USDC_ISSUER)
    .bind(from)
    .bind(from + span)
    .fetch_one::<Measurement>()
    .await
    .unwrap()
}

/// 🔑 THE JUDGEMENT, pure so CI can test it. `None` is a pass; `Some` is the
/// finding, worded for the operator.
fn judge(g: &Grain, m: &Measurement) -> Option<String> {
    let t = g.table;
    if m.rows == 0 {
        return Some(format!(
            "{t}: NO native/USDC candle covering 2023-03-11 at all. That is a finding, \
             not a pass — either a retention boundary or a repair that dropped rows."
        ));
    }
    if m.zeros > 0 {
        return Some(format!(
            "{t}: {} of {} candles sit at close_usd = 0 — reset and never refilled, the \
             task 0182 outcome. Roll the table back from its FREEZE snapshot.",
            m.zeros, m.rows
        ));
    }
    let Some(rate) = m.rate else {
        return Some(format!(
            "{t}: {} candles but none with a positive close_usd to measure.",
            m.rows
        ));
    };
    if rate >= g.max_rate {
        return Some(format!(
            "{t}: implied rate {rate:.6} >= {} — indistinguishable from par. USDC closed \
             {DEPEG_RATE} on 2023-03-11; the pass did not reach this table (an exactly-1.0 \
             rate is the untouched peg value; a value a few bps under it is a PARTIAL pass).",
            g.max_rate
        ));
    }
    if rate < SANITY_FLOOR {
        return Some(format!(
            "{t}: implied rate {rate:.6} < {SANITY_FLOOR} — nothing in the depeg weekend \
             traded that low; a wrong rate or a wrong column was applied."
        ));
    }
    None
}

/// 🔑 **THE FALSIFIER FOR THE WHOLE TASK.** `native` on 2023-03-11 must publish a
/// USD close BELOW its USDC-denominated close by more than the grain's blending
/// can explain, on every granularity that holds deep history, with no row left
/// at zero. Acceptance criteria 1 and 2.
#[tokio::test]
#[ignore = "operator after-check: run against prod AFTER the 0268 re-enrichment pass"]
async fn native_on_the_depeg_day_is_priced_below_its_usdc_close() {
    let ch = client();
    let mut failures = Vec::new();

    for g in &DEEP {
        // A weekly/monthly bucket containing 2023-03-11 opens BEFORE it, so look
        // back a full bucket for those grains.
        let from = DEPEG_DAY.saturating_sub(g.span - 86_400);
        let m = measure(&ch, g.table, from, g.span).await;
        if let Some(f) = judge(g, &m) {
            failures.push(f);
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
    let m = measure(&ch, "price_ohlcv_1d", RECOVERED_DAY, 86_400).await;
    assert!(
        m.rows > 0,
        "no native/USDC daily candle for 2023-03-15 — that is a finding, not a pass"
    );
    assert_eq!(m.zeros, 0, "a zeroed candle on 2023-03-15: {m:?}");
    let rate = m.rate.expect("a priced candle on 2023-03-15");
    assert!(
        (rate - 1.0).abs() < 0.005,
        "2023-03-15 implied rate {rate:.6}, expected ~1.0 (±0.005). A table priced \
         uniformly low would pass the depeg check and fail here — which is what this \
         control exists to catch."
    );
}

// ---- the judgement, tested without a ClickHouse (these DO run in CI) --------

fn priced(rate: f64, rows: u64) -> Measurement {
    Measurement {
        rate: Some(rate),
        rows,
        zeros: 0,
    }
}

fn grain(table: &str) -> &'static Grain {
    DEEP.iter().find(|g| g.table == table).unwrap()
}

/// WR-02's finding, as a test: par must fail on EVERY grain, by the ceiling
/// alone, with no exact-equality test to defeat.
#[test]
fn par_is_rejected_on_every_grain() {
    for g in &DEEP {
        assert!(
            g.max_rate < 1.0,
            "{}: the ceiling itself admits par",
            g.table
        );
        assert!(
            judge(g, &priced(1.0, 1_000)).is_some(),
            "{}: par passed",
            g.table
        );
    }
}

/// A partial pass — 0.1% of the rows repaired — lands a few bps under par and
/// must ALSO fail. This is the case the old `±0.04` band let through.
#[test]
fn a_partial_repair_a_few_bps_under_par_is_rejected() {
    for g in &DEEP {
        let f = judge(g, &priced(0.99997, 654_291)).unwrap();
        assert!(f.contains("PARTIAL"), "{}: {f}", g.table);
    }
}

/// The honest post-repair values pass: the depeg day on the day-or-shorter
/// grains, a six-days-recovered blend on the week, thirty on the month.
#[test]
fn the_measured_depeg_passes_on_each_grain_at_its_own_blend() {
    for t in ["price_ohlcv_1h", "price_ohlcv_4h", "price_ohlcv_1d"] {
        assert_eq!(judge(grain(t), &priced(DEPEG_RATE, 24)), None, "{t}");
        // Intraday buckets average lower than the close; still a pass.
        assert_eq!(judge(grain(t), &priced(0.93, 24)), None, "{t}");
    }
    assert_eq!(judge(grain("price_ohlcv_1w"), &priced(0.995, 1)), None);
    assert_eq!(judge(grain("price_ohlcv_1M"), &priced(0.998, 1)), None);
    // And the blend ceilings are grain-specific, not one number for five: a
    // monthly-shaped value on a daily grain is an unrepaired daily table.
    assert!(judge(grain("price_ohlcv_1d"), &priced(0.998, 1)).is_some());
}

/// No candle at all, a zeroed candle, and no priced candle are three different
/// findings, each named, none a pass.
#[test]
fn missing_zeroed_and_unpriced_rows_are_findings_not_passes() {
    let g = grain("price_ohlcv_1d");
    let none = judge(
        g,
        &Measurement {
            rate: None,
            rows: 0,
            zeros: 0,
        },
    )
    .unwrap();
    assert!(none.contains("NO native/USDC candle"), "{none}");

    let zeroed = judge(
        g,
        &Measurement {
            rate: Some(DEPEG_RATE),
            rows: 24,
            zeros: 3,
        },
    )
    .unwrap();
    assert!(
        zeroed.contains("close_usd = 0") && zeroed.contains("0182"),
        "{zeroed}"
    );

    let unpriced = judge(
        g,
        &Measurement {
            rate: None,
            rows: 24,
            zeros: 0,
        },
    )
    .unwrap();
    assert!(
        unpriced.contains("none with a positive close_usd"),
        "{unpriced}"
    );
}

/// A rate below anything USDC traded at is a broken table, not a repaired one.
#[test]
fn a_rate_below_the_depeg_weekends_low_is_rejected() {
    let f = judge(grain("price_ohlcv_1h"), &priced(0.5, 24)).unwrap();
    assert!(f.contains("wrong rate"), "{f}");
}

/// The lookback arithmetic: a weekly or monthly bucket that CONTAINS the depeg
/// day opens before it, and the span reaches back exactly one bucket less a day
/// so the window `[from, from + span)` still ends after the depeg day.
#[test]
fn the_lookback_window_covers_the_depeg_day_on_every_grain() {
    for g in &DEEP {
        let from = DEPEG_DAY.saturating_sub(g.span - 86_400);
        assert!(from <= DEPEG_DAY, "{}", g.table);
        assert!(from + g.span > DEPEG_DAY, "{}", g.table);
    }
}
