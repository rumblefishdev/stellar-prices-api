//! Stored candles that are internally inconsistent, on all seven tiers (task 0236).
//!
//! A candle's four prices obey two shapes: `low` and `high` bracket the open and
//! the close, and every price is positive. Since task 0229 the `/ohlcv` read path
//! CLAMPS a crossed candle into shape on its way out (ADR-0011 §3), which is the
//! right thing for a read API to do — and it removed the only place anyone would
//! have noticed a corrupt stored row. This check moves that detector to the
//! SOURCE: it reads the stored rows, not what the API serves.
//!
//! # The predicate — exact, with zero tolerance
//!
//! A priced row (`pf_trade_count > 0`) is a violation when
//!
//! ```text
//! low > least(open, close) OR high < greatest(open, close) OR low > high
//! OR open <= 0 OR high <= 0 OR low <= 0 OR close <= 0
//! ```
//!
//! There is no epsilon. The stored decimals are produced by `min`/`max`/
//! `argMin`/`argMax` over the same fills (and, up the chain, over the tier
//! below), which COPY a value bit for bit — nothing is computed, so nothing can
//! round. Any crossing in storage is a real defect, and the comparisons are plain
//! `Decimal` against integer literals: no `toFloat`, no float path at all.
//!
//! The predicate is split into two **disjoint** classes, so their sum counts ROWS:
//!
//! - `band` — the three band arms, over rows whose four prices are ALL `> 0`;
//! - `nonpositive` — any of the four prices `<= 0`.
//!
//! Their union is exactly the predicate above. Without the positive gate a row
//! like `open = 0, low = 3` would fire both `low > least(open, close)` and
//! `open <= 0` and count twice. `low > high` follows logically from the other two
//! band arms (if `low > high` then either `low > least(o, c)`, or
//! `least(o, c) >= low > high`, and then `greatest(o, c) > high`); it is kept as
//! the literal invariant the reader expects to find.
//!
//! # The window is per tier, on bucket START
//!
//! `timestamp >= now() - 2 days - <one bucket length>` (`_1m` plain 2 days). A
//! bucket of length `L` starting at `s` overlaps the last two days iff
//! `s > now - 2d - L`. A flat 2-day window would NEVER see a `_1w` or `_1M`
//! bucket for most of the week or month — its start is older than two days. The
//! widening is by one bucket, and since weeks are Monday-aligned and month
//! subtraction clamps to the month's end, `now - 1 MONTH` is always at or before
//! the current month's start: the window always reaches the current bucket.
//!
//! ⚠️ **Bucket time, not write time** — as with [`crate::zero_invariants`], the
//! tables carry no insert-time column. A backfill, pre-roll or operator
//! `INSERT … SELECT` that writes buckets older than the window is NOT covered;
//! a historical rewrite needs its own assertion over the partitions it touched.
//!
//! ⚠️ **A scheduled assertion, not a ClickHouse `CHECK` constraint** — for the
//! reason ADR 0292 §6 gives: a violated CHECK fails the whole insert, so one bad
//! row would stall a refreshable MV's tier and turn a data defect into a
//! freshness incident.
//!
//! # Overlap and multiplication — both expected
//!
//! - A priced `_1m` row with `close = 0` is also [`crate::zero_invariants`]
//!   invariant 3, so both alarms fire. `low = 0` beside a positive close is
//!   caught ONLY here — the shape the prod baseline was mostly made of.
//! - The metric sums TIER-ROWS. The rollups carry a bad minute into every coarse
//!   bucket above it, so one defect can count up to seven times. Read the per-tier
//!   detail: a violation on a coarse tier with a clean `_1m` is a ROLLUP defect,
//!   not an ingest one.
//!
//! # Seven reads, not one `UNION ALL`
//!
//! One statement fails as a unit: a dropped or renamed tier, or a lost grant,
//! would take every other tier's count down with it, and the alarm is
//! `NOT_BREACHING` on missing data. Seven independent reads keep the six that
//! work.
//!
//! # When a reading is refused
//!
//! A positive sum is always published — it can never be a false OK. A ZERO is
//! published only when all seven tiers were read AND the `_1m` window was
//! non-empty; otherwise [`OhlcBandRefusal`] says why, and the probe's `-errors`
//! alarm carries it. Coarse tiers with nothing in their window are fine.
//!
//! # Baseline
//!
//! Measured on production 2026-09-28: band violations 0 on every tier, every
//! source, since 2015. Priced rows with a price `<= 0` are all exact zeros and
//! all older than the task 0286 rollout (2026-09-22 12:00 UTC); 0 after it. That
//! legacy residue is outside this window by construction and is re-ingested by
//! 0286 phase 3. A healthy reading is therefore exactly **0**.

