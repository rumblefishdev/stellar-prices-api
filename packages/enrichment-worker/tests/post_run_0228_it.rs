//! **The operator's after-check for task 0228 — not a CI test.**
//!
//! Every `#[tokio::test]` here is `#[ignore]`d and reads a REAL database named
//! by `CLICKHOUSE_URL` / `CH_DATABASE`, not a scratch one. They are expected to
//! FAIL until the production re-enrichment campaign has run; that is their
//! purpose. Run them after the campaign, from the runbook:
//!
//!   docs/runbooks/repair-coarse-usd-values.md, Appendix C, "After — the falsifier"
//!
//!   CLICKHOUSE_URL=... CH_DATABASE=prices \
//!   POST_RUN_0228_VERSION_BEFORE_1W=<n> POST_RUN_0228_VERSION_BEFORE_1M=<n> \
//!     cargo test -p enrichment-worker --test post_run_0228_it -- --ignored
//!
//! Against production over mTLS, replace `CLICKHOUSE_URL` with the four
//! variables `coarse-repair --transport hetzner` reads (`CH_DOMAIN`,
//! `MTLS_CERT_PATH`, `MTLS_KEY_PATH`, `MTLS_CA_PATH`) and add
//! `--features aws-mtls` to the `cargo test`.
//!
//! The JUDGEMENT — what counts as repaired — is a pure function ([`judge`]) with
//! plain unit tests that CI does run, so the harness cannot rot silently. Same
//! shape as `post_run_0268_it.rs`, which is its parent.
//!
//! ## The falsifier
//!
//! Task 0228's defect is that the pivot tier priced an XLM-quoted candle as
//! `close × vwap(XLM/USDC)` — a number denominated in **USDC**, stored in a
//! dollar column. USDC closed at **0.9681** on 2023-03-11, so every such candle
//! read ~3.19% HIGH.
//!
//! ## Why the check is a RATIO, not an absolute price
//!
//! An XLM-quoted candle's implied rate `close_usd / close` is XLM's USD price —
//! a number with no fixed expected value, unlike 0268's USDC rate which sits
//! near 1. So this falsifier measures it against the thing the pivot actually
//! multiplied by: the SAME table's XLM/USDC volume-weighted close for the SAME
//! bucket. The quotient
//!
//!   close_usd / (close × vwap(XLM/USDC, same bucket))
//!
//! is the USDC/USD factor the stored value carries — **1.0 exactly if the
//! campaign never reached this table**, 0.9681 once it has. That makes the
//! ceiling self-calibrating: it needs no hand-typed XLM price, and it cannot
//! drift when the market data behind it is re-measured.
//!
//! The MEDIAN of the per-bucket quotients, not a volume-weighted mean: the
//! quotient is already a pure ratio, and a median is what survives one outlier
//! bucket whose reference market was thin.
//!
//! ## Two kinds of check, because ONE rate prices a bucket
//!
//! [`pivot_sql`](enrichment_worker::ch_enrich) resolves ONE USDC/USD rate at the
//! bucket's END and prices the whole bucket with it, so what a grain can show
//! depends only on where its bucket ends:
//!
//! * **`_15m`, `_1h`, `_4h`, `_1d`** — every bucket of 2023-03-11 ends at or
//!   before 2023-03-12 00:00, so each resolves to the 03-11 rate. The ratio IS
//!   the evidence: it must sit UNDER a ceiling par cannot satisfy
//!   ([`Check::RatioUnder`]).
//! * **`_1w`, `_1M`** — the bucket holding 03-11 ends on 03-13 and 04-01, when
//!   USDC was back at (or within bps of) par. A correctly repaired row therefore
//!   reads ~1.0 by construction and NO ceiling can separate "repaired" from
//!   "untouched". Those grains are checked by the MECHANISM instead: the
//!   bucket's `max(version)` must have moved past the value the runbook's
//!   "before" step recorded, and no row may sit at zero with volume.
//!
//! ⚠️ **There is no par-signature clause here, and that is the whole of task
//! 0228's D-06.** 0268's mechanism grains could also ask whether `close_usd =
//! close` survived, because the peg tier stamps that signature on every row it
//! writes. A pivoted row never carried one — it was `close × vwap`, which equals
//! `close` only by coincidence — so there is nothing to look for, and the
//! campaign is value-idempotent rather than a fixed point. The version is the
//! only mechanical evidence such a bucket can offer.
//!
//! ## Why the ceiling is directional, not a band around 0.9681
//!
//! A band `0.9681 ± 0.04` contains 1.0, so par is rejected only by an exact
//! equality test — which any PARTIAL repair defeats: reach a fraction of the
//! rows and the median lands at 1.0 anyway, or a hair under it, comfortably
//! inside the band. So the grains that can show the depeg get a ceiling the
//! campaign must get UNDER.
//!
//! ## Why they fail loudly rather than skip
//!
//! A missing row is not a pass. A row at `close_usd = 0` WITH volume is the task
//! 0182 outcome — reset, never refilled — which the ratio cannot see because a
//! zero is excluded from it; those are counted separately and any of them is a
//! failure. A row at `close_usd = 0` WITHOUT volume is the permanent volume-zero
//! floor every tier shares; it is context, never a failure.
//!
//! ## Why a second date
//!
//! `the_pivot_leg_carries_no_discount_once_usdc_is_back_at_par` exists so the
//! first test cannot be satisfied by pricing EVERYTHING ~3% low — a uniformly
//! scaled table would pass the depeg check and fail this one.

