//! Oracle telemetry — the CloudWatch metrics behind task 0231's dark-feed
//! alarms, derived from a completed pass's [`OracleStats`].
//!
//! ## Why this exists
//!
//! [`crate::reflector_timestamp_to_epoch_seconds`] (task 0227) made the worker
//! **refuse** an implausible Reflector reading instead of writing it. That is
//! correct and it is only half the requirement: the rejection increments a
//! counter and logs at `error`, but [`crate::run_oracle`] still returns `Ok`,
//! the Lambda reports success, and nothing watches. If Reflector changes
//! `lastprice`'s unit again, every reading is refused — correctly — and the
//! poll feed goes 100% dark with a green Lambda in front of it. That trades
//! five months of silently *wrong* rows for an indefinite silently *absent*
//! feed: the same defect wearing different clothes.
//!
//! ## Shape
//!
//! Deliberately copied from `enrichment-worker/src/metrics.rs` rather than
//! reinvented: the mapping ([`pass_metrics`] / [`failure_metrics`]) is a pure
//! function compiled into every build, so it is unit-testable without the AWS
//! SDK, and the publish ([`publish`]) is gated behind the `lambda` feature (it
//! pulls `aws-sdk-cloudwatch`) and is best-effort — a CloudWatch failure logs a
//! warning rather than failing the poll.
//!
//! ## The three states an operator must be able to tell apart
//!
//! This is task 0218's lesson applied to a second worker: `Ok`-only publishing
//! makes a *failed* run indistinguishable from a run that *never happened* —
//! both emit nothing.
//!
//! | state | signal |
//! |---|---|
//! | never invoked | **no datapoint at all** — the `-no-invocations` alarm's job |
//! | ran, wrote nothing | `OracleRuns=1`, `OracleFailedRuns=0`, `OracleRowsWritten=0` |
//! | ran, failed | `OracleRuns=1`, `OracleFailedRuns=1` |
//!
//! No pass-duration metric is published. The oracle Lambda does exactly one
//! thing per invocation, so `AWS/Lambda` `Duration` already measures the pass
//! with nothing else folded into it — unlike the enrichment worker, where three
//! stages share an invocation and the split is the whole point.

use crate::OracleStats;

/// CloudWatch namespace for all oracle metrics. Must match the
/// `cloudwatch:namespace` condition on the Lambda role's `PutMetricData` grant
/// and the alarm wiring in `infra/`, or the publish fails closed at runtime
/// while the deploy stays green.
pub const METRIC_NAMESPACE: &str = "Prices/Oracle";

/// CloudWatch unit for a [`Metric`]. Every oracle metric is a count — see the
/// module docs on why no duration is published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Count,
}

/// One CloudWatch datum: a metric name, its value, and unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub name: &'static str,
    pub value: f64,
    pub unit: Unit,
}

