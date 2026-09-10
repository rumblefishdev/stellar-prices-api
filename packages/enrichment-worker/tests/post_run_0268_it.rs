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
//!   POST_RUN_0268_VERSION_BEFORE_1W=<n> POST_RUN_0268_VERSION_BEFORE_1M=<n> \
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
//! peg tier, so `native`'s USD close read ~3.19% HIGH.
//!
//! ## Two kinds of check, because ONE rate prices a bucket (decision G)
//!
//! The external tier resolves ONE rate at the bucket's END and prices the whole
//! bucket with it: `close_usd = r(bucket_end) × close`. There is no blending of
//! the bucket's days. So the implied rate `Σ(close_usd·vol) / Σ(close·vol)` over
//! a bucket collapses to exactly `r(bucket_end)`, and what a grain can show
//! depends only on WHERE its bucket ends:
//!
//! * **`_1h`, `_4h`, `_1d`** — every bucket of 2023-03-11 ends at or before
//!   2023-03-12 00:00, and the ASOF is strict (`rts < bend`), so against a
//!   day-start daily series every one of them resolves to the **03-11** rate,
//!   0.9681. The rate IS the evidence: it must sit UNDER a ceiling par cannot
//!   satisfy ([`Check::RateUnder`]). `_1d` is the coarsest grain that can carry
//!   the depeg at all.
//! * **`_1w`, `_1M`** — the bucket containing 03-11 ends on 03-13 and 04-01,
//!   when USDC was back at (or within bps of) par. A correctly repaired monthly
//!   row therefore reads ~1.0 by construction, and NO ceiling can separate
//!   "repaired" from "untouched" there. Those grains are checked by the
//!   MECHANISM instead ([`Check::Mechanism`]): the bucket's `max(version)` must
//!   have moved past the value the runbook's "before" step recorded, no row may
//!   sit at zero, and the par signature `close_usd = close` may survive ONLY if
//!   the series' own bucket-end rate is exactly 1.0.
//!
//! ⚠️ That last clause is a real ambiguity, not a corner case: a bucket-end rate
//! of exactly 1.0 makes `close_usd = close`, and the API's read-time label then
//! says `assumed-par` although the value was measured. The falsifier tolerates
//! it because it can read the series; the wire cannot. Task file, Issues 9.
//!
//! ## Why the daily ceiling is directional, not a band around 0.9681
//!
//! A band `0.9681 ± 0.04` contains 1.0. Par is then rejected only by an exact
//! equality test, which any PARTIAL repair defeats: reach 0.1% of the rows and
//! the volume-weighted rate lands at ~0.99997 — not exactly 1.0, comfortably
//! inside the band — and an essentially unrepaired table reports green. So the
//! daily-or-shorter grains get a ceiling the pass must get UNDER.
//!
//! ## Why they fail loudly rather than skip
//!
//! A missing row is not a pass. If `native` has no candle covering 2023-03-11 in
//! a given table, that is either a retention boundary the operator must know
//! about or a repair that silently dropped rows — both are findings. And a row
//! at `close_usd = 0` WITH volume is the task 0182 outcome (reset, never
//! refilled), which the rate alone cannot see because a zero contributes nothing
//! to the sum — so those are counted separately and any of them is a failure.
//! A row at `close_usd = 0` WITHOUT volume is the permanent volume-zero floor
//! every tier shares (`volume_quote > 0` is in every candidate predicate); it is
//! reported as context and is never a failure (review WR-08).
//!
//! ## Why a second date
//!
//! `usdc_is_back_at_par_a_few_days_later` exists so the first test cannot be
//! satisfied by pricing EVERYTHING ~3% low — a uniformly scaled table would pass
//! the depeg check and fail this one.

use clickhouse::Client;

