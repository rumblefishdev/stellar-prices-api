//! `current_prices` writer liveness (task 0243).
//!
//! `prices.current_prices` has exactly one writer, the refreshable MV
//! `mv_current_prices` (`REFRESH EVERY 1 MINUTE`, REPLACE mode — see
//! `schema/current.sql`). If that writer stops, the table keeps its last rows and
//! `GET /price` keeps answering HTTP 200 with a price that has silently stopped
//! moving. Nothing else in this crate can see it: the rollup tiers are fed by
//! ingestion and the rollup MVs, not by this one.
//!
//! This check reads how long ago the table was last rewritten and republishes it
//! as [`crate::METRIC_NAME`] with `Table = current_prices`, which the
//! `prices-{env}-current-prices-freshness` alarm watches.
//!
//! ## It watches the writer, not its input
//!
//! `updated_at` is `now()` at the START of each refresh, stamped on every row
//! whether or not any price moved. An ingestion stall therefore does **not** age
//! it — the MV keeps re-deriving the same prices from stale candles — and
//! `rollup-freshness-1m` is the alarm for that: it fired during the 2026-09-14
//! Galexie stall while this signal, by construction, stayed fresh. What ages it
//! is a writer that stopped: a dropped view, a stopped view, or a refresh that
//! fails every cycle.
//!
//! ## Why it is not an eighth entry in [`crate::ROLLUP_TIERS`]
//!
//! - The tiers' empty-tier sentinel rule is positional (fine → coarse); this
//!   table has no place in that order.
//! - The table has no `timestamp` column; its age column is `updated_at`.
//! - Empty means something different here (below).
//! - A separate read keeps a failure here from costing the tier metrics, and vice
//!   versa — the invariant at the top of the handler in `main.rs`.
//!
//! ## Why empty breaches, and why there is no `HAVING`
//!
//! The MV emits one row per asset with a 1-minute candle in the last 24 h, and
//! REPLACE mode makes the table exactly that output — so an empty
//! `current_prices` means the API serves nothing. Unlike a freshly-provisioned
//! rollup tier, that is never healthy. The query therefore has no
//! `HAVING count() > 0` gate: an aggregate without `GROUP BY` returns one row even
//! over zero rows, `row_count` tells the two cases apart, and an empty table
//! publishes [`crate::EMPTY_TIER_SENTINEL_SECONDS`] instead of the ~56-year epoch
//! age that `max()` over nothing yields.
//!
//! ## Why no `FINAL`
//!
//! The table is `ReplacingMergeTree(updated_at)`: its version column **is** the
//! column measured. `FINAL` keeps each key's newest row, and the table's newest
//! row is one of those, so `max(updated_at)` cannot differ with or without it.
//! Task 0243's sketch proposed `FINAL`; this is the correction, pinned by an
//! integration test against the engine.
//!
//! ## No `system.*`
//!
//! The probe connects as `prices_writer`, which holds no `system.view_refreshes`
//! grant and cannot be given one (an XML-defined user). Reading the table needs
//! nothing beyond `SELECT ON prices.*`. The table has no partition key, so this is
//! a column read rather than a part-metadata answer — trivial at one row per
//! asset.

use crate::{EMPTY_TIER_SENTINEL_SECONDS, Metric};

/// Table name, unqualified like every other probe query (the client is bound to
/// the `prices` database), and the value of the published `Table` dimension.
pub const CURRENT_PRICES_TABLE: &str = "current_prices";

/// `REFRESH EVERY 1 MINUTE` in `schema/current.sql`.
pub const REFRESH_INTERVAL_SECONDS: i64 = 60;

/// Longest single refresh the rollout runbook accepts before it says stop
/// (`docs/runbooks/0072-current-prices-mv-rollout.md`). Measured refreshes run
/// 85–267 ms (tasks 0072, 0135); this is the ceiling, not the expectation.
pub const WORST_ACCEPTED_REFRESH_SECONDS: i64 = 40;