/// Map a completed pass's [`OracleStats`] to its CloudWatch metrics.
///
///   * `OracleRuns` — always `1`. Published on **every** completed pass,
///     including one that wrote nothing, so "ran and found nothing" is a
///     datapoint rather than silence and only "never invoked" is missing data.
///   * `OracleRowsWritten` — rows written to `oracle_prices` this pass. **The
///     dark-feed series.** Sustained zero is the symptom that catches every
///     cause, including ones not yet imagined; alarm on this one first.
///   * `OracleTimestampRejected` — readings refused by the 0227 plausibility
///     guard. Kept as its own series precisely because it must not be read off
///     `OracleRowsSkipped`: a Reflector unit change would arrive there as a
///     handful of extra skips among ordinary fetch failures and look like
///     nothing at all. Here, any non-zero value names the cause directly.
///   * `OracleRowsSkipped` — every reading that produced no row, for **any**
///     reason: no price, a failed fetch, or a rejected timestamp. A superset of
///     `OracleTimestampRejected`, not a disjoint bucket — it keeps the meaning
///     the existing log line and Lambda response already give it, and narrowing
///     it here would silently change a shipped contract to save a metric that
///     is published separately anyway.
///   * `OracleSymbolsQueried` — the denominator. Without it, `RowsWritten = 3`
///     is unreadable: it could be a healthy pass over 3 tracked symbols or a
///     collapsed one over 30.
///   * `OracleUsdRatesSnapshotted` — rows copied into `prices.usd_rate` (task
///     0167). Zero is normal on a steady-state pass, which is exactly why it
///     needs a series rather than an alarm: [`OracleStats`] already records
///     that zero *forever* while `written` climbs is the failure, and that is a
///     shape only a plotted history shows.
///   * `OracleFailedRuns` — `0` here, `1` in [`failure_metrics`]. See the
///     module docs' three-state table.
pub fn pass_metrics(stats: &OracleStats) -> Vec<Metric> {
    let count = |name: &'static str, value: f64| Metric {
        name,
        value,
        unit: Unit::Count,
    };
    vec![
        count("OracleRuns", 1.0),
        count("OracleFailedRuns", 0.0),
        count("OracleSymbolsQueried", stats.queried as f64),
        count("OracleRowsWritten", stats.written as f64),
        count("OracleRowsSkipped", stats.skipped as f64),
        count("OracleTimestampRejected", stats.timestamp_rejected as f64),
        count("OracleUsdRatesSnapshotted", stats.rates_snapshotted as f64),
    ]
}

/// Metrics for a pass that failed before producing an [`OracleStats`] — i.e.
/// [`crate::run_oracle`] returned [`crate::OracleFailure`], so the per-symbol
/// totals were never assembled.
///
/// `OracleRowsWritten` is published as an explicit `0` rather than omitted: a
/// failed pass genuinely wrote nothing, and the dark-feed alarm must see it as
/// a breaching datapoint instead of as missing data it might treat as
/// `notBreaching`.
///
/// `OracleTimestampRejected` carries `rejected` — the count the failed pass had
/// **already measured**, since rejections happen in the per-symbol loop, ahead
/// of the ClickHouse writes that are the likely reason a pass dies. Publishing
/// it here is the same principle as publishing at all on the error path: the
/// signal was measured, so it must not be lost because something later broke.
pub fn failure_metrics(rejected: usize) -> Vec<Metric> {
    let count = |name: &'static str, value: f64| Metric {
        name,
        value,
        unit: Unit::Count,
    };
    vec![
        count("OracleRuns", 1.0),
        count("OracleFailedRuns", 1.0),
        count("OracleRowsWritten", 0.0),
        count("OracleTimestampRejected", rejected as f64),
    ]
}

