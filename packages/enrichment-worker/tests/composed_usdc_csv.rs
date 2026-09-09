//! Task 0267 — the dry run over the REAL composed USDC/USD series, as a CI
//! test.
//!
//! ⚠️ Deliberately NOT `#[ignore]`. It exercises only the pure lib path
//! (`external_rate`), so it needs no ClickHouse, no credentials and no
//! `aws-mtls` feature — which means it runs on every push, unlike the operator
//! binary that wraps the same functions and which CI cannot even compile
//! (`required-features` bins are silently SKIPPED by `cargo test --workspace`).
//!
//! What it pins is BRIEF acceptance criterion 2: the exact figure set the
//! runbook makes an operator check before the load. Those figures are the gate
//! — an operator whose dry run prints something else is told to STOP — so if
//! they can drift silently between here and the tool, the gate is decorative.
//!
//! The input is the artefact task 0265 produced, versioned in this repo under
//! that task's archive directory and read 1:1. The file is deliberately NOT
//! copied into a fixtures directory: a second copy is a second thing to keep in
//! step, and the point of these assertions is that the tool agrees with the
//! artefact an operator will actually pass it.

use enrichment_worker::external_rate::{
    ACCEPTED_QUALITIES, ACCEPTED_SOURCES, CANONICAL_ASSET_CODE, DEPEG_DAY_START_S, DEPEG_HOUR_S,
    Grain, check_identity, parse_csv, partition,
};
use prices_clickhouse::{USDC_ISSUER, USDC_ORACLE_EPOCH_S};

/// Anchored to the CRATE, not to the working directory — `cargo test` runs from
/// the workspace root and `cargo test -p …` from wherever the caller stood.
const COMPOSED_CSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lore/1-tasks/archive/0265_FEATURE_price-usdc-from-measurement-not-the-peg/data/composed_usdc_usd_1d.csv"
));

/// The HOURLY companion, versioned on the same branch (Adam, 2026-09-09). Same
/// ten columns, same vocabulary, same identity — one row per UTC hour instead
/// of one per UTC day.
const COMPOSED_CSV_1H: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lore/1-tasks/archive/0265_FEATURE_price-usdc-from-measurement-not-the-peg/data/composed_usdc_usd_1h.csv"
));

/// The name the refusals carry; the bin passes the real path.
const CSV_NAME: &str = "composed_usdc_usd_1d.csv";
const CSV_NAME_1H: &str = "composed_usdc_usd_1h.csv";

/// The figure set the runbook's dry-run gate checks, all in one place.
///
/// Each was measured against the versioned file, and each fails differently:
/// a wrong `PARSED` means rows were dropped or the file changed; a wrong
/// `LOADABLE`/`SKIPPED` split means the epoch partition moved; wrong quality
/// counts mean the composer's vocabulary changed under us.
const PARSED: usize = 2049;
const LOADABLE: usize = 1872;
const SKIPPED_AT_OR_ABOVE_EPOCH: usize = 177;
const FALLBACK: usize = 24;
const MEASURED_DISPUTED: usize = 4;
const MEASURED: usize = 1844;
/// ⚠️ Source counts over the LOADABLE set, which is not the whole file: all
/// 177 skipped days are chainlink, so the file's own totals are 2025/24 and
/// these are 1848/24. The two are asserted separately below rather than
/// conflated — a single number that could be read either way is how a figure
/// gate stops meaning anything.
const CHAINLINK_LOADABLE: usize = 1848;
const BITSTAMP_LOADABLE: usize = 24;
const CHAINLINK_PARSED: usize = 2025;
const BITSTAMP_PARSED: usize = 24;

#[test]
fn the_versioned_composed_csv_is_present_and_non_empty() {
    // Asserted FIRST and separately so no later check can pass vacuously: an
    // empty or missing file would otherwise make several counts trivially
    // agree at zero.
    assert!(
        COMPOSED_CSV.len() > 100_000,
        "the composed CSV looks truncated ({} bytes)",
        COMPOSED_CSV.len()
    );
    assert!(
        COMPOSED_CSV.lines().count() > PARSED,
        "expected a header plus {PARSED} data rows"
    );
}