use clickhouse::Client;

/// 2023-03-11 00:00:00 UTC — the depeg day.
const DEPEG_DAY: u32 = 1_678_492_800;
/// 2023-03-12 00:00:00 UTC — the end of every `_1d`-or-shorter bucket of it.
const DEPEG_DAY_END: u32 = DEPEG_DAY + 86_400;
/// 2023-03-06 00:00:00 UTC (a Monday) — the `_1w` bucket that contains the depeg
/// day — and its end, 2023-03-13.
const DEPEG_WEEK_START: u32 = 1_678_060_800;
const DEPEG_WEEK_END: u32 = DEPEG_WEEK_START + 7 * 86_400;
/// 2023-03-01 00:00:00 UTC — the `_1M` bucket that contains the depeg day — and
/// its end, 2023-04-01.
const DEPEG_MONTH_START: u32 = 1_677_628_800;
const DEPEG_MONTH_END: u32 = 1_680_307_200;
/// 2023-03-15 00:00:00 UTC — recovered.
const RECOVERED_DAY: u32 = 1_678_838_400;

/// USDC's measured close on 2023-03-11. The number this campaign exists for.
const DEPEG_RATE: f64 = 0.9681;

/// Below this the table is not "repaired", it is broken: nothing in the depeg
/// weekend traded USDC under ~0.88, so a bucket ratio under 0.85 means a wrong
/// rate (or a wrong column) reached the pivot, not the measured one.
const SANITY_FLOOR: f64 = 0.85;

/// The ceiling for the grains whose bucket ends inside the depeg. Par cannot
/// satisfy it; 0.9681 can, with room.
const DEPEG_CEILING: f64 = 0.99;

/// How a grain is judged. See the module doc for why there are two.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Check {
    /// The bucket END is still inside the depeg, so the carried USDC/USD factor
    /// IS that day's and must sit strictly under the ceiling.
    RatioUnder(f64),
    /// The bucket END is past the recovery. The carried factor is ~1.0 by
    /// construction, so par is not evidence of an unrepaired table. Check that
    /// the pass TOUCHED the bucket instead.
    Mechanism,
}

/// One granularity's expectation over the bucket(s) `[from, to)` that hold the
/// depeg day. Table and window travel together, never as parallel arrays: arrays
/// zipped by position silently mis-pair when one is reordered.
struct Grain {
    table: &'static str,
    from: u32,
    to: u32,
    check: Check,
}

