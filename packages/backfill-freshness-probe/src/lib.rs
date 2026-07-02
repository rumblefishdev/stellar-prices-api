//! Backfill push-freshness probe (task 0056).
//!
//! The SDEX push-freshness alarm (§5.6 / Tranche-1 AC #5) must fire when a
//! scheduled `sdex-cloud-push` cycle is skipped and `sdex_archive.last_push_at`
//! ages past the operator-tunable threshold. `last_push_at` lives in Hetzner
//! ClickHouse (`prices.backfill_progress`), not in CloudWatch, so a CloudWatch
//! alarm cannot read it directly. This scheduled Lambda bridges the gap: it
//! queries the age of each stream's most recent push and republishes it as the
//! custom metric `PushAgeSeconds` under the `Prices/Backfill` namespace, tagged
//! with a `Stream` dimension. The alarm then fires on the metric.
//!
//! The age is computed **server-side** (`now() - last_push_at` in CH) so it is
//! immune to clock skew between the Lambda and ClickHouse — `last_push_at` was
//! itself written with CH's `now()` by the backfill sink.
//!
//! Split for testability: the pure metric-shaping ([`age_metrics`]) is compiled
//! in every build and unit-tested without the AWS SDK; the actual CloudWatch
//! publish ([`publish`]) is gated behind the `lambda` feature.

/// CloudWatch namespace for the backfill freshness metric. Must match the
/// `cloudwatch:namespace` condition on the Lambda role's `PutMetricData` grant
/// and the alarm wiring in `infra/`.
pub const METRIC_NAMESPACE: &str = "Prices/Backfill";

/// Custom metric name published per stream. One metric, disambiguated by the
/// `Stream` dimension, so additional `task_name`s (targeted gap-fills, future
/// AMM reindexes) publish without a code change.
pub const METRIC_NAME: &str = "PushAgeSeconds";

/// Canonical stream `task_name`s (mirror `prices.backfill_progress`). The SDEX
/// archive stream is the one the Tranche-1 freshness alarm watches; the Soroban
/// AMM stream's age is published for forensic value but has no wired alarm (it
/// completes in a single push then transitions to `completed` — ongoing
/// freshness is not meaningful; task Out-of-scope).
pub const SDEX_ARCHIVE_STREAM: &str = "sdex_archive";
pub const SOROBAN_AMM_STREAM: &str = "soroban_amm";

/// Sentinel published for a stream whose `last_push_at` is still NULL (no push
/// has landed yet). A negative value can never exceed the positive freshness
/// threshold, so the alarm reads "no push expected yet" as OK during the
/// pre-first-push window (§5.6) rather than false-firing on a fresh deploy.
pub const NO_PUSH_SENTINEL: f64 = -1.0;

/// One stream's push age as read from `backfill_progress`. `age_seconds` is
/// `None` when `last_push_at` is NULL (no push yet).
#[derive(Debug, Clone, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct StreamAge {
    pub task_name: String,
    pub age_seconds: Option<i64>,
}

/// One CloudWatch datum: the stream it belongs to and its push-age value
/// (seconds, or [`NO_PUSH_SENTINEL`] for a stream that has never pushed).
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub stream: String,
    pub value: f64,
}

/// Map the queried per-stream ages to CloudWatch data. A NULL age (no push yet)
/// becomes [`NO_PUSH_SENTINEL`]; a present age is published verbatim in seconds.
pub fn age_metrics(rows: &[StreamAge]) -> Vec<Metric> {
    rows.iter()
        .map(|row| Metric {
            stream: row.task_name.clone(),
            value: row
                .age_seconds
                .map(|s| s as f64)
                .unwrap_or(NO_PUSH_SENTINEL),
        })
        .collect()
}

/// SQL that reads each stream's push age (latest row per stream via `FINAL`).
/// Age is `now() - last_push_at` evaluated in ClickHouse; NULL `last_push_at`
/// yields NULL age (surfaced as [`NO_PUSH_SENTINEL`] by [`age_metrics`]).
pub const AGE_QUERY: &str = "SELECT \
     task_name, \
     if(isNull(last_push_at), NULL, \
        toInt64(toUnixTimestamp(now()) - toUnixTimestamp(last_push_at))) AS age_seconds \
   FROM backfill_progress FINAL \
   ORDER BY task_name";

/// Publish `metrics` to CloudWatch under [`METRIC_NAMESPACE`] as
/// [`METRIC_NAME`], each tagged with `Environment` + `Stream` dimensions. One
/// `PutMetricData` call for the whole batch. Best-effort at the call site: the
/// Lambda logs a warning on failure rather than failing the invocation.
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

    let env_dim = Dimension::builder()
        .name("Environment")
        .value(environment)
        .build();

    let data = metrics
        .iter()
        .map(|m| {
            MetricDatum::builder()
                .metric_name(METRIC_NAME)
                .value(m.value)
                .unit(StandardUnit::Seconds)
                .dimensions(env_dim.clone())
                .dimensions(Dimension::builder().name("Stream").value(&m.stream).build())
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
    fn present_age_maps_verbatim() {
        let rows = vec![StreamAge {
            task_name: SDEX_ARCHIVE_STREAM.to_string(),
            age_seconds: Some(604_801),
        }];
        let m = age_metrics(&rows);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].stream, SDEX_ARCHIVE_STREAM);
        assert_eq!(m[0].value, 604_801.0);
    }

    #[test]
    fn null_age_becomes_no_push_sentinel() {
        // A stream that has never pushed must publish a value that can never
        // exceed a positive freshness threshold, so the alarm stays OK during
        // the pre-first-push window.
        let rows = vec![StreamAge {
            task_name: SDEX_ARCHIVE_STREAM.to_string(),
            age_seconds: None,
        }];
        let m = age_metrics(&rows);
        assert_eq!(m[0].value, NO_PUSH_SENTINEL);
        assert!(m[0].value < 7.0 * 86_400.0);
    }

    #[test]
    fn both_streams_published() {
        let rows = vec![
            StreamAge {
                task_name: SDEX_ARCHIVE_STREAM.to_string(),
                age_seconds: Some(10),
            },
            StreamAge {
                task_name: SOROBAN_AMM_STREAM.to_string(),
                age_seconds: Some(20),
            },
        ];
        let m = age_metrics(&rows);
        let by = |s: &str| m.iter().find(|x| x.stream == s).expect("stream present");
        assert_eq!(by(SDEX_ARCHIVE_STREAM).value, 10.0);
        assert_eq!(by(SOROBAN_AMM_STREAM).value, 20.0);
        assert_eq!(m.len(), 2);
    }
}