#[test]
fn the_dry_run_over_the_versioned_csv_reports_the_runbook_figures() {
    let rows = parse_csv(CSV_NAME, COMPOSED_CSV, Grain::Daily)
        .expect("the versioned composed CSV must parse in full");
    assert_eq!(rows.len(), PARSED, "data rows parsed");

    // Over the WHOLE file, before the epoch partition — the figures the task
    // text quotes. Every skipped day is a chainlink day, which is why the
    // loadable totals below differ only in that column.
    assert_eq!(
        rows.iter().filter(|r| r.source == "chainlink").count(),
        CHAINLINK_PARSED
    );
    assert_eq!(
        rows.iter().filter(|r| r.source == "bitstamp").count(),
        BITSTAMP_PARSED
    );

    let plan = partition(rows);
    assert_eq!(plan.parsed, PARSED);
    assert_eq!(
        plan.loadable.len(),
        LOADABLE,
        "rows strictly below the oracle epoch — these are the ones written"
    );
    assert_eq!(
        plan.skipped_at_or_above_epoch, SKIPPED_AT_OR_ABOVE_EPOCH,
        "Decision F: the composed series runs to 2026-09-04, well past our own \
         oracle's first reading. These rows are correct data this tool has no \
         business writing — counted and reported, never written, never a refusal."
    );
    assert_eq!(
        plan.loadable.len() + plan.skipped_at_or_above_epoch,
        PARSED,
        "the partition must be exhaustive — a row that is neither loadable nor \
         skipped has gone missing without anyone being told"
    );

    assert_eq!(plan.by_quality.get("fallback"), Some(&FALLBACK));
    assert_eq!(
        plan.by_quality.get("measured-disputed"),
        Some(&MEASURED_DISPUTED)
    );
    assert_eq!(plan.by_quality.get("measured"), Some(&MEASURED));
    assert_eq!(plan.by_source.get("chainlink"), Some(&CHAINLINK_LOADABLE));
    assert_eq!(plan.by_source.get("bitstamp"), Some(&BITSTAMP_LOADABLE));

    // Every counted row is accounted for by a known vocabulary word, so a new
    // value the composer starts emitting cannot hide inside a matching total.
    let quality_total: usize = plan.by_quality.values().sum();
    let source_total: usize = plan.by_source.values().sum();
    assert_eq!(quality_total, LOADABLE);
    assert_eq!(source_total, LOADABLE);
    assert!(
        plan.by_quality
            .keys()
            .all(|k| ACCEPTED_QUALITIES.contains(&k.as_str()))
    );
    assert!(
        plan.by_source
            .keys()
            .all(|k| ACCEPTED_SOURCES.contains(&k.as_str()))
    );
}

#[test]
fn the_2023_03_11_close_is_the_depeg_value_and_it_is_loadable() {
    let plan = partition(parse_csv(CSV_NAME, COMPOSED_CSV, Grain::Daily).unwrap());
    let depeg = plan
        .loadable
        .iter()
        .find(|r| r.ts == DEPEG_DAY_START_S)
        .expect("2023-03-11 must be in the LOADABLE set — it is this task's falsifier");

    // The exact decimal the composer wrote. `0.9681` as quoted in the task text
    // is a four-significant-figure rounding of this, and asserting on it would
    // fail.
    assert_eq!(
        depeg.rate.to_string(),
        "0.96812",
        "the day USDC depegged must carry its measured rate, not the $1 peg"
    );
    assert_eq!(depeg.source, "chainlink");
    assert_eq!(depeg.quality, "measured");
}

#[test]
fn the_last_loadable_day_is_below_the_epoch_and_the_first_skipped_one_is_not() {
    let plan = partition(parse_csv(CSV_NAME, COMPOSED_CSV, Grain::Daily).unwrap());
    let (first, last) = plan.loadable_span().expect("the loadable set is non-empty");

    assert!(
        last < USDC_ORACLE_EPOCH_S,
        "the newest loaded day must sit strictly below the instant our own \
         oracle takes over — the two populations must never share a KEY"
    );
    assert!(first < last, "the span must span");
    // 2021-01-25 00:00:00 UTC — the composer's first day.
    assert_eq!(first, 1_611_532_800);
}