/// The granularities the campaign covers. `price_ohlcv_1m` is absent by
/// decision (D-07): it is the live base table, `coarse-repair` refuses it by
/// name, and the cleanup worker's 7-day policy means it cannot hold a 2023 row
/// anyway.
const DEEP: [Grain; 6] = [
    Grain {
        table: "price_ohlcv_15m",
        from: DEPEG_DAY,
        to: DEPEG_DAY_END,
        check: Check::RatioUnder(DEPEG_CEILING),
    },
    Grain {
        table: "price_ohlcv_1h",
        from: DEPEG_DAY,
        to: DEPEG_DAY_END,
        check: Check::RatioUnder(DEPEG_CEILING),
    },
    Grain {
        table: "price_ohlcv_4h",
        from: DEPEG_DAY,
        to: DEPEG_DAY_END,
        check: Check::RatioUnder(DEPEG_CEILING),
    },
    Grain {
        table: "price_ohlcv_1d",
        from: DEPEG_DAY,
        to: DEPEG_DAY_END,
        check: Check::RatioUnder(DEPEG_CEILING),
    },
    // The week of 03-06 ends on 03-13, when USDC had recovered to within bps of
    // par; the month ends on 04-01, at par. The depeg is invisible at these
    // grains by construction, so they are checked by the mechanism.
    Grain {
        table: "price_ohlcv_1w",
        from: DEPEG_WEEK_START,
        to: DEPEG_WEEK_END,
        check: Check::Mechanism,
    },
    Grain {
        table: "price_ohlcv_1M",
        from: DEPEG_MONTH_START,
        to: DEPEG_MONTH_END,
        check: Check::Mechanism,
    },
];

/// What one table says about the XLM-quoted (pivot-leg) population over
/// `[from, to)`.
///
/// `ratio` is the MEDIAN per-bucket `close_usd / (close × vwap(XLM/USDC))` — the
/// USDC/USD factor the stored value carries — and is meaningful only when
/// `matched > 0`; over an empty set the aggregate is `NaN`, which is why the
/// count is carried beside it rather than inferred from the value.
///
/// `zeros` counts rows at `close_usd = 0` that HAVE volume (the 0182 shape);
/// `unpriceable` counts the ones without (the permanent floor). `max_version` is
/// the highest `version` any row in the window carries.
#[derive(clickhouse::Row, serde::Deserialize, Debug, Clone, PartialEq)]
struct Measurement {
    ratio: f64,
    matched: u64,
    rows: u64,
    zeros: u64,
    unpriceable: u64,
    max_version: u64,
}

/// The one input a [`Check::Mechanism`] grain needs that the candle table cannot
/// supply: the bucket's `max(version)` BEFORE the run, recorded by the runbook's
/// "before" step and handed in as `POST_RUN_0228_VERSION_BEFORE_1W` / `_1M`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Mechanism {
    version_before: u64,
}

/// Production sits behind Caddy's mTLS, which the plain `CLICKHOUSE_URL` path
/// cannot reach from an operator laptop (task 0276). With `CH_DOMAIN` set, and
/// built with `--features aws-mtls`, connect the way `coarse-repair
/// --transport hetzner` does, from the same four variables.
#[cfg(feature = "aws-mtls")]
fn client() -> Client {
    if let Ok(domain) = std::env::var("CH_DOMAIN") {
        let path = |k: &str| {
            std::path::PathBuf::from(
                std::env::var(k).unwrap_or_else(|_| panic!("CH_DOMAIN is set, so {k} must be")),
            )
        };
        return prices_clickhouse::mtls::client_with_mtls_from_paths(
            &domain,
            &path("MTLS_CERT_PATH"),
            &path("MTLS_KEY_PATH"),
            &path("MTLS_CA_PATH"),
            &std::env::var("CH_DATABASE").unwrap_or_else(|_| "prices".into()),
        )
        .expect("mTLS client");
    }
    plain_client()
}

#[cfg(not(feature = "aws-mtls"))]
fn client() -> Client {
    plain_client()
}

fn plain_client() -> Client {
    Client::default()
        .with_url(
            std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".into()),
        )
        .with_database(std::env::var("CH_DATABASE").unwrap_or_else(|_| "prices".into()))
}

