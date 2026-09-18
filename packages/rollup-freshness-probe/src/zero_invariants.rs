//! The two stored-data invariants of the `close_usd = 0` sentinel (ADR 0292 §6).
//!
//! `close_usd` is a non-nullable `DEFAULT 0` column whose zero means "no USD
//! value", and since ADR 0287 a candle with no price-forming fill stores
//! `close = 0` beside `pf_trade_count = 0`. Every reader of those columns is
//! guarded (the inventory is `docs/database-schema/close-usd-zero-guardrails.md`),
//! and every guard assumes the WRITERS kept two things true:
//!
//! 1. `pf_trade_count = 0 ⇒ close = 0` — a candle that formed no price carries
//!    none. A row breaking it is published as a price by every surface that
//!    gates on `close`/`close_usd` rather than on the count (`views.sql`,
//!    `current.sql`), and refused by `/ohlcv`, which gates on the count.
//! 2. `close_usd > 0 ⇒ close > 0` — a USD close is `rate × close`, so it cannot
//!    exist without one. A row breaking it lends a coarse bucket a rate computed
//!    against a zero.
//!
//! Tests pin the writers we know about. This pins the DATA, which is what also
//! catches the writer nobody listed: an operator's `INSERT … SELECT`, a pre-roll
//! script, a future statement that omits a column and silently takes its
//! DEFAULT (the writer is name-routed — ADR 0287's `pf_trade_count` trap).
//!
//! ⚠️ **A scheduled assertion, not a ClickHouse `CHECK` constraint** — ADR 0292
//! rejected the constraint: a violated CHECK fails the whole insert, so one bad
//! row would stall a refreshable MV's tier and turn a data defect into a
//! freshness incident.
//!
//! ⚠️ **Exact zero, not the precision floor, on purpose.** Invariant 2 reads
//! `close = 0`, so a legacy row with `close = 1e-13` and `close_usd > 0` is not
//! counted although every price gate treats it as having no price. Such a row is
//! a pre-0286 residue, not a writer regression: it cannot be written any more
//! (`price_forming` enforces the floor at ingest) and it dies with 0286 phase 3.
//! Counting it would latch the alarm on history nobody can repair in place.
//!
//! ⚠️ **Windowed, on purpose.** Legacy rows written before task 0286 are out of
//! scope until its phase 3 re-ingests them; an all-time scan would also be a
//! full read of the largest table on every probe tick.

use crate::usd_sanity::{SanityMetric, SanityRefusal};

/// CloudWatch metric: candles in the window breaking either invariant.
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
    /// Candles in the window breaking either invariant.
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
                  OR (close_usd > 0 AND close = 0)) AS violations, \
             count() AS scanned \
         FROM {table} FINAL \
         WHERE timestamp >= now() - INTERVAL {lookback} SECOND",
        table = ZERO_INVARIANT_TABLE,
        lookback = ZERO_INVARIANT_LOOKBACK_SECONDS,
    )
}

/// Shape one reading into the datum to publish, or refuse it.
///
/// An empty scan is REFUSED rather than published as a healthy zero — the same
/// rule, and the same [`SanityRefusal::EmptyScan`], as the USD-sanity metrics:
/// the alarm is `NOT_BREACHING` on missing data, so a query that matched
/// nothing must not read as a clean bill of health.
pub fn zero_invariant_metric(counts: &ZeroInvariantCounts) -> Result<SanityMetric, SanityRefusal> {
    if counts.scanned == 0 {
        return Err(SanityRefusal::EmptyScan {
            table: ZERO_INVARIANT_TABLE,
            lookback_seconds: ZERO_INVARIANT_LOOKBACK_SECONDS,
        });
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
    fn the_query_asserts_both_invariants() {
        let sql = zero_invariant_query();
        assert!(
            sql.contains("pf_trade_count = 0 AND close != 0"),
            "a candle that formed no price must carry none: {sql}"
        );
        assert!(
            sql.contains("close_usd > 0 AND close = 0"),
            "a USD close cannot exist without a close: {sql}"
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
        assert_eq!(
            refusal,
            SanityRefusal::EmptyScan {
                table: ZERO_INVARIANT_TABLE,
                lookback_seconds: ZERO_INVARIANT_LOOKBACK_SECONDS,
            }
        );
    }
}