/// Publish `metrics` to CloudWatch under [`METRIC_NAMESPACE`], tagged with an
/// `Environment` dimension. One `PutMetricData` call for the whole batch.
///
/// ⚠️ The `Environment` dimension is part of the metric's identity: an alarm
/// that omits it, or spells it differently, watches a series that does not
/// exist and returns 0 datapoints rather than an error (task 0204 shipped 10 of
/// 13 alarms blind this way). Whatever `infra/` declares must match the
/// `ENV_NAME` the Lambda actually runs with.
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

    fn by<'a>(metrics: &'a [Metric], name: &str) -> &'a Metric {
        metrics
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("metric {name} present"))
    }

    #[test]
    fn maps_every_pass_stat() {
        let stats = OracleStats {
            queried: 30,
            written: 27,
            skipped: 3,
            timestamp_rejected: 1,
            rates_snapshotted: 12,
        };
        let m = pass_metrics(&stats);

        assert_eq!(by(&m, "OracleSymbolsQueried").value, 30.0);
        assert_eq!(by(&m, "OracleRowsWritten").value, 27.0);
        assert_eq!(by(&m, "OracleUsdRatesSnapshotted").value, 12.0);
        // The rejection is its own series AND still counted in the general
        // skip total — the two are a subset relation, not disjoint buckets.
        assert_eq!(by(&m, "OracleRowsSkipped").value, 3.0);
        assert_eq!(by(&m, "OracleTimestampRejected").value, 1.0);
        // A completed pass always reports itself, so an idle pass is a
        // datapoint rather than silence.
        assert_eq!(by(&m, "OracleRuns").value, 1.0);
        assert_eq!(by(&m, "OracleFailedRuns").value, 0.0);
        assert_eq!(m.len(), 7);
    }

    /// The dark-feed case: the guard refuses every reading, so nothing is
    /// written and the Lambda still succeeds. `OracleRowsWritten` must be an
    /// explicit `0` datapoint — if this were omitted, the alarm would see
    /// missing data instead of a breach, which is the whole failure 0231
    /// exists to close.
    #[test]
    fn a_wholly_dark_pass_publishes_an_explicit_zero_written() {
        let stats = OracleStats {
            queried: 30,
            written: 0,
            skipped: 30,
            timestamp_rejected: 30,
            rates_snapshotted: 0,
        };
        let m = pass_metrics(&stats);
        let written = by(&m, "OracleRowsWritten");
        assert_eq!(written.value, 0.0);
        assert_eq!(written.unit, Unit::Count);
        // …and the cause is named on its own series, not buried in the skips.
        assert_eq!(by(&m, "OracleTimestampRejected").value, 30.0);
        // The pass itself succeeded — this is precisely the state that reads as
        // healthy everywhere else.
        assert_eq!(by(&m, "OracleFailedRuns").value, 0.0);
    }

    /// A single rejection among otherwise healthy readings must still be
    /// visible. Read off `OracleRowsSkipped` alone this is 3 vs 2 — noise;
    /// on its own series it is a step off zero.
    #[test]
    fn one_rejection_is_visible_apart_from_ordinary_skips() {
        let healthy = OracleStats {
            queried: 30,
            written: 28,
            skipped: 2,
            timestamp_rejected: 0,
            rates_snapshotted: 4,
        };
        let one_bad = OracleStats {
            written: 27,
            skipped: 3,
            timestamp_rejected: 1,
            ..healthy
        };
        assert_eq!(
            by(&pass_metrics(&healthy), "OracleTimestampRejected").value,
            0.0
        );
        assert_eq!(
            by(&pass_metrics(&one_bad), "OracleTimestampRejected").value,
            1.0
        );
    }

    /// A failed run is a datapoint, not silence — otherwise it is
    /// indistinguishable from an invocation that never happened.
    #[test]
    fn a_failed_run_is_a_datapoint_not_silence() {
        let m = failure_metrics(0);
        assert_eq!(by(&m, "OracleRuns").value, 1.0);
        assert_eq!(by(&m, "OracleFailedRuns").value, 1.0);
        // It wrote nothing, and the dark-feed alarm must see that as a breach.
        assert_eq!(by(&m, "OracleRowsWritten").value, 0.0);
        assert_eq!(m.len(), 4);
    }

    /// A pass that refused a reading and THEN died must still report the
    /// rejection. The rejections happen in the per-symbol loop, ahead of the
    /// ClickHouse writes that are the likely failure — so this is a real
    /// ordering, not a hypothetical, and dropping the count would lose the one
    /// signal task 0231 exists to produce on exactly the invocation that had
    /// something to say.
    #[test]
    fn a_failed_run_still_reports_rejections_it_had_already_measured() {
        let m = failure_metrics(2);
        assert_eq!(by(&m, "OracleTimestampRejected").value, 2.0);
        assert_eq!(by(&m, "OracleFailedRuns").value, 1.0);
    }

    /// Both mappings must agree on the two series that carry the three-state
    /// distinction, or an alarm gating on them reads one shape from success and
    /// another from failure.
    #[test]
    fn both_mappings_publish_runs_and_failed_runs() {
        for m in [pass_metrics(&OracleStats::default()), failure_metrics(0)] {
            assert_eq!(by(&m, "OracleRuns").value, 1.0);
            assert!(m.iter().any(|x| x.name == "OracleFailedRuns"));
            assert!(m.iter().any(|x| x.name == "OracleRowsWritten"));
        }
    }
}
