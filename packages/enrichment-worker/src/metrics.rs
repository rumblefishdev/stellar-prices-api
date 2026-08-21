//! Enrichment telemetry — the CloudWatch metrics from the 0024 design spec §5
//! (plus two refinements from task 0026 review findings), derived from a
//! completed pass's [`ChPassStats`].
//!
//! Spec §5 named four metrics; this maps the equivalents plus:
//!   * `EnrichmentRowsRemainingRecent` — the recency-bounded backlog that
//!     excludes the permanent exotic-quote floor, so the stall alarm does not
//!     false-fire on an idle env (finding #5).
//!   * `EnrichmentPassDurationMs` (renamed from the misleading
//!     `EnrichmentBatchDurationMs`) + a derived per-batch
//!     `EnrichmentAvgBatchDurationMs`, so operators size batch/timeout headroom
//!     off a true per-batch figure rather than a whole-pass value that grows
//!     with backlog and one-shot mode (finding #7).
//!
//! The mapping ([`pass_metrics`]) is a pure function compiled in every build, so
//! it is unit-testable without the AWS SDK. The actual publish ([`publish`]) is
//! gated behind the `lambda` feature (it pulls `aws-sdk-cloudwatch`); the
//! default build / prototype CLI never links it. Publish is best-effort: the
//! Lambda logs a warning on failure rather than failing the invocation, so a
//! transient CloudWatch error never blocks enrichment.

use crate::ch_enrich::ChPassStats;
use crate::frontier::SweepSummary;
use crate::repair::CoarseSweepSummary;

/// CloudWatch namespace for all enrichment metrics. Matches the
/// `cloudwatch:namespace` condition on the Lambda role's `PutMetricData` grant
/// and the alarm/dashboard wiring in `infra/`.
pub const METRIC_NAMESPACE: &str = "Prices/Enrichment";

/// CloudWatch unit for a [`Metric`]. Kept minimal — the enrichment metrics are
/// counts, a duration, or a bare identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Count,
    Milliseconds,
    /// Dimensionless. For values that are identifiers rather than quantities —
    /// `EnrichmentFrontierMonth` is a `YYYYMM`, and publishing it as a `Count`
    /// would invite a dashboard to sum or average it into nonsense.
    None,
}

/// One CloudWatch datum: a spec §5 metric name, its value, and unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub name: &'static str,
    pub value: f64,
    pub unit: Unit,
}

/// Map a completed pass's stats to its CloudWatch metrics: the spec §5
/// `EnrichmentRowsEnriched`, `EnrichmentOracleMiss`,
/// `EnrichmentRowsRemainingAtVolumeZero`, plus the task 0026 refinements
/// `EnrichmentRowsRemainingRecent` (finding #5) and `EnrichmentPassDurationMs`
/// / `EnrichmentAvgBatchDurationMs` (finding #7).
pub fn pass_metrics(stats: &ChPassStats) -> Vec<Metric> {
    let mut metrics = vec![
        Metric {
            name: "EnrichmentRowsEnriched",
            value: stats.rows_enriched as f64,
            unit: Unit::Count,
        },
        Metric {
            name: "EnrichmentOracleMiss",
            value: stats.oracle_misses as f64,
            unit: Unit::Count,
        },
        // Full volume-zero backlog — kept for the dashboard / forensic value.
        Metric {
            name: "EnrichmentRowsRemainingAtVolumeZero",
            value: stats.rows_remaining_at_volume_zero as f64,
            unit: Unit::Count,
        },
        // Recency-bounded backlog (excludes the permanent exotic-quote floor) —
        // the series the stall alarm gates on so an idle env reads zero (finding #5).
        Metric {
            name: "EnrichmentRowsRemainingRecent",
            value: stats.rows_remaining_recent as f64,
            unit: Unit::Count,
        },
        // Whole-pass wall-clock (all batches + the FINAL count scans), NOT
        // per-batch — renamed from the misleading `EnrichmentBatchDurationMs`
        // (finding #7).
        Metric {
            name: "EnrichmentPassDurationMs",
            value: stats.duration_ms as f64,
            unit: Unit::Milliseconds,
        },
    ];

    // True per-batch figure for sizing batch/timeout headroom — only meaningful
    // when a batch actually ran (an empty-backlog pass does 0 batches, which
    // would divide by zero and carries no per-batch signal anyway).
    if stats.batches > 0 {
        metrics.push(Metric {
            name: "EnrichmentAvgBatchDurationMs",
            value: stats.duration_ms as f64 / stats.batches as f64,
            unit: Unit::Milliseconds,
        });
    }

    metrics
}