use crate::usd_sanity::SanityMetric;

/// CloudWatch metric: stored candles in the per-tier windows that break the band
/// or carry a price `<= 0`, summed over all seven tiers.
pub const OHLC_BAND_METRIC: &str = "CandleBandViolations";

/// Two days, the same base window as
/// [`crate::zero_invariants::ZERO_INVARIANT_LOOKBACK_SECONDS`]; the coarse tiers
/// widen it by one bucket (see [`OHLC_BAND_TIERS`]).
pub const OHLC_BAND_LOOKBACK_SECONDS: i64 = 2 * 86_400;

/// `(table, widening)` per tier, in [`crate::ROLLUP_TIERS`] order (a unit test
/// pins the two together). The widening is one bucket length, as a ClickHouse
/// `INTERVAL` literal; `_1m` is not widened.
pub const OHLC_BAND_TIERS: [(&str, Option<&str>); 7] = [
    ("price_ohlcv_1m", None),
    ("price_ohlcv_15m", Some("15 MINUTE")),
    ("price_ohlcv_1h", Some("1 HOUR")),
    ("price_ohlcv_4h", Some("4 HOUR")),
    ("price_ohlcv_1d", Some("1 DAY")),
    ("price_ohlcv_1w", Some("1 WEEK")),
    ("price_ohlcv_1M", Some("1 MONTH")),
];

/// One tier's reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct OhlcBandCounts {
    /// Priced rows with four positive prices that break the band.
    pub band: u64,
    /// Priced rows with any of the four prices `<= 0`. Disjoint from `band`.
    pub nonpositive: u64,
    /// Rows examined. Zero on `_1m` means the scan measured nothing.
    pub scanned: u64,
}

impl OhlcBandCounts {
    /// Rows breaking either shape. The classes are disjoint, so this is a row
    /// count, not a count of arms.
    pub fn violations(&self) -> u64 {
        self.band + self.nonpositive
    }
}

/// The seven assertion queries, `(table, sql)` in [`OHLC_BAND_TIERS`] order.
///
/// `FINAL`, because a corrected row is re-inserted at a higher `version` and the
/// alarm must stop once the data is fixed. Takes no argument: the SQL is built
/// only from `'static` constants, so no caller-supplied string reaches `format!`.
pub fn ohlc_band_queries() -> Vec<(&'static str, String)> {
    OHLC_BAND_TIERS
        .iter()
        .map(|&(table, widen)| {
            let widening = widen
                .map(|w| format!(" - INTERVAL {w}"))
                .unwrap_or_default();
            let sql = format!(
                "SELECT \
                     countIf(pf_trade_count > 0 AND open > 0 AND high > 0 AND low > 0 AND close > 0 \
                     AND (low > least(open, close) OR high < greatest(open, close) OR low > high)) \
                     AS band, \
                     countIf(pf_trade_count > 0 AND (open <= 0 OR high <= 0 OR low <= 0 OR close <= 0)) \
                     AS nonpositive, \
                     count() AS scanned \
                 FROM {table} FINAL \
                 WHERE timestamp >= now() - INTERVAL {lookback} SECOND{widening}",
                lookback = OHLC_BAND_LOOKBACK_SECONDS,
            );
            (table, sql)
        })
        .collect()
}

/// Why a zero was not published.
///
/// Its own type, NOT [`crate::usd_sanity::SanityRefusal`], for the reason
/// [`crate::zero_invariants::ZeroInvariantRefusal`] gives: that one blames the
/// USDT identity, and this check resolves no quote leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OhlcBandRefusal {
    /// The `_1m` window held no candle at all.
    EmptyScan,
    /// At least one tier's read failed in this run.
    Incomplete,
}

impl std::fmt::Display for OhlcBandRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyScan => write!(
                f,
                "no candle of any pair was found in the last {} s of {} — ingestion has \
                 stopped or the table is empty (this check is not scoped to a quote leg); \
                 refusing to publish a healthy zero over a scan that measured nothing",
                OHLC_BAND_LOOKBACK_SECONDS, OHLC_BAND_TIERS[0].0
            ),
            Self::Incomplete => write!(
                f,
                "fewer than {} tiers were read in this run (each failed tier read is \
                 reported alongside) — a zero would claim tiers nobody measured",
                OHLC_BAND_TIERS.len()
            ),
        }
    }
}

impl std::error::Error for OhlcBandRefusal {}