/// Measure the XLM-quoted population of `table` over `[from, to)` against the
/// XLM/USDC reference market in the SAME table and the SAME buckets.
///
/// The reference subquery reproduces [`pivot_sql`]'s own inline aggregate —
/// `sum(close·volume_base) / sum(volume_base)` grouped by `timestamp` — so the
/// denominator here is the very number the pivot multiplied by. It is a LEFT
/// JOIN: a bucket whose reference market has no candle still counts toward
/// `rows` and `zeros`, it simply cannot contribute a ratio.
///
/// One ungrouped aggregate always returns exactly one row — over an empty match
/// set it returns zero counts and a `NaN` ratio, never zero rows — so this is
/// `fetch_one` and emptiness is read from the counts.
async fn measure(ch: &Client, table: &str, from: u32, to: u32) -> Measurement {
    ch.query(&format!(
        "SELECT \
             medianIf(toFloat64(p.close_usd) / (toFloat64(p.close) * r.vwap), \
                      p.close_usd > 0 AND r.vwap > 0) AS ratio, \
             countIf(p.close_usd > 0 AND r.vwap > 0) AS matched, \
             count() AS rows, \
             countIf(p.close_usd = 0 AND p.volume_quote > 0) AS zeros, \
             countIf(p.close_usd = 0 AND p.volume_quote = 0) AS unpriceable, \
             max(p.version) AS max_version \
         FROM {table} AS p FINAL \
         INNER JOIN ( SELECT asset_id FROM assets FINAL \
                      WHERE asset_code = 'XLM' AND issuer_address = '' \
                        AND contract_address = '' ) AS x ON x.asset_id = p.quote_asset_id \
         LEFT JOIN ( \
             SELECT timestamp AS rts, \
                    sum(toFloat64(close) * toFloat64(volume_base)) \
                      / nullIf(sum(toFloat64(volume_base)), 0) AS vwap \
             FROM {table} FINAL \
             WHERE asset_id IN ( SELECT asset_id FROM assets FINAL \
                                 WHERE asset_code = 'XLM' AND issuer_address = '' \
                                   AND contract_address = '' ) \
               AND quote_asset_id IN ( SELECT asset_id FROM assets FINAL \
                                       WHERE asset_code = 'USDC' AND issuer_address = ? \
                                         AND contract_address = '' ) \
               AND timestamp >= toDateTime(?) AND timestamp < toDateTime(?) \
             GROUP BY timestamp \
         ) AS r ON r.rts = p.timestamp \
         WHERE p.timestamp >= toDateTime(?) AND p.timestamp < toDateTime(?) \
           AND p.close > 0"
    ))
    .bind(prices_clickhouse::USDC_ISSUER)
    .bind(from)
    .bind(to)
    .bind(from)
    .bind(to)
    .fetch_one::<Measurement>()
    .await
    .unwrap()
}

/// The pre-run `max(version)` the runbook's "before" step recorded for a
/// mechanism grain, from `POST_RUN_0228_VERSION_BEFORE_1W` / `_1M`. `None` when
/// the variable is unset or unparsable — which [`judge`] reports as a finding,
/// never as a pass.
fn version_before_from_env(table: &str) -> Option<u64> {
    let name = version_before_var_name(table)?;
    parse_version_before(std::env::var(name).ok().as_deref())
}

/// The variable name the runbook's "before" step must export for `table`;
/// `None` for anything that is not a `price_ohlcv_*` grain. Derived, so the
/// runbook and this file cannot drift apart.
fn version_before_var_name(table: &str) -> Option<String> {
    let suffix = table.strip_prefix("price_ohlcv_")?.to_ascii_uppercase();
    Some(format!("POST_RUN_0228_VERSION_BEFORE_{suffix}"))
}

/// Pure parser for the baseline: unset, blank or unparsable is `None`, and
/// [`judge`] turns `None` into a finding, never a pass. Kept free of the process
/// environment so the CI tests cannot race the falsifier under
/// `--include-ignored` (0268 review round 3, WR-11).
fn parse_version_before(raw: Option<&str>) -> Option<u64> {
    raw?.trim().parse().ok()
}