/// Metrics for the recurring coarse-table sweep (task 0114). Published under the
/// **same** [`METRIC_NAMESPACE`] as the 1m pass, so the existing `PutMetricData`
/// grant (scoped to that namespace) covers them without an IAM change.
///
///   * `CoarseSweepRowsEnriched` — coarse rows corrected this run, across all
///     swept tables. Steady-state ≈ 0 once the tables sit at the floor; a
///     sustained non-zero is the actionable signal — the rollup path is
///     re-freezing zeros (enrichment lag exceeded the MV windows — task 0111
///     territory) and the guard is earning its keep.
///   * `CoarseSweepTableFailures` — tables whose pass **errored** this run. The
///     dead-sweep signal to alarm on. Deliberately excludes config-skipped
///     tables (below), so a benign mis-config cannot false-fire the alarm.
///   * `CoarseSweepTablesSkipped` — non-coarse names left in the config
///     (`price_ohlcv_1m` / typos). Visible for hygiene, but a static condition,
///     not a runtime failure — kept off the alarm series.
///
/// A `RowsRemaining` metric is intentionally NOT published: `zeros_after` is
/// dominated by the permanent multi-million exotic `no_reference` floor, so it
/// sits near-constant whether or not the sweep is keeping up and cannot observe
/// the lag it would exist to catch. `RowsEnriched` is the usable signal; the
/// floor size is available on demand via the per-quote-class composition query.
/// Metrics for the frontier-driven historical drain (task 0111 phase 4). Same
/// [`METRIC_NAMESPACE`], so the existing `PutMetricData` grant covers them.
///
/// The point of these is that **drain progress becomes a number instead of an
/// archaeology dig**. Before this, answering "is the backlog moving?" meant
/// hand-querying `query_log` and `system.parts` on prod.
///
///   * `EnrichmentFrontierMonthsPending` — monthly partitions below the live
///     window still believed to hold work. Monotonically falling in a healthy
///     drain; flat for days is the actionable signal.
///   * `EnrichmentFrontierMonth` — the frontier position itself as `YYYYMM`.
///     A gauge you can eyeball against the known span. Published as `None` →
///     omitted rather than 0, so a finished drain does not read as "January
///     year zero".
///   * `EnrichmentHistoricalRowsEnriched` — rows the drain corrected this run.
///     This is the per-leg progress signal the pass itself cannot give: the XLM
///     pivot writes exactly `batch_size` every batch and so masks every other
///     leg's stall in the aggregate count (task 0219).
///   * `EnrichmentFrontierMonthsReopened` — exhausted months that turned out to
///     have gained work and were re-opened. Published only when non-zero.
///   * `EnrichmentHistoricalDeadlineHit` — 1 when the run stopped on its
///     wall-clock budget rather than on `max_months`. Distinguishes "nothing
///     left to do" from "ran out of time", which otherwise look identical from
///     a rows-enriched of zero.
pub fn historical_sweep_metrics(summary: &SweepSummary) -> Vec<Metric> {
    let mut m = vec![
        Metric {
            name: "EnrichmentFrontierMonthsPending",
            value: summary.months_pending as f64,
            unit: Unit::Count,
        },
        Metric {
            name: "EnrichmentHistoricalRowsEnriched",
            value: summary.total_enriched() as f64,
            unit: Unit::Count,
        },
        Metric {
            name: "EnrichmentHistoricalDeadlineHit",
            value: if summary.deadline_hit { 1.0 } else { 0.0 },
            unit: Unit::Count,
        },
    ];
    if summary.months_reopened > 0 {
        // Only published when non-zero: a month re-opening means something wrote
        // into a partition the sweep had finished. Worth seeing, but a constant
        // 0 series on a dashboard trains people to ignore it.
        m.push(Metric {
            name: "EnrichmentFrontierMonthsReopened",
            value: summary.months_reopened as f64,
            unit: Unit::Count,
        });
    }
    if let Some(month) = summary.frontier_month {
        m.push(Metric {
            name: "EnrichmentFrontierMonth",
            value: month as f64,
            unit: Unit::None,
        });
    }
    m
}

pub fn sweep_metrics(summary: &CoarseSweepSummary) -> Vec<Metric> {
    vec![
        Metric {
            name: "CoarseSweepRowsEnriched",
            value: summary.total_enriched() as f64,
            unit: Unit::Count,
        },
        Metric {
            name: "CoarseSweepTableFailures",
            value: summary.failed_tables.len() as f64,
            unit: Unit::Count,
        },
        Metric {
            name: "CoarseSweepTablesSkipped",
            value: summary.skipped_tables.len() as f64,
            unit: Unit::Count,
        },
    ]
}

