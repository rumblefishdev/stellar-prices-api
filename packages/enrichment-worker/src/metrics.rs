//! Enrichment telemetry — the four CloudWatch metrics from the 0024 design
//! spec §5, derived from a completed pass's [`ChPassStats`].
//!
//! The mapping ([`pass_metrics`]) is a pure function compiled in every build, so
//! it is unit-testable without the AWS SDK. The actual publish ([`publish`]) is
//! gated behind the `lambda` feature (it pulls `aws-sdk-cloudwatch`); the
//! default build / prototype CLI never links it. Publish is best-effort: the
//! Lambda logs a warning on failure rather than failing the invocation, so a
//! transient CloudWatch error never blocks enrichment.

use crate::ch_enrich::ChPassStats;

/// CloudWatch namespace for all enrichment metrics. Matches the
/// `cloudwatch:namespace` condition on the Lambda role's `PutMetricData` grant
/// and the alarm/dashboard wiring in `infra/`.
pub const METRIC_NAMESPACE: &str = "Prices/Enrichment";

/// CloudWatch unit for a [`Metric`]. Kept minimal — the enrichment metrics are
/// either counts or a duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Count,
    Milliseconds,
}

/// One CloudWatch datum: a spec §5 metric name, its value, and unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub name: &'static str,
    pub value: f64,
    pub unit: Unit,
}

/// Map a completed pass's stats to the four spec §5 metrics:
/// `EnrichmentRowsEnriched`, `EnrichmentOracleMiss`,
/// `EnrichmentRowsRemainingAtVolumeZero`, `EnrichmentBatchDurationMs`.
pub fn pass_metrics(stats: &ChPassStats) -> Vec<Metric> {
    vec![
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
        Metric {
            name: "EnrichmentRowsRemainingAtVolumeZero",
            value: stats.rows_remaining_at_volume_zero as f64,
            unit: Unit::Count,
        },
        Metric {
            name: "EnrichmentBatchDurationMs",
            value: stats.duration_ms as f64,
            unit: Unit::Milliseconds,
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
    fn maps_all_four_spec_metrics() {
        let stats = ChPassStats {
            batches: 3,
            candidates_before: 100,
            candidates_after: 7,
            rows_enriched: 93,
            oracle_misses: 12,
            rows_remaining_at_volume_zero: 4,
            duration_ms: 4500,
        };
        let m = pass_metrics(&stats);
        let by = |name: &str| m.iter().find(|x| x.name == name).expect("metric present");

        assert_eq!(by("EnrichmentRowsEnriched").value, 93.0);
        assert_eq!(by("EnrichmentOracleMiss").value, 12.0);
        // The volume-zero metric tracks `rows_remaining_at_volume_zero` (4), NOT
        // the general `candidates_after` remainder (7).
        assert_eq!(by("EnrichmentRowsRemainingAtVolumeZero").value, 4.0);
        let dur = by("EnrichmentBatchDurationMs");
        assert_eq!(dur.value, 4500.0);
        assert_eq!(dur.unit, Unit::Milliseconds);
        assert_eq!(m.len(), 4);
    }
}