/// 2023-03-11 00:00:00 UTC — the depeg day.
const DEPEG_DAY: u32 = 1_678_492_800;
/// 2023-03-12 00:00:00 UTC — the end of every `_1d`-or-shorter bucket of it.
const DEPEG_DAY_END: u32 = DEPEG_DAY + 86_400;
/// 2023-03-06 00:00:00 UTC (a Monday) — the start of the `_1w` bucket that
/// contains the depeg day — and its end, 2023-03-13.
const DEPEG_WEEK_START: u32 = 1_678_060_800;
const DEPEG_WEEK_END: u32 = DEPEG_WEEK_START + 7 * 86_400;
/// 2023-03-01 00:00:00 UTC — the `_1M` bucket that contains the depeg day —
/// and its end, 2023-04-01.
const DEPEG_MONTH_START: u32 = 1_677_628_800;
const DEPEG_MONTH_END: u32 = 1_680_307_200;
/// 2023-03-15 00:00:00 UTC — recovered.
const RECOVERED_DAY: u32 = 1_678_838_400;

/// USDC's measured close on 2023-03-11. The number this task exists for.
const DEPEG_RATE: f64 = 0.9681;

/// Below this the table is not "repaired", it is broken: nothing in the
/// depeg weekend traded USDC under ~0.88, so a bucket rate under 0.85 means a
/// wrong rate (or a wrong column) was applied, not the measured one.
const SANITY_FLOOR: f64 = 0.85;

/// The ceiling for the grains whose bucket ends inside the depeg. Par cannot
/// satisfy it; a partial pass at ~0.99997 cannot; 0.9681 can with room.
const DAILY_CEILING: f64 = 0.99;

/// How a grain is judged. See the module doc for why there are two.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Check {
    /// The bucket END is still inside the depeg, so the rate IS the day's and
    /// must sit strictly under the ceiling.
    RateUnder(f64),
    /// The bucket END is past the recovery. The implied rate is ~1.0 by
    /// construction (one rate, resolved at the bucket end, prices the whole
    /// bucket), so par is not evidence of an unrepaired table. Check that the
    /// pass TOUCHED the bucket and that the par signature survived only if the
    /// series itself says 1.0 at the bucket end.
    Mechanism,
}

/// One granularity's expectation over the bucket(s) `[from, to)` that hold the
/// depeg day. `to` is also the bucket END for the mechanism grains — the instant
/// the series is read at to learn what rate the tier resolved.
struct Grain {
    table: &'static str,
    from: u32,
    to: u32,
    check: Check,
}

