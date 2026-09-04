//! Ingest telemetry — `ClickHouseWriteLatencyMs`, the one ClickHouse-side
//! figure the Tranche 3 overview dashboard needs and the cluster cannot give us
//! (task 0125, ratified Decision B2).
//!
//! ClickHouse sits on Hetzner behind mTLS and streams no metrics of its own, and
//! the runtime identities hold no grant on `system.*` — so the write path is
//! measured where we already stand in it: at the candle-INSERT call sites in
//! [`crate::reconcile`]. One invocation makes 1+N INSERTs (SDEX plus one per AMM
//! source), and every one of those timings is published as a RAW value.
//!
//! Raw values, not a StatisticSet: CloudWatch cannot compute a percentile from
//! an aggregated statistic set (SampleCount/Sum/Minimum/Maximum discard the
//! distribution), and the dashboard queries `p95` in the row-0 acceptance strip
//! and `p50`/`p95` in the ingestion trend row. Published as a StatisticSet those
//! panels render "No data" forever, which on a dashboard whose audience is a
//! Stellar reviewer reads as an unmonitored system. Publishing the samples
//! themselves keeps the 1+N INSERTs inside ONE `PutMetricData` call — the
//! `Values` array on a single datum — so the batching rationale survives; it is
//! only the encoding that changes.
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
/// `write_candles` call, published as raw values so percentiles resolve.
pub const CH_WRITE_LATENCY: &str = "ClickHouseWriteLatencyMs";

/// `PutMetricData` accepts at most 150 entries in a datum's `Values` array, so
/// a run with more INSERTs than that spills into further datums of the same
/// metric rather than being truncated (or, worse, aggregated back into
/// something percentiles cannot read).
/// In production a run records `1 + <AMM sources>` samples — a handful — so
/// the spill into a second datum is a defence against the API limit, not a
/// path that is expected to run. Do not size anything against it.
pub const MAX_VALUES_PER_DATUM: usize = 150;

/// CloudWatch unit for a [`Metric`]. Only the one the ingest path emits — a
/// wider enum would be dead code here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Milliseconds,
}

/// Accumulated candle-write latency for one reconcile run.
///
/// Holds the individual measurements rather than an aggregate: that is what
/// makes `p50`/`p95` answerable on the dashboard, and it removes the old
/// `Default`-initialised-to-0.0 minimum trap by construction. A run that wrote
/// nothing must publish *no datapoint*, not a 0 ms sample — hence the carrier
/// is held as an `Option` on [`crate::reconcile::RunStats`] and an empty
/// `samples_ms` maps to no [`Metric`] at all.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WriteLatency {
    /// One entry per measured `write_candles` call, in milliseconds.
    pub samples_ms: Vec<f64>,
}

impl WriteLatency {
    /// Record one successful write of `ms` milliseconds.
    pub fn record(&mut self, ms: f64) {
        self.samples_ms.push(ms);
    }
}

/// One CloudWatch datum, SDK-free: a metric name, its unit and the raw values
/// measured for it (at most [`MAX_VALUES_PER_DATUM`] of them).
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub name: &'static str,
    pub unit: Unit,
    pub values: Vec<f64>,
}

