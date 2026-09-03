//! Ingest telemetry — `ClickHouseWriteLatencyMs`, the one ClickHouse-side
//! figure the Tranche 3 overview dashboard needs and the cluster cannot give us
//! (task 0125, ratified Decision B2).
//!
//! ClickHouse sits on Hetzner behind mTLS and streams no metrics of its own, and
//! the runtime identities hold no grant on `system.*` — so the write path is
//! measured where we already stand in it: at the candle-INSERT call sites in
//! [`crate::reconcile`]. One invocation makes 1+N INSERTs (SDEX plus one per AMM
//! source), so the datapoints fold into a single StatisticSet rather than 1+N
//! separate `PutMetricData` values.
//!
//! Same split as `enrichment-worker::metrics`: the mapping
//! ([`write_latency_metrics`]) is a pure function compiled in every build, so it
//! is unit-testable without the AWS SDK, while the actual [`publish`] is gated
//! behind the `lambda` feature (it pulls `aws-sdk-cloudwatch`) and the default
//! build / prototype CLI never links it. Publish is best-effort: the Lambda logs
//! a warning rather than failing the invocation, and it runs only after the
//! cursor commit, so a CloudWatch outage can never redeliver a doorbell.

/// CloudWatch namespace for the ingest metrics. Matches the
/// `cloudwatch:namespace` condition on the ledger-processor role's
/// `PutMetricData` grant (`infra/src/lib/stacks/compute-stack.ts`) — a mismatch
/// makes every publish fail with AccessDenied and nothing fails loudly.
pub const METRIC_NAMESPACE: &str = "Prices/Ingest";

/// The single metric this crate publishes. Milliseconds spent inside one
/// `write_candles` call, as a StatisticSet over the invocation's INSERTs.
pub const CH_WRITE_LATENCY: &str = "ClickHouseWriteLatencyMs";

/// CloudWatch unit for a [`Metric`]. Only the one the ingest path emits — a
/// wider enum would be dead code here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Milliseconds,
}

/// Accumulated candle-write latency for one reconcile run.
///
/// Deliberately **not** a bare set of `f64`s reached through `Default`: a run
/// that wrote nothing must publish *no datapoint*, not a 0 ms minimum, so the
/// carrier is held as an `Option` on [`crate::reconcile::RunStats`] and
/// [`record`](WriteLatency::record) widens `min_ms`/`max_ms` out of the empty
/// state rather than against a zero seed.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WriteLatency {
    /// Number of `write_candles` calls measured.
    pub count: u64,
    pub sum_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
}

impl WriteLatency {
    /// Fold one successful write of `ms` milliseconds into the accumulator.
    ///
    /// The first sample seeds both bounds; a `min_ms.min(ms)` against the
    /// `Default`-initialised `0.0` would pin the minimum at zero forever.
    pub fn record(&mut self, ms: f64) {
        if self.count == 0 {
            self.min_ms = ms;
            self.max_ms = ms;
        } else {
            if ms < self.min_ms {
                self.min_ms = ms;
            }
            if ms > self.max_ms {
                self.max_ms = ms;
            }
        }
        self.count += 1;
        self.sum_ms += ms;
    }
}

/// One CloudWatch datum, SDK-free: a metric name, its unit and its statistic
/// set (count / sum / min / max).
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub name: &'static str,
    pub unit: Unit,
    pub sample_count: f64,
    pub sum: f64,
    pub minimum: f64,
    pub maximum: f64,
}

/// Map a run's accumulated write latency to its CloudWatch metrics.
///
/// Returns an **empty** list when the run made no INSERT (or, defensively, when
/// the carrier holds no sample): CloudWatch rejects a `PutMetricData` with no
/// data, and a zero datapoint would poison the p50 of a metric whose whole
/// purpose is to show how long a real write takes.
pub fn write_latency_metrics(latency: Option<WriteLatency>) -> Vec<Metric> {
    match latency {
        Some(l) if l.count > 0 => vec![Metric {
            name: CH_WRITE_LATENCY,
            unit: Unit::Milliseconds,
            sample_count: l.count as f64,
            sum: l.sum_ms,
            minimum: l.min_ms,
            maximum: l.max_ms,
        }],
        _ => Vec::new(),
    }
}

/// Publish `metrics` to CloudWatch under [`METRIC_NAMESPACE`], tagged with an
/// `Environment` dimension. One `PutMetricData` call for the whole batch.
///
/// A no-op on an empty slice: `PutMetricData` with no data is an API error, so
/// an idle run must not reach the wire at all.
#[cfg(feature = "lambda")]
pub async fn publish(
    client: &aws_sdk_cloudwatch::Client,
    environment: &str,
    metrics: &[Metric],
) -> Result<(), aws_sdk_cloudwatch::Error> {
    use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit, StatisticSet};

    if metrics.is_empty() {
        return Ok(());
    }

    let dimension = Dimension::builder()
        .name("Environment")
        .value(environment)
        .build();

    let data = metrics
        .iter()
        .map(|m| {
            MetricDatum::builder()
                .metric_name(m.name)
                .statistic_values(
                    StatisticSet::builder()
                        .sample_count(m.sample_count)
                        .sum(m.sum)
                        .minimum(m.minimum)
                        .maximum(m.maximum)
                        .build(),
                )
                .unit(match m.unit {
                    Unit::Milliseconds => StandardUnit::Milliseconds,
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
    fn a_run_with_no_insert_publishes_no_datapoint() {
        assert!(
            write_latency_metrics(None).is_empty(),
            "an idle run must publish nothing at all — a zero would be a lie about write latency"
        );
    }

    #[test]
    fn a_single_insert_maps_to_one_metric() {
        let mut l = WriteLatency::default();
        l.record(42.5);

        let m = write_latency_metrics(Some(l));
        assert_eq!(m.len(), 1, "exactly one metric, ever");

        let d = &m[0];
        assert_eq!(d.name, CH_WRITE_LATENCY);
        assert_eq!(d.unit, Unit::Milliseconds);
        assert_eq!(d.sample_count, 1.0);
        assert_eq!(d.sum, 42.5);
        assert_eq!(d.minimum, 42.5);
        assert_eq!(d.maximum, 42.5);
    }

    #[test]
    fn several_inserts_fold_into_one_statistic_set() {
        let mut l = WriteLatency::default();
        l.record(30.0);
        l.record(10.0);
        l.record(20.0);

        let m = write_latency_metrics(Some(l));
        assert_eq!(
            m.len(),
            1,
            "1+N INSERTs are one StatisticSet, not N datapoints"
        );

        let d = &m[0];
        assert_eq!(d.sample_count, 3.0);
        assert_eq!(d.sum, 60.0);
        assert_eq!(d.minimum, 10.0);
        assert_eq!(d.maximum, 30.0);
    }

    #[test]
    fn a_carrier_with_no_samples_maps_to_nothing() {
        // Defensive: PutMetricData with an empty datum list is an API error, so
        // the mapping must yield nothing for the publish to skip.
        assert!(write_latency_metrics(Some(WriteLatency::default())).is_empty());
    }

    #[test]
    fn the_minimum_is_the_smallest_measurement_not_zero() {
        // The `Default`-initialised-to-0.0 trap: every sample is well above
        // zero, so the reported minimum must be too.
        let mut l = WriteLatency::default();
        l.record(180.0);
        l.record(240.0);

        let m = write_latency_metrics(Some(l));
        assert_eq!(m[0].minimum, 180.0);
        assert_eq!(m[0].maximum, 240.0);
    }
}