/// Review IN-01: the two populations share no key, but they DO share one daily
/// bucket. The epoch is 14:00 UTC and the composed series is stamped at 00:00,
/// so the last loadable row is the START of the epoch day, and the 1d bucket of
/// that day holds one `external` row and every `oracle` poll from 14:00 on.
/// That is the ONE bucket in production on which the read path's oracle-first
/// rank does real work — not a hypothetical, and not "the populations do not
/// overlap". Pinned here so the prose in the views, the runbook and the task
/// file cannot drift back to claiming they do not.
#[test]
fn the_last_loadable_day_is_the_epoch_day_itself_so_one_daily_bucket_holds_both() {
    let plan = partition(parse_csv(CSV_NAME, COMPOSED_CSV, Grain::Daily).unwrap());
    let (_, last) = plan.loadable_span().unwrap();
    let epoch_day_start = USDC_ORACLE_EPOCH_S - USDC_ORACLE_EPOCH_S % 86_400;
    assert_eq!(
        last, epoch_day_start,
        "the last loadable row is the 00:00 of the epoch day"
    );
    assert!(
        USDC_ORACLE_EPOCH_S > epoch_day_start,
        "and the epoch is not itself midnight, so that day holds polls too"
    );
}

#[test]
fn the_identity_gate_accepts_only_the_asset_this_series_describes() {
    check_identity(CANONICAL_ASSET_CODE, USDC_ISSUER)
        .expect("the composed series is canonical USDC's own history");
    assert!(check_identity("USDT", USDC_ISSUER).is_err());
    assert!(check_identity(CANONICAL_ASSET_CODE, "GSOMEOTHERISSUER").is_err());
}

// ---------------------------------------------------------------------------
// The HOURLY grain (Adam, 2026-09-09). Same file shape, same refusals, same
// epoch partition — 24× the rows and one new falsifier.
// ---------------------------------------------------------------------------

/// The hourly dry-run gate, measured against the versioned file exactly as the
/// daily set above was.
const H_PARSED: usize = 49_176;
const H_LOADABLE: usize = 44_918;
const H_SKIPPED_AT_OR_ABOVE_EPOCH: usize = 4_258;
const H_MEASURED: usize = 44_321;
const H_FALLBACK: usize = 597;
/// ⚠️ **Zero**, and that is not an omission. `measured-disputed` is the
/// composer's verdict on a DAILY cross-check between feeds (task 0265's
/// `xcheck_spread_bps`); it has no hourly analogue, so the four disputed days
/// appear at hourly grain as plain `measured`. Asserted as an absence rather
/// than left unmentioned, because a reader comparing the two figure sets will
/// otherwise assume rows went missing.
const H_MEASURED_DISPUTED: usize = 0;
const H_CHAINLINK_LOADABLE: usize = 44_321;
const H_BITSTAMP_LOADABLE: usize = 597;
const H_CHAINLINK_PARSED: usize = 48_579;
const H_BITSTAMP_PARSED: usize = 597;
/// 2026-03-11 13:00:00 UTC — the last full hour strictly below the 14:00 epoch.
const H_LAST_LOADABLE_S: u32 = 1_773_234_000;

#[test]
fn the_versioned_hourly_csv_is_present_and_non_empty() {
    assert!(
        COMPOSED_CSV_1H.len() > 2_000_000,
        "the hourly composed CSV looks truncated ({} bytes)",
        COMPOSED_CSV_1H.len()
    );
    assert!(
        COMPOSED_CSV_1H.lines().count() > H_PARSED,
        "expected a header plus {H_PARSED} data rows"
    );
}