/// The granularities that hold deep history. `_15m` has a 30-day retention and
/// `_1m` was largely dropped by the cleanup worker for 2025-02 → 2026-02, so
/// neither can carry a 2023 row; the repair driver refuses `_1m` outright.
///
/// Table and window travel together (review IN-07): arrays zipped by position
/// silently mis-pair when one is reordered.
const DEEP: [Grain; 5] = [
    // A daily-or-shorter bucket ends inside the depeg day. Against a day-start
    // DAILY series every one of them resolves to the same 03-11 rate, 0.9681
    // exactly — the ASOF is strict and the 23:00 bucket's end is 03-12 00:00.
    // If task 0267 also loads HOURLY rows for the stress days, the intraday
    // buckets resolve to their own hour's rate instead, which on 03-11 ran as
    // low as ~0.88; either way, under the ceiling.
    Grain {
        table: "price_ohlcv_1h",
        from: DEPEG_DAY,
        to: DEPEG_DAY_END,
        check: Check::RateUnder(DAILY_CEILING),
    },
    Grain {
        table: "price_ohlcv_4h",
        from: DEPEG_DAY,
        to: DEPEG_DAY_END,
        check: Check::RateUnder(DAILY_CEILING),
    },
    Grain {
        table: "price_ohlcv_1d",
        from: DEPEG_DAY,
        to: DEPEG_DAY_END,
        check: Check::RateUnder(DAILY_CEILING),
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

/// What one table says about `native`/USDC over `[from, to)`.
///
/// `rate` is `None` when no candle in the window has a positive `close_usd` —
/// distinct from `rows == 0`, which is "no candle at all". `zeros` counts rows
/// at `close_usd = 0` that HAVE volume (the 0182 shape); `unpriceable` counts
/// the ones without (the permanent floor). `at_par` counts rows carrying the
/// peg signature `close_usd = close`; `max_version` is the highest `version`
/// any row in the window carries.
#[derive(clickhouse::Row, serde::Deserialize, Debug, Clone, PartialEq)]
struct Measurement {
    rate: Option<f64>,
    rows: u64,
    zeros: u64,
    unpriceable: u64,
    at_par: u64,
    max_version: u64,
}

/// The two inputs a [`Check::Mechanism`] grain needs that the candle table
/// cannot supply: the bucket's `max(version)` BEFORE the run (recorded by the
/// runbook's "before" step, handed in as `POST_RUN_0268_VERSION_BEFORE_1W` /
/// `_1M`), and the series' own rate at the bucket end.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Mechanism {
    version_before: u64,
    /// The newest `external` rate strictly before the bucket end — what the
    /// tier's ASOF resolved. `None` when the series has no row before it.
    bucket_end_rate: Option<f64>,
}

fn client() -> Client {
    Client::default()
        .with_url(
            std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".into()),
        )
        .with_database(std::env::var("CH_DATABASE").unwrap_or_else(|_| "prices".into()))
}

/// The volume-weighted implied rate `close_usd / close` for `native`'s
/// USDC-quoted candles in `[from, to)`, in `table`, plus the counts.
///
/// Volume-weighted rather than a bare average because a bucket may merge several
/// sources; `argMax`-ing one of them would make the answer depend on which venue
/// happened to trade last. The `close > 0` guard keeps a zero close out of the
/// division; the `close_usd > 0` condition is INSIDE the sums, not in the
/// `WHERE`, so a zeroed row still counts toward `rows` and `zeros`.
///
/// `zeros` mirrors the tiers' own candidate predicate — `close_usd = 0 AND
/// volume_quote > 0` — because a zero-volume row is one no tier will ever price
/// (review WR-08); it is counted as `unpriceable` instead.
///
/// One ungrouped aggregate always returns exactly one row — over an empty match
/// set it returns `NULL`s and zero counts, never zero rows — so this is
/// `fetch_one` into a typed row with a `Nullable` column, and emptiness is read
/// from `rows`, not from the absence of a result (review WR-01).
async fn measure(ch: &Client, table: &str, from: u32, to: u32) -> Measurement {
    ch.query(&format!(
        "SELECT \
             sumIf(toFloat64(close_usd) * toFloat64(volume_base), close_usd > 0) \
               / nullIf(sumIf(toFloat64(close) * toFloat64(volume_base), close_usd > 0), 0) AS rate, \
             count() AS rows, \
             countIf(close_usd = 0 AND volume_quote > 0) AS zeros, \
             countIf(close_usd = 0 AND volume_quote = 0) AS unpriceable, \
             countIf(close_usd = close) AS at_par, \
             max(version) AS max_version \
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
    .bind(to)
    .fetch_one::<Measurement>()
    .await
    .unwrap()
}

/// The newest `external` rate for canonical USDC strictly before `bucket_end`
/// — the row the tier's `ASOF … r.rts < p.bend` resolved. Same identity tuple
/// and `method` filter as `ch_enrich::external_sql`'s reference side.
async fn bucket_end_rate(ch: &Client, bucket_end: u32) -> Option<f64> {
    ch.query(
        "SELECT toFloat64(usd_rate) FROM usd_rate FINAL \
         WHERE asset_kind = 'credit' AND asset_code = 'USDC' \
           AND issuer_address = ? AND contract_address = '' \
           AND method = 'external' AND timestamp < toDateTime(?) \
         ORDER BY timestamp DESC LIMIT 1",
    )
    .bind(prices_clickhouse::USDC_ISSUER)
    .bind(bucket_end)
    .fetch_optional::<f64>()
    .await
    .unwrap()
}

/// The pre-run `max(version)` the runbook's "before" step recorded for a
/// mechanism grain, from `POST_RUN_0268_VERSION_BEFORE_1W` / `_1M`. `None` when
/// the variable is unset or unparsable — which [`judge`] reports as a finding,
/// never as a pass.
fn version_before_from_env(table: &str) -> Option<u64> {
    let name = version_before_var_name(table)?;
    parse_version_before(std::env::var(name).ok().as_deref())
}

/// The variable name the runbook's "before" step must export for `table`;
/// `None` for anything that is not a `price_ohlcv_*` grain.
fn version_before_var_name(table: &str) -> Option<String> {
    let suffix = table.strip_prefix("price_ohlcv_")?.to_ascii_uppercase();
    Some(format!("POST_RUN_0268_VERSION_BEFORE_{suffix}"))
}

/// Pure parser for the baseline: unset, blank or unparsable is `None`, and
/// [`judge`] turns `None` into a finding, never a pass. Kept free of the
/// process environment so the CI test cannot race the falsifier under
/// `--include-ignored` (review round 3, WR-11).
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
            "{t}: NO native/USDC candle covering 2023-03-11 at all. That is a finding, \
             not a pass — either a retention boundary or a repair that dropped rows."
        ));
    }
    if m.zeros > 0 {
        return Some(format!(
            "{t}: {} of {} candles WITH volume sit at close_usd = 0 — reset and never \
             refilled, the task 0182 outcome. Roll the table back from its FREEZE \
             snapshot. ({} further rows sit at zero with no volume: the permanent \
             volume-zero floor, not a finding.)",
            m.zeros, m.rows, m.unpriceable
        ));
    }
    match g.check {
        Check::RateUnder(ceiling) => {
            let Some(rate) = m.rate else {
                return Some(format!(
                    "{t}: {} candles but none with a positive close_usd to measure.",
                    m.rows
                ));
            };
            if rate >= ceiling {
                let shape = if rate == 1.0 {
                    "exactly 1.0 is the untouched peg value"
                } else {
                    "a few bps under par is a PARTIAL pass"
                };
                return Some(format!(
                    "{t}: implied rate {rate:.6} >= {ceiling} — indistinguishable from par. \
                     USDC closed {DEPEG_RATE} on 2023-03-11; the pass did not reach this \
                     table ({shape})."
                ));
            }
            if rate < SANITY_FLOOR {
                return Some(format!(
                    "{t}: implied rate {rate:.6} < {SANITY_FLOOR} — nothing in the depeg \
                     weekend traded that low; a wrong rate or a wrong column was applied."
                ));
            }
            None
        }
        Check::Mechanism => {
            let Some(mech) = mech else {
                return Some(format!(
                    "{t}: no pre-run version supplied. The bucket ends past the recovery, so \
                     its rate cannot show the depeg; the check is that the pass TOUCHED it. \
                     Record max(version) in the runbook's \"before\" step and pass it as \
                     POST_RUN_0268_VERSION_BEFORE_{}.",
                    t.trim_start_matches("price_ohlcv_").to_ascii_uppercase()
                ));
            };
            if m.max_version <= mech.version_before {
                return Some(format!(
                    "{t}: max(version) is {} and was {} before the run — the pass did not \
                     touch the bucket. Its rate (~1.0, the bucket ends after the recovery) \
                     cannot show that; the version can.",
                    m.max_version, mech.version_before
                ));
            }
            if m.at_par > 0 && mech.bucket_end_rate != Some(1.0) {
                return Some(format!(
                    "{t}: {} of {} candles still carry close_usd = close while the series' \
                     rate at the bucket end is {:?} — the par signature survived a run that \
                     bumped version. (Only a bucket-end rate of exactly 1.0 may leave the \
                     signature in place; then the wire label reads assumed-par for a measured \
                     value — task file, Issues 9.)",
                    m.at_par, m.rows, mech.bucket_end_rate
                ));
            }
            if let Some(rate) = m.rate
                && rate < SANITY_FLOOR
            {
                return Some(format!(
                    "{t}: implied rate {rate:.6} < {SANITY_FLOOR} — nothing in the depeg \
                     weekend traded that low; a wrong rate or a wrong column was applied."
                ));
            }
            None
        }
    }
}