/// 🔑 THE JUDGEMENT, pure so CI can test it. `None` is a pass; `Some` is the
/// finding, worded for the operator. `mech` is consulted only for a
/// [`Check::Mechanism`] grain, where it is required.
fn judge(g: &Grain, m: &Measurement, mech: Option<&Mechanism>) -> Option<String> {
    let t = g.table;
    if m.rows == 0 {
        return Some(format!(
            "{t}: NO XLM-quoted candle in the window at all. That is a finding, not a \
             pass — either a retention boundary or a campaign that dropped rows."
        ));
    }
    if m.zeros > 0 {
        return Some(format!(
            "{t}: {} of {} XLM-quoted candles WITH volume sit at close_usd = 0 — reset \
             and never refilled, the task 0182 outcome. Roll the table back from its \
             FREEZE snapshot. ({} further rows sit at zero with no volume: the permanent \
             volume-zero floor, not a finding.)",
            m.zeros, m.rows, m.unpriceable
        ));
    }
    if m.matched == 0 {
        return Some(format!(
            "{t}: {} XLM-quoted candles, but none shares a bucket with a priced XLM/USDC \
             reference candle, so the carried USDC/USD factor cannot be measured. Check \
             that the reference market has candles in this window — the pivot needs one \
             too.",
            m.rows
        ));
    }
    if m.ratio < SANITY_FLOOR {
        return Some(format!(
            "{t}: carried USDC/USD factor {:.6} < {SANITY_FLOOR} — nothing in the depeg \
             weekend traded that low; a wrong rate or a wrong column was applied.",
            m.ratio
        ));
    }
    match g.check {
        Check::RatioUnder(ceiling) => {
            if m.ratio >= ceiling {
                let shape = if m.ratio == 1.0 {
                    "exactly 1.0 is the untouched USDC-denominated value"
                } else {
                    "a hair under par is a PARTIAL campaign"
                };
                return Some(format!(
                    "{t}: carried USDC/USD factor {:.6} >= {ceiling} — indistinguishable \
                     from par. USDC closed {DEPEG_RATE} on 2023-03-11 and the pivot must \
                     carry that measurement; the campaign did not reach this table \
                     ({shape}). Measured over {} of {} candles.",
                    m.ratio, m.matched, m.rows
                ));
            }
            None
        }
        Check::Mechanism => {
            let Some(mech) = mech else {
                return Some(format!(
                    "{t}: no pre-run version supplied. The bucket ends past the recovery, so \
                     its carried factor is ~1.0 whether or not the campaign ran; the check \
                     is that the campaign TOUCHED it. Record max(version) in Appendix C's \
                     \"before\" step and pass it as POST_RUN_0228_VERSION_BEFORE_{}.",
                    t.trim_start_matches("price_ohlcv_").to_ascii_uppercase()
                ));
            };
            if m.max_version <= mech.version_before {
                return Some(format!(
                    "{t}: max(version) is {} and was {} before the run — the campaign did \
                     not touch the bucket. Its carried factor (~1.0, the bucket ends after \
                     the recovery) cannot show that; the version can. \
                     ⚠️ A pivoted row leaves no par signature, so there is no second \
                     signal here — this IS the evidence (task 0228, D-06).",
                    m.max_version, mech.version_before
                ));
            }
            None
        }
    }
}

/// 🔑 **THE FALSIFIER FOR THE WHOLE CAMPAIGN.** An XLM-quoted candle on
/// 2023-03-11 must carry the measured USDC/USD factor rather than a dollar, on
/// every grain whose bucket ends inside the depeg, and the coarser buckets that
/// contain the day must show the campaign went through them with no row left at
/// zero. Acceptance criteria 1 and 2.
#[tokio::test]
#[ignore = "operator after-check: run against prod AFTER the 0228 re-enrichment campaign"]
async fn the_pivot_leg_on_the_depeg_day_carries_the_measured_usdc_rate() {
    let ch = client();
    let mut failures = Vec::new();

    for g in &DEEP {
        let m = measure(&ch, g.table, g.from, g.to).await;
        let mech = match (g.check, version_before_from_env(g.table)) {
            (Check::Mechanism, Some(version_before)) => Some(Mechanism { version_before }),
            _ => None,
        };
        if let Some(f) = judge(g, &m, mech.as_ref()) {
            failures.push(f);
        }
    }

    assert!(
        failures.is_empty(),
        "USDC closed at 0.9681 on 2023-03-11, so an XLM-quoted candle's stored USD \
         value must be its USDC-denominated value scaled by that measurement:\n  {}",
        failures.join("\n  ")
    );
}

