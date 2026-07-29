//! Write-amplification probe (task 0133).
//!
//! The guardrail that would have caught task 0132 in minutes instead of weeks.
//! 0132 was a 9,413× write amplification: the live ledger-processor re-emitted
//! the whole `prices.assets` registry every reconcile, writing ~130M rows/hour
//! to a ~200k-row table and billing ~$337/mo of AWS→Hetzner egress. Nothing
//! watched write volume, so it ran undetected until the BE team found it in
//! `system.part_log`.
//!
//! This scheduled Lambda closes that gap. Once an hour it asks prod ClickHouse
//! how many rows were written to each `prices.*` table in the trailing hour
//! (from `system.part_log`), and republishes the **maximum across all tables**
//! as the custom metric [`METRIC_NAME`] under the [`METRIC_NAMESPACE`] namespace.
//! A CloudWatch alarm fires when that max exceeds an operator-tunable threshold
//! (default well above the busiest legitimate table, far below a 0132-class
//! runaway) → the existing 0056 SNS topic → Slack `#stellar-prices-api-bot`.
//!
//! The window is evaluated **server-side** (`event_time >= now() - INTERVAL 1
//! HOUR`), so it is immune to clock skew between the Lambda and ClickHouse.
//!
//! ## Why "max rows written", not a written÷real *ratio*
//!
//! Amplification is conceptually `rows_written / real_rows`, but the probe reads
//! as `prices_reader`, which is granted `SELECT` on `system.part_log` (the write
//! log) and `prices.*` — **not** `system.parts` (the real-row snapshot). A
//! per-table ratio would need that second system grant, so v1 alarms on the
//! absolute rows-written-per-hour instead: the busiest legitimate table writes
//! well under 1M rows/hour, while 0132 wrote ~130M/hour, so an absolute
//! threshold with wide margin is a perfectly good guardrail. A true ratio is a
//! documented future enhancement (see the task) that would add a `system.parts`
//! read grant.
//!
//! ## A quiet window is healthy
//!
//! [`WRITE_VOLUME_QUERY`] returns a row only for a table actually written to in
//! the last hour. After the 0132 fix, `assets` writes nothing on an idle hour,
//! so it simply produces no row; [`max_rows_written`] returns `0.0` and the
//! alarm stays OK. That is the correct healthy steady state, not missing data.
//!
//! ## Real tables only
//!
//! The `table NOT LIKE '.%'` filter drops ClickHouse's internal storage tables
//! (`.inner_id.*` materialized-view targets, `.tmp.*` merge scratch) so the
//! metric tracks the real `prices.*` surface and its `Table`-space stays bounded
//! (no UUID-named dimensions).
//!
//! Split for testability: the pure metric-shaping ([`max_rows_written`]) and the
//! query text are compiled in every build and unit-tested without the AWS SDK;
//! the actual CloudWatch publish ([`publish`]) is gated behind the `lambda`
//! feature.

/// CloudWatch namespace for the write-volume metric. Must match the
/// `cloudwatch:namespace` condition on the Lambda role's `PutMetricData` grant
/// and the alarm wiring in `infra/`.
pub const METRIC_NAMESPACE: &str = "Prices/Ingest";

/// Custom metric name: the maximum rows-written-per-hour across all real
/// `prices.*` tables. A single scalar per run (dimensioned only by
/// `Environment`) so the alarm is a trivial threshold and never fans out over a
/// growing set of table dimensions.
pub const METRIC_NAME: &str = "MaxRowsWrittenPerHour";

/// The trailing window (hours) summed by [`WRITE_VOLUME_QUERY`]. It is baked into
/// the SQL literal (`INTERVAL 1 HOUR`), so this const is the single documented
/// source of truth for the value and the anchor for the coupling note on the
/// query: the EventBridge schedule and the alarm metric period must both equal
/// this. If it ever needs to change, update the SQL, the schedule
/// (`scheduleExpressions.writeAmplificationProbe`), and the alarm period together.
pub const WINDOW_HOURS: u32 = 1;

/// One table's rows-written total over the trailing window, as read from
/// `system.part_log`. `sum(rows)` over `NewPart` events is a `UInt64`, so this
/// deserializes into a plain `u64`; a table not written to in the window simply
/// produces no row (see [`WRITE_VOLUME_QUERY`]).
#[derive(Debug, Clone, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct TableWrite {
    pub table: String,
    pub rows_written: u64,
}

/// The metric value for one run: the maximum rows-written across all reported
/// tables. `0.0` when the window was quiet (no rows returned) — the healthy
/// post-0132 steady state, not missing data.
pub fn max_rows_written(rows: &[TableWrite]) -> f64 {
    rows.iter().map(|r| r.rows_written).max().unwrap_or(0) as f64
}