/// 🔑 **THE FALSIFIER FOR THE WHOLE TASK.** `native` on 2023-03-11 must publish a
/// USD close BELOW its USDC-denominated close on every grain whose bucket ends
/// inside the depeg, and the coarser buckets that contain the day must show the
/// pass went through them, with no row left at zero. Acceptance criteria 1 and 2.
#[tokio::test]
#[ignore = "operator after-check: run against prod AFTER the 0268 re-enrichment pass"]
async fn native_on_the_depeg_day_is_priced_below_its_usdc_close() {
    let ch = client();
    let mut failures = Vec::new();

    for g in &DEEP {
        let m = measure(&ch, g.table, g.from, g.to).await;
        let mech = match (g.check, version_before_from_env(g.table)) {
            (Check::Mechanism, Some(version_before)) => Some(Mechanism {
                version_before,
                bucket_end_rate: bucket_end_rate(&ch, g.to).await,
            }),
            _ => None,
        };
        if let Some(f) = judge(g, &m, mech.as_ref()) {
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
    let m = measure(&ch, "price_ohlcv_1d", RECOVERED_DAY, RECOVERED_DAY + 86_400).await;
    assert!(
        m.rows > 0,
        "no native/USDC daily candle for 2023-03-15 — that is a finding, not a pass"
    );
    assert_eq!(
        m.zeros, 0,
        "a zeroed candle with volume on 2023-03-15: {m:?}"
    );
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
        unpriceable: 0,
        at_par: 0,
        max_version: 2,
    }
}

fn grain(table: &str) -> &'static Grain {
    DEEP.iter().find(|g| g.table == table).unwrap()
}