/// The control. Four days later USDC was back within a few bps of par, so the
/// carried factor must be ~1.0 — which a table scaled uniformly ~3% low could not
/// satisfy while also passing the test above.
#[tokio::test]
#[ignore = "operator after-check: run against prod AFTER the 0228 re-enrichment campaign"]
async fn the_pivot_leg_carries_no_discount_once_usdc_is_back_at_par() {
    let ch = client();
    let m = measure(&ch, "price_ohlcv_1d", RECOVERED_DAY, RECOVERED_DAY + 86_400).await;
    assert!(
        m.rows > 0,
        "no XLM-quoted daily candle for 2023-03-15 — that is a finding, not a pass"
    );
    assert_eq!(
        m.zeros, 0,
        "a zeroed candle with volume on 2023-03-15: {m:?}"
    );
    assert!(
        m.matched > 0,
        "no XLM/USDC reference candle shares a bucket on 2023-03-15: {m:?}"
    );
    assert!(
        (m.ratio - 1.0).abs() < 0.005,
        "2023-03-15 carried factor {:.6}, expected ~1.0 (±0.005). A table scaled \
         uniformly low would pass the depeg check and fail here — which is what this \
         control exists to catch.",
        m.ratio
    );
}

// ---- the judgement, tested without a ClickHouse (these DO run in CI) --------

fn carried(ratio: f64, rows: u64) -> Measurement {
    Measurement {
        ratio,
        matched: rows,
        rows,
        zeros: 0,
        unpriceable: 0,
        max_version: 2,
    }
}

fn grain(table: &str) -> &'static Grain {
    DEEP.iter().find(|g| g.table == table).unwrap()
}

/// A mechanism-grain reading after a CORRECT campaign: version bumped past the
/// recorded baseline by the reset + refill pair.
fn repaired(ratio: f64, version_before: u64) -> (Measurement, Mechanism) {
    let mut m = carried(ratio, 1);
    m.max_version = version_before + 2;
    (m, Mechanism { version_before })
}

/// Which grains are judged which way is itself a design fact: `_1d` is the
/// coarsest grain whose bucket ends inside the depeg, and `_1m` is not covered
/// by the campaign at all (D-07).
#[test]
fn the_daily_and_shorter_grains_are_judged_by_ratio_and_the_coarser_by_mechanism() {
    for t in [
        "price_ohlcv_15m",
        "price_ohlcv_1h",
        "price_ohlcv_4h",
        "price_ohlcv_1d",
    ] {
        assert!(
            matches!(grain(t).check, Check::RatioUnder(c) if c < 1.0),
            "{t}: judged by a ratio ceiling under par"
        );
    }
    for t in ["price_ohlcv_1w", "price_ohlcv_1M"] {
        assert_eq!(grain(t).check, Check::Mechanism, "{t}");
    }
    assert!(
        !DEEP.iter().any(|g| g.table == "price_ohlcv_1m"),
        "the live base table is out of the campaign's scope (D-07)"
    );
}

/// Par must fail on every RATIO grain, by the ceiling alone, with no exact
/// equality test to defeat — and the message must say it is the untouched value.
#[test]
fn par_is_rejected_on_every_ratio_grain() {
    for g in DEEP.iter().filter(|g| g.check != Check::Mechanism) {
        let f = judge(g, &carried(1.0, 1_000), None)
            .unwrap_or_else(|| panic!("{}: par passed", g.table));
        assert!(f.contains("untouched"), "{}: {f}", g.table);
    }
}