/// Publish `metrics` to CloudWatch under [`METRIC_NAMESPACE`], tagged with an
/// `Environment` dimension. One `PutMetricData` call for the whole batch.
#[cfg(feature = "lambda")]
pub async fn publish(
    client: &aws_sdk_cloudwatch::Client,
    environment: &str,
    metrics: &[Metric],
) -> Result<(), aws_sdk_cloudwatch::Error> {
    use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};

    let dimension = Dimension::builder()
        .name("Environment")
        .value(environment)
        .build();

    let data = metrics
        .iter()
        .map(|m| {
            MetricDatum::builder()
                .metric_name(m.name)
                .value(m.value)
                .unit(match m.unit {
                    Unit::Count => StandardUnit::Count,
                    Unit::Milliseconds => StandardUnit::Milliseconds,
                    Unit::None => StandardUnit::None,
                })
                .dimensions(dimension.clone())
                .build()
        })
        .collect::<Vec<_>>();

    client
        .put_metric_data()
        .namespace(METRIC_NAMESPACE)
        .set_metric_data(Some(data))
        .send()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_all_spec_metrics() {
        let stats = ChPassStats {
            batches: 3,
            candidates_before: 100,
            candidates_after: 7,
            rows_enriched: 93,
            rows_reset: 0,
            oracle_misses: 12,
            rows_remaining_at_volume_zero: 4,
            rows_remaining_recent: 1,
            duration_ms: 4500,
        };
        let m = pass_metrics(&stats);
        let by = |name: &str| m.iter().find(|x| x.name == name).expect("metric present");

        assert_eq!(by("EnrichmentRowsEnriched").value, 93.0);
        assert_eq!(by("EnrichmentOracleMiss").value, 12.0);
        // The volume-zero metric tracks `rows_remaining_at_volume_zero` (4), NOT
        // the general `candidates_after` remainder (7).
        assert_eq!(by("EnrichmentRowsRemainingAtVolumeZero").value, 4.0);
        // The recency-bounded subset (1) is a distinct series from the full
        // backlog (4) — the stall alarm gates on this one.
        assert_eq!(by("EnrichmentRowsRemainingRecent").value, 1.0);
        // Whole-pass duration (renamed from EnrichmentBatchDurationMs) …
        let dur = by("EnrichmentPassDurationMs");
        assert_eq!(dur.value, 4500.0);
        assert_eq!(dur.unit, Unit::Milliseconds);
        // … and the derived true per-batch figure: 4500ms / 3 batches.
        let avg = by("EnrichmentAvgBatchDurationMs");
        assert_eq!(avg.value, 1500.0);
        assert_eq!(avg.unit, Unit::Milliseconds);
        assert_eq!(m.len(), 6);
    }

    #[test]
    fn sweep_metrics_sum_enriched_and_split_failed_from_skipped() {
        use crate::repair::{CoarseSweepSummary, MonthRepair, RepairSummary, TableSweep};

        let table = |name: &str, enriched: u64, remaining: u64| TableSweep {
            table: name.to_string(),
            summary: RepairSummary {
                months: vec![MonthRepair {
                    month: 202_606,
                    zeros_before: enriched + remaining,
                    zeros_after: remaining,
                    rows_enriched: enriched,
                    rows_reset: 0,
                    snapshot_name: None,
                }],
            },
        };
        let summary = CoarseSweepSummary {
            start_month: 202_605,
            end_month: 202_606,
            tables: vec![table("price_ohlcv_1h", 8, 2), table("price_ohlcv_4h", 4, 1)],
            // A genuine runtime pass error vs a benign config skip — distinct series.
            failed_tables: vec!["price_ohlcv_1d".to_string()],
            skipped_tables: vec!["price_ohlcv_1m".to_string()],
        };

        let m = sweep_metrics(&summary);
        let by = |name: &str| m.iter().find(|x| x.name == name).expect("metric present");
        // Enriched is summed across every swept table (8+4).
        assert_eq!(by("CoarseSweepRowsEnriched").value, 12.0);
        // The alarm series counts ONLY the errored table, not the config skip …
        assert_eq!(by("CoarseSweepTableFailures").value, 1.0);
        // … which is surfaced on its own, non-alarming series instead.
        assert_eq!(by("CoarseSweepTablesSkipped").value, 1.0);
        // RowsRemaining is deliberately not published (floor-dominated).
        assert!(!m.iter().any(|x| x.name == "CoarseSweepRowsRemaining"));
        assert_eq!(m.len(), 3);
    }

    /// A pass that ran zero batches (empty backlog) has no per-batch figure, so
    /// `EnrichmentAvgBatchDurationMs` is omitted rather than dividing by zero.
    #[test]
    fn omits_avg_batch_duration_when_no_batches_ran() {
        let stats = ChPassStats {
            batches: 0,
            duration_ms: 120,
            ..Default::default()
        };
        let m = pass_metrics(&stats);
        assert!(
            !m.iter().any(|x| x.name == "EnrichmentAvgBatchDurationMs"),
            "avg-batch metric must be absent when batches == 0"
        );
        assert_eq!(m.len(), 5);
    }
}
