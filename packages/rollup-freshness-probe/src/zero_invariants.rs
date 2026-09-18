//! The three stored-data invariants of the `close_usd = 0` sentinel (ADR 0292 §6).
//!
//! `close_usd` is a non-nullable `DEFAULT 0` column whose zero means "no USD
//! value", and since ADR 0287 a candle with no price-forming fill stores
//! `close = 0` beside `pf_trade_count = 0`. Every reader of those columns is
//! guarded (the inventory is `docs/database-schema/close-usd-zero-guardrails.md`),
//! and every guard assumes the WRITERS kept three things true:
//!
//! 1. `pf_trade_count = 0 ⇒ close = 0` — a candle that formed no price carries
//!    none. A row breaking it is published as a price by every surface that
//!    gates on `close`/`close_usd` rather than on the count (`views.sql`,
//!    `current.sql`), and refused by `/ohlcv`, which gates on the count.
//! 2. `close_usd > 0 ⇒ close > 0` — a USD close is `rate × close`, so it cannot
//!    exist without one. A row breaking it lends a coarse bucket a rate computed
//!    against a zero.
//! 3. `pf_trade_count > 0 ⇒ close > 0` — a candle that claims price-forming
//!    fills carries the price they formed. This is the one that catches the
//!    writer defect ADR 0287 warns about: a statement that OMITS
//!    `pf_trade_count` takes its DEFAULT (`trade_count`), so a dust-only minute
//!    is stored as `close = 0, pf_trade_count = 5` — invisible to invariant 1
//!    (which needs the count to be 0) and to invariant 2 (which needs a USD
//!    close). The writer is name-routed, so the omission is silent everywhere
//!    else too.
//!
//! Tests pin the writers we know about. This pins the DATA, which is what also
//! catches the writer nobody listed — **at the live tip only**. The window below
//! filters on the candle's BUCKET time, not on when the row was written (the
//! table has no insert-time column), so an operator's `INSERT … SELECT`, a
//! pre-roll script or a backfill writing buckets older than the window is
//! outside this scan. A historical rewrite needs its own assertion over the
//! partitions it touched; this check does not stand in for one.
//!
//! ⚠️ **A scheduled assertion, not a ClickHouse `CHECK` constraint** — ADR 0292
//! rejected the constraint: a violated CHECK fails the whole insert, so one bad
//! row would stall a refreshable MV's tier and turn a data defect into a
//! freshness incident.
//!
//! ⚠️ **Exact zero, not the precision floor, on purpose.** Invariants 2 and 3
//! read `close = 0`, so a legacy row with `close = 1e-13` and `close_usd > 0` is not
//! counted although every price gate treats it as having no price. Such a row is
//! a pre-0286 residue, not a writer regression: it cannot be written any more
//! (`price_forming` enforces the floor at ingest) and it dies with 0286 phase 3.
//! Counting it would latch the alarm on history nobody can repair in place.
//!
//! ⚠️ **Windowed, on purpose.** Legacy rows written before task 0286 are out of
//! scope until its phase 3 re-ingests them; an all-time scan would also be a
//! full read of the largest table on every probe tick.

use crate::usd_sanity::SanityMetric;

/// CloudWatch metric: candles in the window breaking any of the invariants.
pub const ZERO_INVARIANT_METRIC: &str = "CandleZeroInvariantViolations";

/// The tier the ingest and the enrichment worker write. The coarse tiers are
/// derived from it through the gated rollups, so a violation is caught where it
/// is made rather than six times over.
pub const ZERO_INVARIANT_TABLE: &str = "price_ohlcv_1m";

/// Two days, the same window as [`crate::usd_sanity::PEG_LOOKBACK_SECONDS`] and
/// for the same reason: it is the tier cleanup may drop, and a violation is wrong
/// the instant it is written — waiting cannot improve it.
pub const ZERO_INVARIANT_LOOKBACK_SECONDS: i64 = 2 * 86_400;

/// One reading of [`ZERO_INVARIANT_TABLE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct ZeroInvariantCounts {
    /// Candles in the window breaking any of the invariants.
    pub violations: u64,
    /// Candles examined. Zero means the scan measured nothing.
    pub scanned: u64,
}