/// The oldest a healthy `current_prices` reads: a full interval since the last
/// refresh started, plus the longest refresh the runbook accepts. Mirrored by
/// `CURRENT_PRICES_HEALTHY_PEAK_SECONDS` in `infra/src/lib/types.ts`, which
/// rejects any threshold at or below it at synth.
pub const HEALTHY_PEAK_SECONDS: i64 = REFRESH_INTERVAL_SECONDS + WORST_ACCEPTED_REFRESH_SECONDS;

/// Alarm threshold: 15 missed refreshes, the bound the `1m` tier uses at the same
/// 15-minute probe cadence.
///
/// ⚠️ Documentation and a test fixture, like the bounds in
/// [`crate::ROLLUP_TIERS`]. The deployed threshold is
/// `config.opsAlarms.currentPricesFreshnessSeconds`. Change both, or neither.
pub const AGE_BOUND_SECONDS: i64 = 900;

/// One reading of `current_prices`: how many rows it holds and how long ago its
/// newest row was written.
///
/// ⚠️ Field order must match the SELECT order in [`current_prices_age_query`] —
/// RowBinary is positional.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct CurrentPricesAge {
    pub row_count: u64,
    pub age_seconds: i64,
}

/// The query behind [`CurrentPricesAge`]. Server-side `now()`, so Lambda clock
/// skew cannot move the answer.
pub fn current_prices_age_query() -> &'static str {
    "SELECT count() AS row_count, \
     toInt64(toUnixTimestamp(now()) - toUnixTimestamp(max(updated_at))) AS age_seconds \
     FROM current_prices"
}

/// The datum to publish for one reading. Every successful read yields exactly
/// one — the age, or [`EMPTY_TIER_SENTINEL_SECONDS`] when the table is empty —
/// never nothing, which is what lets the alarm read missing data as a dead probe.
pub fn current_prices_metric(age: &CurrentPricesAge) -> Metric {
    let value = if age.row_count == 0 {
        EMPTY_TIER_SENTINEL_SECONDS
    } else {
        age.age_seconds
    };
    Metric {
        table: CURRENT_PRICES_TABLE.to_string(),
        value: value as f64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ROLLUP_TIERS, freshness_query};

    #[test]
    fn a_populated_table_publishes_its_age_verbatim() {
        let m = current_prices_metric(&CurrentPricesAge {
            row_count: 4444,
            age_seconds: 37,
        });
        assert_eq!(
            m,
            Metric {
                table: "current_prices".to_string(),
                value: 37.0,
            }
        );
    }

    #[test]
    fn an_empty_table_publishes_the_sentinel_not_the_epoch_age() {
        // max() over zero rows is 1970-01-01, a ~56-year age (measured on
        // 26.3.10.60). Published as-is it would breach too, but it would look
        // like a real measurement; the sentinel is unmistakably synthetic.
        let m = current_prices_metric(&CurrentPricesAge {
            row_count: 0,
            age_seconds: 1_786_526_859,
        });
        assert_eq!(m.value, EMPTY_TIER_SENTINEL_SECONDS as f64);
        assert!(
            m.value > AGE_BOUND_SECONDS as f64,
            "an empty table must breach"
        );
    }

    // Checked at compile time rather than in a #[test]: both sides are
    // constants, and a runtime assert on constants is clippy's
    // assertions_on_constants.
    const _: () = assert!(
        AGE_BOUND_SECONDS > HEALTHY_PEAK_SECONDS,
        "the alarm bound must exceed the healthy peak, or it fires on a healthy table"
    );

    #[test]
    fn the_query_reads_updated_at_without_final_having_or_system_tables() {
        let q = current_prices_age_query();
        assert!(q.contains("count() AS row_count"), "{q}");
        assert!(q.contains("max(updated_at)"), "{q}");
        assert!(q.contains("FROM current_prices"), "{q}");
        for forbidden in ["FINAL", "HAVING", "system.", "max(timestamp)"] {
            assert!(!q.contains(forbidden), "must not contain {forbidden}: {q}");
        }
    }

    #[test]
    fn current_prices_is_not_a_rollup_tier() {
        assert!(ROLLUP_TIERS.iter().all(|t| t.table != CURRENT_PRICES_TABLE));
        assert!(!freshness_query().contains(CURRENT_PRICES_TABLE));
    }
}