/// Map a run's accumulated write latency to its CloudWatch metrics.
///
/// Returns an **empty** list when the run made no INSERT (or, defensively, when
/// the carrier holds no sample): CloudWatch rejects a `PutMetricData` with no
/// data, and a zero datapoint would poison the p50 of a metric whose whole
/// purpose is to show how long a real write takes.
///
/// Samples are chunked at [`MAX_VALUES_PER_DATUM`]; the chunks are datums of the
/// same metric inside the same publish call, so the "one `PutMetricData` per
/// invocation" property holds for any realistic run.
pub fn write_latency_metrics(latency: Option<WriteLatency>) -> Vec<Metric> {
    match latency {
        Some(l) if !l.samples_ms.is_empty() => l
            .samples_ms
            .chunks(MAX_VALUES_PER_DATUM)
            .map(|chunk| Metric {
                name: CH_WRITE_LATENCY,
                unit: Unit::Milliseconds,
                values: chunk.to_vec(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Publish `metrics` to CloudWatch under [`METRIC_NAMESPACE`], tagged with an
/// `Environment` dimension. One `PutMetricData` call for the whole batch.
///
/// Each datum carries its raw `Values` (no `Counts`, which defaults to 1 per
/// value) rather than a `StatisticValues` set — a statistic set cannot be read
/// back as a percentile, and the dashboard asks this metric for `p50`/`p95`.
///
/// A no-op on an empty slice: `PutMetricData` with no data is an API error, so
/// an idle run must not reach the wire at all.
#[cfg(feature = "lambda")]
pub async fn publish(
    client: &aws_sdk_cloudwatch::Client,
    environment: &str,
    metrics: &[Metric],
) -> Result<(), aws_sdk_cloudwatch::Error> {
    use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};

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
                .set_values(Some(m.values.clone()))
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
    fn a_single_insert_maps_to_one_datum_holding_one_value() {
        let mut l = WriteLatency::default();
        l.record(42.5);

        let m = write_latency_metrics(Some(l));
        assert_eq!(m.len(), 1, "exactly one datum, ever");

        let d = &m[0];
        assert_eq!(d.name, CH_WRITE_LATENCY);
        assert_eq!(d.unit, Unit::Milliseconds);
        assert_eq!(
            d.values,
            vec![42.5],
            "the raw measurement, not an aggregate — percentiles need the sample itself"
        );
    }

    #[test]
    fn several_inserts_fold_into_one_datum_of_raw_values() {
        let mut l = WriteLatency::default();
        l.record(30.0);
        l.record(10.0);
        l.record(20.0);

        let m = write_latency_metrics(Some(l));
        assert_eq!(
            m.len(),
            1,
            "1+N INSERTs are ONE datum carrying N values, not N datums"
        );

        let d = &m[0];
        assert_eq!(
            d.values,
            vec![30.0, 10.0, 20.0],
            "every sample survives to CloudWatch — an aggregate here would kill p50/p95"
        );
    }

    #[test]
    fn a_carrier_with_no_samples_maps_to_nothing() {
        // Defensive: PutMetricData with an empty datum list is an API error, so
        // the mapping must yield nothing for the publish to skip.
        assert!(write_latency_metrics(Some(WriteLatency::default())).is_empty());
    }

    #[test]
    fn the_smallest_value_is_the_smallest_measurement_never_zero() {
        // The `Default`-initialised-to-0.0 trap: every sample is well above
        // zero, so the smallest value published must be too.
        let mut l = WriteLatency::default();
        l.record(180.0);
        l.record(240.0);

        let m = write_latency_metrics(Some(l));
        let values = &m[0].values;
        let smallest = values.iter().cloned().fold(f64::INFINITY, f64::min);
        let largest = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(smallest, 180.0);
        assert_eq!(largest, 240.0);
        assert!(
            m.iter().all(|d| d.values.iter().all(|v| *v != 0.0)),
            "no datum may carry a zero — a 0 ms write never happened"
        );
    }

    #[test]
    fn more_than_one_hundred_and_fifty_samples_split_across_datums() {
        // `Values` accepts at most 150 entries per datum (PutMetricData API
        // limit), so a long run spills into a second datum of the SAME metric
        // rather than being truncated or aggregated away.
        let mut l = WriteLatency::default();
        for i in 1..=151 {
            l.record(i as f64);
        }

        let m = write_latency_metrics(Some(l));
        assert_eq!(
            m.len(),
            2,
            "151 samples are 150 + 1, not one oversized datum"
        );
        assert_eq!(m[0].values.len(), MAX_VALUES_PER_DATUM);
        assert_eq!(m[1].values.len(), 1);
        assert_eq!(m[1].values, vec![151.0]);
        assert!(
            m.iter().all(|d| d.name == CH_WRITE_LATENCY
                && d.unit == Unit::Milliseconds
                && !d.values.is_empty()),
            "every datum is the same metric, same unit, and carries values"
        );
    }
}
