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
    ACCEPTED_QUALITIES, ACCEPTED_SOURCES, CANONICAL_ASSET_CODE, DEPEG_DAY_START_S, check_identity,
    parse_csv, partition,
};
use prices_clickhouse::{USDC_ISSUER, USDC_ORACLE_EPOCH_S};

/// Anchored to the CRATE, not to the working directory — `cargo test` runs from
/// the workspace root and `cargo test -p …` from wherever the caller stood.
const COMPOSED_CSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lore/1-tasks/archive/0265_FEATURE_price-usdc-from-measurement-not-the-peg/data/composed_usdc_usd_1d.csv"
));

/// The name the refusals carry; the bin passes the real path.
const CSV_NAME: &str = "composed_usdc_usd_1d.csv";

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
    let rows =
        parse_csv(CSV_NAME, COMPOSED_CSV).expect("the versioned composed CSV must parse in full");
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
    let plan = partition(parse_csv(CSV_NAME, COMPOSED_CSV).unwrap());
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
    let plan = partition(parse_csv(CSV_NAME, COMPOSED_CSV).unwrap());
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
    let plan = partition(parse_csv(CSV_NAME, COMPOSED_CSV).unwrap());
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