/// A partial campaign lands a hair under par and must ALSO fail, with different
/// words: par and a partial pass are different operator situations.
#[test]
fn a_partial_campaign_a_hair_under_par_is_rejected_and_named() {
    for g in DEEP.iter().filter(|g| g.check != Check::Mechanism) {
        let f = judge(g, &carried(0.99997, 654_291), None).unwrap();
        assert!(f.contains("PARTIAL"), "{}: {f}", g.table);
        assert!(!f.contains("untouched"), "{}: {f}", g.table);
    }
}

/// The honest post-campaign value on the ratio grains: 0.9681 against a daily
/// series, lower where 0267's hourly rows cover the stress hours.
#[test]
fn the_measured_depeg_passes_on_the_ratio_grains() {
    for g in DEEP.iter().filter(|g| g.check != Check::Mechanism) {
        assert_eq!(
            judge(g, &carried(DEPEG_RATE, 24), None),
            None,
            "{}",
            g.table
        );
        assert_eq!(judge(g, &carried(0.91, 24), None), None, "{}", g.table);
    }
}

/// A correctly repaired `_1M` bucket reads ~1.0 — the month ends on 04-01, at
/// par — and MUST PASS. A ceiling there would report a false failure over a
/// correct table, with a FREEZE rollback advised next to it.
#[test]
fn a_correctly_repaired_coarse_bucket_at_a_par_ish_ratio_passes() {
    for t in ["price_ohlcv_1M", "price_ohlcv_1w"] {
        for ratio in [0.9999, 1.0, 0.9873] {
            let (m, mech) = repaired(ratio, 3);
            assert_eq!(judge(grain(t), &m, Some(&mech)), None, "{t} at {ratio}");
        }
    }
}

/// And the mechanism check has teeth: the same par-ish reading with the version
/// UNCHANGED is an untouched bucket, and fails. A missing baseline is a finding
/// too, never a pass.
#[test]
fn an_unrepaired_coarse_bucket_fails_on_the_version_not_the_ratio() {
    let (mut m, mech) = repaired(1.0, 3);
    m.max_version = 3;
    let f = judge(grain("price_ohlcv_1M"), &m, Some(&mech)).unwrap();
    assert!(f.contains("did not touch"), "{f}");
    assert!(
        f.contains("no par signature") || f.contains("leaves no par signature"),
        "the finding must say why the version is the only signal: {f}"
    );

    let f = judge(grain("price_ohlcv_1M"), &m, None).unwrap();
    assert!(f.contains("POST_RUN_0228_VERSION_BEFORE_1M"), "{f}");
}

/// `_1d` is judged by the ratio: par fails there and 0.9681 passes, whatever the
/// version did. The two kinds of check never cross.
#[test]
fn the_daily_grain_is_judged_by_its_ratio_alone() {
    let g = grain("price_ohlcv_1d");
    let (m, mech) = repaired(1.0, 3);
    assert!(
        judge(g, &m, Some(&mech)).is_some(),
        "par on _1d is unrepaired even with version moved"
    );
    let mut m = carried(DEPEG_RATE, 1);
    m.max_version = 1;
    assert_eq!(judge(g, &m, None), None);
}

/// No candle at all, a zeroed candle with volume, and no measurable bucket are
/// three different findings, each named, none a pass.
#[test]
fn missing_zeroed_and_unmeasurable_rows_are_findings_not_passes() {
    let g = grain("price_ohlcv_1d");
    let mut m = carried(DEPEG_RATE, 0);
    m.matched = 0;
    let none = judge(g, &m, None).unwrap();
    assert!(none.contains("NO XLM-quoted candle"), "{none}");

    let mut m = carried(DEPEG_RATE, 24);
    m.zeros = 3;
    let zeroed = judge(g, &m, None).unwrap();
    assert!(
        zeroed.contains("close_usd = 0") && zeroed.contains("0182"),
        "{zeroed}"
    );

    let mut m = carried(DEPEG_RATE, 24);
    m.matched = 0;
    m.ratio = f64::NAN;
    let unmeasurable = judge(g, &m, None).unwrap();
    assert!(
        unmeasurable.contains("cannot be measured"),
        "{unmeasurable}"
    );
}