/// A mechanism-grain reading after a CORRECT repair: version bumped past the
/// recorded baseline, no par signature, and the series' bucket-end rate is
/// whatever the bucket was priced from.
fn repaired(rate: f64, version_before: u64) -> (Measurement, Mechanism) {
    let mut m = priced(rate, 1);
    m.max_version = version_before + 2;
    (
        m,
        Mechanism {
            version_before,
            bucket_end_rate: Some(rate),
        },
    )
}

/// Which grains are judged which way is itself a design fact (decision G):
/// `_1d` is the coarsest grain whose bucket ends inside the depeg.
#[test]
fn the_daily_and_shorter_grains_are_judged_by_rate_and_the_coarser_by_mechanism() {
    for t in ["price_ohlcv_1h", "price_ohlcv_4h", "price_ohlcv_1d"] {
        assert!(
            matches!(grain(t).check, Check::RateUnder(c) if c < 1.0),
            "{t}: judged by a rate ceiling under par"
        );
    }
    for t in ["price_ohlcv_1w", "price_ohlcv_1M"] {
        assert_eq!(grain(t).check, Check::Mechanism, "{t}");
    }
}

/// WR-02's finding, as a test: par must fail on every RATE grain, by the
/// ceiling alone, with no exact-equality test to defeat.
#[test]
fn par_is_rejected_on_every_rate_grain() {
    for g in DEEP.iter().filter(|g| g.check != Check::Mechanism) {
        let f = judge(g, &priced(1.0, 1_000), None)
            .unwrap_or_else(|| panic!("{}: par passed", g.table));
        assert!(f.contains("untouched peg value"), "{}: {f}", g.table);
    }
}