/// SQL that sums rows written to each real `prices.*` table over the trailing
/// hour from `system.part_log`.
///
/// - **`event_type = 'NewPart'`** — count only part *creations* (the inserts /
///   merged outputs that actually cross the wire and land as writes), not
///   merges/mutations/removals.
/// - **`event_time >= now() - INTERVAL 1 HOUR`** — evaluated server-side, so the
///   window is immune to Lambda↔CH clock skew (mirrors the freshness probe).
/// - **`table NOT LIKE '.%'`** — drop ClickHouse-internal tables (`.inner_id.*`
///   MV storage, `.tmp.*` merge scratch) so the metric tracks the real surface
///   and the `Table` space stays bounded.
///
/// Ordered by volume so the caller can log the top writers for forensics when
/// the alarm fires.
///
/// ⚠️ **This trailing-hour window is coupled to two infra values and they must
/// stay in lockstep — there is no compile-time link:**
/// 1. the EventBridge schedule `scheduleExpressions.writeAmplificationProbe`
///    (must be `rate(1 hour)`), so runs neither overlap (double-count) nor gap
///    the window, and
/// 2. the alarm metric `period` in `observability-stack.ts` (must be 1 hour),
///    so the threshold is compared against a whole window's writes.
///
/// Changing the window here **requires** changing both. See [`WINDOW_HOURS`].
pub const WRITE_VOLUME_QUERY: &str = "SELECT \
     table, \
     sum(rows) AS rows_written \
   FROM system.part_log \
   WHERE database = 'prices' \
     AND event_type = 'NewPart' \
     AND event_time >= now() - INTERVAL 1 HOUR \
     AND table NOT LIKE '.%' \
   GROUP BY table \
   ORDER BY rows_written DESC";

/// Publish the max rows-written value to CloudWatch under [`METRIC_NAMESPACE`]
/// as [`METRIC_NAME`], tagged with an `Environment` dimension. One
/// `PutMetricData` call. Always publishes (including `0.0`) so the alarm has a
/// fresh datum every run and a quiet hour reads as a real zero, not missing
/// data.
#[cfg(feature = "lambda")]
pub async fn publish(
    client: &aws_sdk_cloudwatch::Client,
    environment: &str,
    max_rows: f64,
) -> Result<(), aws_sdk_cloudwatch::Error> {
    use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};

    let datum = MetricDatum::builder()
        .metric_name(METRIC_NAME)
        .value(max_rows)
        .unit(StandardUnit::Count)
        .dimensions(
            Dimension::builder()
                .name("Environment")
                .value(environment)
                .build(),
        )
        .build();

    client
        .put_metric_data()
        .namespace(METRIC_NAMESPACE)
        .metric_data(datum)
        .send()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(table: &str, rows: u64) -> TableWrite {
        TableWrite {
            table: table.to_string(),
            rows_written: rows,
        }
    }

    #[test]
    fn max_picks_the_largest_writer() {
        let rows = vec![
            w("price_ohlcv_15m", 715_000),
            w("assets", 130_000_000), // a 0132-class runaway
            w("price_ohlcv_1m", 140_000),
        ];
        assert_eq!(max_rows_written(&rows), 130_000_000.0);
    }

    #[test]
    fn quiet_window_is_zero_not_missing() {
        // Post-0132 healthy steady state: nothing written → no rows → 0.0, so the
        // alarm reads a real zero rather than treating-missing-data.
        assert_eq!(max_rows_written(&[]), 0.0);
    }

    #[test]
    fn legitimate_traffic_stays_well_under_a_runaway() {
        // The busiest legit table (the 15m rollup) is ~0.7M/hour — orders of
        // magnitude below a 0132-class ~130M/hour, so any reasonable threshold
        // between them separates healthy from pathological.
        let rows = vec![w("price_ohlcv_15m", 715_000), w("price_ohlcv_1m", 140_000)];
        let max = max_rows_written(&rows);
        assert!(
            max < 5_000_000.0,
            "legit max {max} should be under a guardrail threshold"
        );
    }

    #[test]
    fn query_targets_the_write_log_correctly() {
        // Reads the write log, counts only part creations, over a server-side
        // trailing hour, for the real prices surface only.
        assert!(WRITE_VOLUME_QUERY.contains("system.part_log"));
        assert!(WRITE_VOLUME_QUERY.contains("database = 'prices'"));
        assert!(WRITE_VOLUME_QUERY.contains("event_type = 'NewPart'"));
        assert!(WRITE_VOLUME_QUERY.contains("now() - INTERVAL 1 HOUR"));
        // Internal storage tables (.inner_id.* / .tmp.*) are excluded so the
        // Table space stays bounded and the metric tracks the real surface.
        assert!(WRITE_VOLUME_QUERY.contains("table NOT LIKE '.%'"));
    }
}