/// Shape the readings into the one datum to publish, or refuse.
///
/// In order: a positive sum is always published (never a false OK, even with a
/// tier missing); a zero with any tier unread is [`OhlcBandRefusal::Incomplete`];
/// a zero over an empty `_1m` window is [`OhlcBandRefusal::EmptyScan`]; otherwise
/// a healthy zero.
pub fn ohlc_band_metric(
    readings: &[(&'static str, OhlcBandCounts)],
) -> Result<SanityMetric, OhlcBandRefusal> {
    let total: u64 = readings.iter().map(|(_, c)| c.violations()).sum();
    if total > 0 {
        return Ok(SanityMetric {
            name: OHLC_BAND_METRIC,
            value: total as f64,
        });
    }
    let read = |table: &str| readings.iter().find(|(t, _)| *t == table).map(|(_, c)| c);
    if OHLC_BAND_TIERS.iter().any(|(t, _)| read(t).is_none()) {
        return Err(OhlcBandRefusal::Incomplete);
    }
    if read(OHLC_BAND_TIERS[0].0).is_some_and(|c| c.scanned == 0) {
        return Err(OhlcBandRefusal::EmptyScan);
    }
    Ok(SanityMetric {
        name: OHLC_BAND_METRIC,
        value: 0.0,
    })
}

/// One `<table> band=<n> nonpositive=<n> scanned=<n>` segment per reading,
/// joined by `; ` — the per-tier breakdown the alarm text points at.
pub fn ohlc_band_detail(readings: &[(&'static str, OhlcBandCounts)]) -> String {
    readings
        .iter()
        .map(|(t, c)| {
            format!(
                "{t} band={} nonpositive={} scanned={}",
                c.band, c.nonpositive, c.scanned
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ROLLUP_TIERS;

    fn counts(band: u64, nonpositive: u64, scanned: u64) -> OhlcBandCounts {
        OhlcBandCounts {
            band,
            nonpositive,
            scanned,
        }
    }

    /// Seven readings, one per tier, all the given counts.
    fn all_tiers(c: OhlcBandCounts) -> Vec<(&'static str, OhlcBandCounts)> {
        OHLC_BAND_TIERS.iter().map(|(t, _)| (*t, c)).collect()
    }

    #[test]
    fn every_tier_query_asserts_every_predicate_arm() {
        let queries = ohlc_band_queries();
        assert_eq!(queries.len(), 7);
        for (table, sql) in &queries {
            for arm in [
                "low > least(open, close)",
                "high < greatest(open, close)",
                "low > high",
                "open <= 0 OR high <= 0 OR low <= 0 OR close <= 0",
            ] {
                assert!(sql.contains(arm), "{table} is missing `{arm}`: {sql}");
            }
        }
    }

    /// The two classes are disjoint: `band` only over four positive prices, so
    /// a row breaking both shapes (open = 0 with a positive low) counts once,
    /// and both only over priced rows.
    #[test]
    fn the_classes_are_disjoint_and_both_gated_on_a_priced_row() {
        for (table, sql) in ohlc_band_queries() {
            assert!(
                sql.contains(
                    "countIf(pf_trade_count > 0 AND open > 0 AND high > 0 AND low > 0 AND close > 0 \
                     AND (low > least(open, close) OR high < greatest(open, close) OR low > high)) \
                     AS band"
                ),
                "{table}: {sql}"
            );
            assert!(
                sql.contains(
                    "countIf(pf_trade_count > 0 AND (open <= 0 OR high <= 0 OR low <= 0 OR close <= 0)) \
                     AS nonpositive"
                ),
                "{table}: {sql}"
            );
            assert!(sql.contains("count() AS scanned"), "{table}: {sql}");
            assert!(
                !sql.contains("toFloat"),
                "exact Decimal, no float path: {sql}"
            );
        }
    }

    /// BRIEF point 4: the window is on bucket START, widened by one bucket per
    /// tier. One assertion per tier, each pinned to the END of the statement.
    #[test]
    fn each_tier_reads_its_table_final_with_its_own_bucket_window() {
        let expected = [
            (
                "price_ohlcv_1m",
                "timestamp >= now() - INTERVAL 172800 SECOND",
            ),
            (
                "price_ohlcv_15m",
                "timestamp >= now() - INTERVAL 172800 SECOND - INTERVAL 15 MINUTE",
            ),
            (
                "price_ohlcv_1h",
                "timestamp >= now() - INTERVAL 172800 SECOND - INTERVAL 1 HOUR",
            ),
            (
                "price_ohlcv_4h",
                "timestamp >= now() - INTERVAL 172800 SECOND - INTERVAL 4 HOUR",
            ),
            (
                "price_ohlcv_1d",
                "timestamp >= now() - INTERVAL 172800 SECOND - INTERVAL 1 DAY",
            ),
            (
                "price_ohlcv_1w",
                "timestamp >= now() - INTERVAL 172800 SECOND - INTERVAL 1 WEEK",
            ),
            (
                "price_ohlcv_1M",
                "timestamp >= now() - INTERVAL 172800 SECOND - INTERVAL 1 MONTH",
            ),
        ];
        let queries = ohlc_band_queries();
        assert_eq!(queries.len(), expected.len());
        for ((table, sql), (want_table, want_window)) in queries.iter().zip(expected) {
            assert_eq!(*table, want_table);
            assert!(
                sql.contains(&format!("FROM {want_table} FINAL WHERE")),
                "{table} must read FINAL: {sql}"
            );
            assert!(
                sql.ends_with(&format!("WHERE {want_window}")),
                "{table} window: {sql}"
            );
        }
    }

    /// An eighth tier added to `ROLLUP_TIERS` must not go unwatched here.
    #[test]
    fn the_tiers_are_exactly_the_rollup_tiers_in_order() {
        let ours: Vec<&str> = OHLC_BAND_TIERS.iter().map(|(t, _)| *t).collect();
        let rollup: Vec<&str> = ROLLUP_TIERS.iter().map(|t| t.table).collect();
        assert_eq!(ours, rollup);
    }

    #[test]
    fn the_metric_is_the_sum_of_both_classes_over_every_tier() {
        let readings: Vec<(&'static str, OhlcBandCounts)> = OHLC_BAND_TIERS
            .iter()
            .enumerate()
            .map(|(i, (t, _))| (*t, counts(i as u64, 2 * i as u64, 100)))
            .collect();
        // band 0+1+..+6 = 21, nonpositive 2*21 = 42
        let metric = ohlc_band_metric(&readings).unwrap();
        assert_eq!(metric.name, OHLC_BAND_METRIC);
        assert_eq!(metric.name, "CandleBandViolations");
        assert_eq!(metric.value, 63.0);
        assert_eq!(counts(2, 3, 9).violations(), 5);
    }

    #[test]
    fn an_empty_1m_scan_is_refused_not_published_as_healthy() {
        let refusal = ohlc_band_metric(&all_tiers(counts(0, 0, 0))).unwrap_err();
        assert_eq!(refusal, OhlcBandRefusal::EmptyScan);
    }

    #[test]
    fn a_zero_over_a_missing_tier_is_refused_as_incomplete() {
        let mut readings = all_tiers(counts(0, 0, 10));
        readings.remove(3);
        assert_eq!(
            ohlc_band_metric(&readings).unwrap_err(),
            OhlcBandRefusal::Incomplete
        );
    }

    /// A positive value is never a false OK, so a failed tier elsewhere does
    /// not hide the violations the other six found.
    #[test]
    fn a_positive_sum_is_published_even_with_a_tier_missing() {
        let mut readings = all_tiers(counts(0, 0, 10));
        readings.remove(6);
        readings[2].1 = counts(1, 2, 10);
        assert_eq!(ohlc_band_metric(&readings).unwrap().value, 3.0);
    }

    #[test]
    fn empty_coarse_tiers_beside_a_scanned_1m_publish_a_healthy_zero() {
        let mut readings = all_tiers(counts(0, 0, 0));
        readings[0].1 = counts(0, 0, 500);
        assert_eq!(ohlc_band_metric(&readings).unwrap().value, 0.0);
    }

    #[test]
    fn a_coarse_violation_is_published_even_when_1m_scanned_nothing() {
        let mut readings = all_tiers(counts(0, 0, 0));
        readings[5].1 = counts(2, 0, 4);
        assert_eq!(ohlc_band_metric(&readings).unwrap().value, 2.0);
    }

    #[test]
    fn the_empty_scan_refusal_names_the_table_and_not_a_quote_leg() {
        let message = OhlcBandRefusal::EmptyScan.to_string();
        assert!(message.contains("price_ohlcv_1m"), "{message}");
        assert!(message.contains("ingestion"), "{message}");
        assert!(!message.contains("USDT"), "{message}");
        assert!(!message.contains("0139"), "{message}");
    }

    #[test]
    fn the_incomplete_refusal_says_a_tier_read_failed_in_this_run() {
        let message = OhlcBandRefusal::Incomplete.to_string();
        assert!(message.contains("tier"), "{message}");
        assert!(message.contains("read"), "{message}");
        assert!(message.contains("this run"), "{message}");
    }

    #[test]
    fn the_detail_names_every_reading() {
        let detail = ohlc_band_detail(&[
            ("price_ohlcv_1m", counts(1, 2, 6)),
            ("price_ohlcv_1w", counts(0, 0, 0)),
        ]);
        assert_eq!(
            detail,
            "price_ohlcv_1m band=1 nonpositive=2 scanned=6; \
             price_ohlcv_1w band=0 nonpositive=0 scanned=0"
        );
    }
}