/// 🔑 The hourly half of BRIEF acceptance criterion 2 — the figure set the
/// runbook makes an operator check before the hourly load, pinned so it cannot
/// drift silently away from the tool that prints it.
#[test]
fn the_dry_run_over_the_versioned_hourly_csv_reports_the_runbook_figures() {
    let rows = parse_csv(CSV_NAME_1H, COMPOSED_CSV_1H, Grain::Hourly)
        .expect("the versioned hourly CSV must parse in full at Grain::Hourly");
    assert_eq!(rows.len(), H_PARSED, "data rows parsed");
    assert_eq!(
        rows.iter().filter(|r| r.source == "chainlink").count(),
        H_CHAINLINK_PARSED
    );
    assert_eq!(
        rows.iter().filter(|r| r.source == "bitstamp").count(),
        H_BITSTAMP_PARSED
    );

    let plan = partition(rows);
    assert_eq!(plan.parsed, H_PARSED);
    assert_eq!(plan.loadable.len(), H_LOADABLE);
    assert_eq!(
        plan.skipped_at_or_above_epoch, H_SKIPPED_AT_OR_ABOVE_EPOCH,
        "Decision F applies unchanged at hourly grain: the hourly file also runs \
         to 2026-09-04, and the hours at or above the epoch are ours to measure \
         rather than to import"
    );
    assert_eq!(
        plan.loadable.len() + plan.skipped_at_or_above_epoch,
        H_PARSED,
        "the partition must be exhaustive"
    );

    assert_eq!(plan.by_quality.get("measured"), Some(&H_MEASURED));
    assert_eq!(plan.by_quality.get("fallback"), Some(&H_FALLBACK));
    assert_eq!(
        plan.by_quality.get("measured-disputed").copied(),
        None,
        "see H_MEASURED_DISPUTED: the disputed verdict is a DAILY cross-check \
         and has no hourly analogue"
    );
    assert_eq!(H_MEASURED_DISPUTED, 0);
    assert_eq!(plan.by_source.get("chainlink"), Some(&H_CHAINLINK_LOADABLE));
    assert_eq!(plan.by_source.get("bitstamp"), Some(&H_BITSTAMP_LOADABLE));

    let quality_total: usize = plan.by_quality.values().sum();
    let source_total: usize = plan.by_source.values().sum();
    assert_eq!(quality_total, H_LOADABLE);
    assert_eq!(source_total, H_LOADABLE);

    let (first, last) = plan.loadable_span().unwrap();
    assert_eq!(first, 1_611_532_800, "2021-01-25 00:00 UTC");
    assert_eq!(
        last, H_LAST_LOADABLE_S,
        "the last loadable hour is 13:00 on the epoch day — the epoch is 14:00, \
         and the bound is strict"
    );
    assert!(last < USDC_ORACLE_EPOCH_S);
}

/// 🔑 The hourly falsifier. The DAILY row for 2023-03-11 closes at 0.96812
/// because the peg had largely recovered by 23:00; the hour that actually shows
/// the depeg is 07:00, at **0.8833**. A consumer asking `granularity=1h` for
/// that day gets a flat 0.96812 from a daily-only load and the real trough from
/// an hourly one — which is the whole argument for loading this grain.
#[test]
fn the_hourly_series_carries_the_depeg_trough_at_0700_and_the_day_close_at_2300() {
    let plan = partition(parse_csv(CSV_NAME_1H, COMPOSED_CSV_1H, Grain::Hourly).unwrap());
    let at = |ts: u32| {
        plan.loadable
            .iter()
            .find(|r| r.ts == ts)
            .unwrap_or_else(|| panic!("hour {ts} must be in the LOADABLE set"))
    };

    let trough = at(DEPEG_HOUR_S);
    assert_eq!(
        trough.rate.to_string(),
        "0.8833",
        "2023-03-11 07:00 UTC is the trough hour and the reason this grain is \
         loaded at all"
    );
    assert_eq!(trough.source, "chainlink");
    assert_eq!(trough.quality, "measured");

    let close = at(DEPEG_DAY_START_S + 23 * 3600);
    assert_eq!(
        close.rate.to_string(),
        "0.96812",
        "the 23:00 hour's close IS the daily close — which is what keeps \
         price_usd_series (daily) unchanged once the hourly rows are loaded"
    );
}