/// The assertion query. `FINAL`, because a corrected row is re-inserted at a
/// higher `version` and the alarm must stop once the data is fixed.
pub fn zero_invariant_query() -> String {
    format!(
        "SELECT \
             countIf((pf_trade_count = 0 AND close != 0) \
                  OR (close_usd > 0 AND close = 0) \
                  OR (pf_trade_count > 0 AND close = 0)) AS violations, \
             count() AS scanned \
         FROM {table} FINAL \
         WHERE timestamp >= now() - INTERVAL {lookback} SECOND",
        table = ZERO_INVARIANT_TABLE,
        lookback = ZERO_INVARIANT_LOOKBACK_SECONDS,
    )
}

/// Why a reading was not published.
///
/// Its own type, NOT [`crate::usd_sanity::SanityRefusal`]: that one's
/// `EmptyScan` message blames the USDT identity and task 0139, and this check
/// resolves no quote leg at all. A refusal that names the wrong cause sends the
/// operator to the asset registry for an ingestion outage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZeroInvariantRefusal {
    /// The window held no candle at all.
    EmptyScan,
}

impl std::fmt::Display for ZeroInvariantRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyScan => write!(
                f,
                "no candle of any pair was found in the last {} s of {} — ingestion has \
                 stopped or the table is empty (this check is not scoped to a quote leg, so \
                 the asset registry is not the place to look); refusing to publish a healthy \
                 zero over a scan that measured nothing",
                ZERO_INVARIANT_LOOKBACK_SECONDS, ZERO_INVARIANT_TABLE
            ),
        }
    }
}

impl std::error::Error for ZeroInvariantRefusal {}

/// Shape one reading into the datum to publish, or refuse it.
///
/// An empty scan is REFUSED rather than published as a healthy zero — the same
/// rule as the USD-sanity metrics: the alarm is `NOT_BREACHING` on missing
/// data, so a query that matched nothing must not read as a clean bill of
/// health.
pub fn zero_invariant_metric(
    counts: &ZeroInvariantCounts,
) -> Result<SanityMetric, ZeroInvariantRefusal> {
    if counts.scanned == 0 {
        return Err(ZeroInvariantRefusal::EmptyScan);
    }
    Ok(SanityMetric {
        name: ZERO_INVARIANT_METRIC,
        value: counts.violations as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_asserts_all_three_invariants() {
        let sql = zero_invariant_query();
        assert!(
            sql.contains("pf_trade_count = 0 AND close != 0"),
            "a candle that formed no price must carry none: {sql}"
        );
        assert!(
            sql.contains("close_usd > 0 AND close = 0"),
            "a USD close cannot exist without a close: {sql}"
        );
        assert!(
            sql.contains("pf_trade_count > 0 AND close = 0"),
            "a candle claiming price-forming fills must carry a price: {sql}"
        );
    }

    #[test]
    fn the_query_reads_the_written_tier_deduplicated_and_windowed() {
        let sql = zero_invariant_query();
        assert!(sql.contains("FROM price_ohlcv_1m FINAL"), "{sql}");
        assert!(
            sql.contains("timestamp >= now() - INTERVAL 172800 SECOND"),
            "{sql}"
        );
        assert!(sql.contains("count() AS scanned"), "{sql}");
    }

    #[test]
    fn violations_are_published_as_a_count() {
        let metric = zero_invariant_metric(&ZeroInvariantCounts {
            violations: 3,
            scanned: 10_000,
        })
        .unwrap();
        assert_eq!(metric.name, ZERO_INVARIANT_METRIC);
        assert_eq!(metric.value, 3.0);
    }

    #[test]
    fn a_scan_that_examined_nothing_is_refused_not_published_as_healthy() {
        let refusal = zero_invariant_metric(&ZeroInvariantCounts {
            violations: 0,
            scanned: 0,
        })
        .unwrap_err();
        assert_eq!(refusal, ZeroInvariantRefusal::EmptyScan);
    }

    /// The USD-sanity refusal it used to borrow told the operator the USDT
    /// identity had drifted (task 0139). This check resolves no leg.
    #[test]
    fn the_empty_scan_refusal_does_not_blame_a_quote_leg() {
        let message = ZeroInvariantRefusal::EmptyScan.to_string();
        assert!(!message.contains("USDT"), "{message}");
        assert!(!message.contains("0139"), "{message}");
        assert!(message.contains("price_ohlcv_1m"), "{message}");
        assert!(message.contains("ingestion"), "{message}");
    }
}