/// A row at `close_usd = 0` WITHOUT volume is the permanent volume-zero floor —
/// no tier will ever price it — and must not read as the 0182 incident, nor mask
/// the ratio check. Only a zero WITH volume is a finding.
#[test]
fn zero_volume_rows_at_close_usd_zero_are_context_not_the_0182_outcome() {
    for g in &DEEP {
        let (mut m, mech) = repaired(DEPEG_RATE, 3);
        m.rows = 29;
        m.matched = 24;
        m.unpriceable = 5;
        assert_eq!(judge(g, &m, Some(&mech)), None, "{}", g.table);
        // The ratio check is still reached behind a clean zeros count.
        if let Check::RatioUnder(_) = g.check {
            let mut par = m.clone();
            par.ratio = 1.0;
            assert!(
                judge(g, &par, None).is_some(),
                "{}: unpriceable rows masked par",
                g.table
            );
        }
        // And one zero WITH volume is the finding, with the floor as context.
        m.zeros = 1;
        let f = judge(g, &m, Some(&mech)).unwrap();
        assert!(
            f.contains("0182") && f.contains("5 further rows"),
            "{}: {f}",
            g.table
        );
    }
}

/// A ratio below anything USDC traded at is a broken table, not a repaired one —
/// on both kinds of grain, because a wrong factor is wrong whether or not the
/// bucket could have shown the depeg.
#[test]
fn a_ratio_below_the_depeg_weekends_low_is_rejected_on_every_grain() {
    let f = judge(grain("price_ohlcv_1h"), &carried(0.5, 24), None).unwrap();
    assert!(f.contains("wrong rate"), "{f}");
    let (m, mech) = repaired(0.5, 3);
    let f = judge(grain("price_ohlcv_1M"), &m, Some(&mech)).unwrap();
    assert!(f.contains("wrong rate"), "{f}");
}

/// Every grain's window contains the depeg day, and the mechanism grains'
/// windows are exactly ONE bucket.
#[test]
fn every_window_covers_the_depeg_day_and_the_mechanism_windows_are_one_bucket() {
    for g in &DEEP {
        assert!(g.from <= DEPEG_DAY && DEPEG_DAY < g.to, "{}", g.table);
    }
    assert_eq!(DEPEG_WEEK_END - DEPEG_WEEK_START, 7 * 86_400);
    // 2023-03-06 is a Monday: (days since 1970-01-01 + 3) % 7 == 0 for Mondays
    // (1970-01-01 was a Thursday).
    assert_eq!(
        (DEPEG_WEEK_START / 86_400 + 3) % 7,
        0,
        "the week bucket starts on a Monday"
    );
    assert_eq!(
        DEPEG_MONTH_END - DEPEG_MONTH_START,
        31 * 86_400,
        "March has 31 days"
    );
    assert_eq!(DEPEG_MONTH_START % 86_400, 0);
}

/// The baseline variable name is derived from the table, so Appendix C's
/// `POST_RUN_0228_VERSION_BEFORE_1W` / `_1M` and this file cannot drift apart.
///
/// Pure on both sides: the derivation and the parser are tested without touching
/// the process environment, so this test cannot hand the falsifier a fake
/// baseline when the binary runs with `--include-ignored`.
#[test]
fn the_version_baseline_is_read_from_a_per_table_variable() {
    assert_eq!(
        version_before_var_name("price_ohlcv_1M").as_deref(),
        Some("POST_RUN_0228_VERSION_BEFORE_1M")
    );
    assert_eq!(
        version_before_var_name("price_ohlcv_1w").as_deref(),
        Some("POST_RUN_0228_VERSION_BEFORE_1W")
    );
    assert_eq!(version_before_var_name("not_a_table"), None);
    assert_eq!(parse_version_before(Some(" 7 ")), Some(7));
    assert_eq!(parse_version_before(Some("")), None);
    assert_eq!(parse_version_before(Some("seven")), None);
    assert_eq!(parse_version_before(None), None);
}