/// 🔴 THE SHARED MIDNIGHT KEY, and why the runbook's order is not advisory.
///
/// Both files carry a row at every UTC midnight of their common span, both land
/// under the same `method`, and `prices.usd_rate` keys on
/// (identity, timestamp, method) — so those rows are ONE ReplacingMergeTree key
/// and the higher `version` wins, i.e. whichever grain was loaded LAST.
///
/// Their VALUES are not the same: the daily row carries the DAY's close, the
/// hourly row the 00:00 HOUR's close. On the versioned files they disagree at
/// 1 980 of 2 049 shared midnights. There is nothing to reconcile — the two
/// numbers answer different questions — so the load order decides, and the
/// correct order is DAILY FIRST, HOURLY SECOND:
///
///   * `price_usd_series` (daily) argMaxes over every rate row in the day, so
///     with hourly rows present it lands on 23:00, whose close equals the daily
///     close for ALL 2 049 days (asserted here). The daily surface is therefore
///     unchanged by the hourly load.
///   * `price_usd_series_1h` and `/ohlcv` at `1h` resolve at the hour, and the
///     00:00 bucket must carry the 00:00 HOUR's close, not the day's.
///
/// Loading them the other way round breaks the second bullet and fixes nothing.
#[test]
fn the_two_grains_share_every_midnight_key_and_the_hourly_row_must_win_it() {
    use std::collections::BTreeMap;

    let daily: BTreeMap<u32, String> =
        partition(parse_csv(CSV_NAME, COMPOSED_CSV, Grain::Daily).unwrap())
            .loadable
            .iter()
            .map(|r| (r.ts, r.rate.to_string()))
            .collect();
    let hourly_rows = parse_csv(CSV_NAME_1H, COMPOSED_CSV_1H, Grain::Hourly).unwrap();
    let hourly: BTreeMap<u32, String> = hourly_rows
        .iter()
        .map(|r| (r.ts, r.rate.to_string()))
        .collect();

    // Every loadable daily key is also an hourly key — the collision is total,
    // not incidental.
    let shared: Vec<u32> = daily
        .keys()
        .copied()
        .filter(|k| hourly.contains_key(k))
        .collect();
    assert_eq!(
        shared.len(),
        daily.len(),
        "every daily midnight must also exist in the hourly file"
    );
    assert!(shared.len() > 1_800, "not a vacuous comparison");

    let differing = shared.iter().filter(|k| daily[k] != hourly[k]).count();
    assert!(
        differing > 1_500,
        "the two grains genuinely disagree at the shared key ({differing} of {}) — \
         if this ever became 0, the load order would stop mattering and this \
         test's whole rationale would need revisiting",
        shared.len()
    );

    // And the property that makes "hourly last" safe: the 23:00 close IS the
    // daily close, every day, so the daily view is indifferent to the swap.
    let mut checked = 0usize;
    for (&day, day_close) in &daily {
        let last_hour = day + 23 * 3600;
        let h = hourly
            .get(&last_hour)
            .unwrap_or_else(|| panic!("no 23:00 row for day {day}"));
        assert_eq!(
            h, day_close,
            "day {day}: the daily close must equal the 23:00 hourly close, or \
             loading hourly rows would MOVE the daily surface"
        );
        checked += 1;
    }
    assert_eq!(checked, daily.len());
}

/// The grain is a GATE, not a hint: pointing the tool at the hourly file while
/// saying `--grain daily` refuses on the first non-midnight row rather than
/// loading a quarter of the file. A grain mismatch is an operator running the
/// wrong command.
#[test]
fn the_hourly_file_is_refused_at_daily_grain_and_the_daily_file_parses_at_both() {
    let err = parse_csv(CSV_NAME_1H, COMPOSED_CSV_1H, Grain::Daily).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("01:00:00"),
        "must name the offending row: {msg}"
    );
    assert!(msg.contains("00:00:00 UTC"), "{msg}");

    // The converse is NOT symmetric and must not be: midnight IS a full hour,
    // so the daily file parses at hourly grain too. That is why `--grain`
    // defaults to daily and why the runbook names the file beside the flag.
    let rows = parse_csv(CSV_NAME, COMPOSED_CSV, Grain::Hourly)
        .expect("every midnight is also a full hour");
    assert_eq!(rows.len(), PARSED);
}