/// A partial pass — 0.1% of the rows repaired — lands a few bps under par and
/// must ALSO fail, and the finding must SAY so: par and a partial pass are
/// different operator situations, so they get different words (review IN-11).
#[test]
fn a_partial_repair_a_few_bps_under_par_is_rejected_and_named() {
    for g in DEEP.iter().filter(|g| g.check != Check::Mechanism) {
        let f = judge(g, &priced(0.99997, 654_291), None).unwrap();
        assert!(f.contains("PARTIAL"), "{}: {f}", g.table);
        assert!(!f.contains("untouched"), "{}: {f}", g.table);
    }
}

/// The honest post-repair value on the rate grains: 0.9681 exactly against a
/// daily series, lower if 0267 ships hourly rows for the stress days.
#[test]
fn the_measured_depeg_passes_on_the_rate_grains() {
    for t in ["price_ohlcv_1h", "price_ohlcv_4h", "price_ohlcv_1d"] {
        assert_eq!(judge(grain(t), &priced(DEPEG_RATE, 24), None), None, "{t}");
        assert_eq!(judge(grain(t), &priced(0.93, 24), None), None, "{t}");
    }
}

/// 🔑 CR-02. A correctly repaired `_1M` bucket reads ~1.0 — the month ends on
/// 04-01, at par — and MUST PASS. The first falsifier put a ceiling of 0.9995 on
/// it and would have reported a false failure over a correct table, with a
/// FREEZE rollback advised next to it.
#[test]
fn a_correctly_repaired_monthly_bucket_at_a_par_ish_rate_passes() {
    for t in ["price_ohlcv_1M", "price_ohlcv_1w"] {
        for rate in [0.9999, 1.0, 0.9873] {
            let (m, mech) = repaired(rate, 3);
            assert_eq!(judge(grain(t), &m, Some(&mech)), None, "{t} at {rate}");
        }
    }
}

/// And the mechanism check has teeth: the same par-ish reading with the
/// version UNCHANGED is an untouched bucket, and fails.
#[test]
fn an_unrepaired_monthly_bucket_fails_on_the_version_not_the_rate() {
    let (mut m, mech) = repaired(1.0, 3);
    m.max_version = 3;
    let f = judge(grain("price_ohlcv_1M"), &m, Some(&mech)).unwrap();
    assert!(f.contains("did not touch"), "{f}");
    // A missing baseline is a finding too, never a pass.
    let f = judge(grain("price_ohlcv_1M"), &m, None).unwrap();
    assert!(f.contains("POST_RUN_0268_VERSION_BEFORE_1M"), "{f}");
}

/// The par signature may survive on a mechanism grain ONLY when the series'
/// bucket-end rate is exactly 1.0 — the label-ambiguity trap, tolerated here
/// because the falsifier can read the series. Anything else is a peg value
/// that outlived a run which bumped version.
#[test]
fn a_surviving_par_signature_needs_a_bucket_end_rate_of_exactly_one() {
    let g = grain("price_ohlcv_1w");
    let (mut m, mut mech) = repaired(1.0, 3);
    m.at_par = 1;
    assert_eq!(
        judge(g, &m, Some(&mech)),
        None,
        "rate exactly 1.0 explains the signature"
    );
    mech.bucket_end_rate = Some(0.9999);
    assert!(
        judge(g, &m, Some(&mech))
            .unwrap()
            .contains("par signature survived")
    );
    mech.bucket_end_rate = None;
    assert!(
        judge(g, &m, Some(&mech))
            .unwrap()
            .contains("par signature survived")
    );
}

/// `_1d` is judged by the rate: par fails there and 0.9681 passes, whatever the
/// version did. The two kinds of check never cross.
#[test]
fn the_daily_grain_is_judged_by_its_rate_alone() {
    let g = grain("price_ohlcv_1d");
    let (m, mech) = repaired(1.0, 3);
    assert!(
        judge(g, &m, Some(&mech)).is_some(),
        "par on _1d is unrepaired even with version moved"
    );
    let mut m = priced(DEPEG_RATE, 1);
    m.max_version = 1;
    assert_eq!(judge(g, &m, None), None);
}

/// No candle at all, a zeroed candle with volume, and no priced candle are three
/// different findings, each named, none a pass.
#[test]
fn missing_zeroed_and_unpriced_rows_are_findings_not_passes() {
    let g = grain("price_ohlcv_1d");
    let mut m = priced(DEPEG_RATE, 0);
    m.rate = None;
    let none = judge(g, &m, None).unwrap();
    assert!(none.contains("NO native/USDC candle"), "{none}");

    let mut m = priced(DEPEG_RATE, 24);
    m.zeros = 3;
    let zeroed = judge(g, &m, None).unwrap();
    assert!(
        zeroed.contains("close_usd = 0") && zeroed.contains("0182"),
        "{zeroed}"
    );

    let mut m = priced(DEPEG_RATE, 24);
    m.rate = None;
    let unpriced = judge(g, &m, None).unwrap();
    assert!(
        unpriced.contains("none with a positive close_usd"),
        "{unpriced}"
    );
}

/// 🔑 WR-08. A row at `close_usd = 0` WITHOUT volume is the permanent
/// volume-zero floor — no tier will ever price it — and must not read as the
/// 0182 incident, nor mask the rate check. Only a zero WITH volume is a finding.
#[test]
fn zero_volume_rows_at_close_usd_zero_are_context_not_the_0182_outcome() {
    for g in &DEEP {
        let (mut m, mech) = repaired(DEPEG_RATE, 3);
        m.rows = 29;
        m.unpriceable = 5;
        assert_eq!(judge(g, &m, Some(&mech)), None, "{}", g.table);
        // The rate check is still reached behind a clean zeros count.
        if let Check::RateUnder(_) = g.check {
            let mut par = m.clone();
            par.rate = Some(1.0);
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

/// A rate below anything USDC traded at is a broken table, not a repaired one —
/// on both kinds of grain.
#[test]
fn a_rate_below_the_depeg_weekends_low_is_rejected() {
    let f = judge(grain("price_ohlcv_1h"), &priced(0.5, 24), None).unwrap();
    assert!(f.contains("wrong rate"), "{f}");
    let (m, mech) = repaired(0.5, 3);
    let f = judge(grain("price_ohlcv_1M"), &m, Some(&mech)).unwrap();
    assert!(f.contains("wrong rate"), "{f}");
}

/// Every grain's window contains the depeg day, and the mechanism grains'
/// windows are exactly ONE bucket — `to` doubles as the bucket end the series
/// is read at, so it must be the calendar end of the bucket that starts at
/// `from`.
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

/// The baseline variable name is derived from the table, so the runbook's
/// `POST_RUN_0268_VERSION_BEFORE_1W` / `_1M` and this file cannot drift apart.
#[test]
fn the_version_baseline_is_read_from_a_per_table_variable() {
    // Pure on both sides: the name derivation and the parser are tested
    // without touching the process environment, so this test cannot hand the
    // falsifier a fake baseline when the binary runs with --include-ignored.
    assert_eq!(
        version_before_var_name("price_ohlcv_1M").as_deref(),
        Some("POST_RUN_0268_VERSION_BEFORE_1M")
    );
    assert_eq!(
        version_before_var_name("price_ohlcv_1w").as_deref(),
        Some("POST_RUN_0268_VERSION_BEFORE_1W")
    );
    assert_eq!(version_before_var_name("not_a_table"), None);
    assert_eq!(parse_version_before(Some(" 7 ")), Some(7));
    assert_eq!(parse_version_before(Some("")), None);
    assert_eq!(parse_version_before(Some("seven")), None);
    assert_eq!(parse_version_before(None), None);
}
