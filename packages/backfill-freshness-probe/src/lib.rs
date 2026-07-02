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

/// One stream's push age as read from `backfill_progress`, in seconds. For a
/// stream that has never pushed (`last_push_at` IS NULL) the age is measured
/// from `started_at` (registration time) instead of a NULL — see [`AGE_QUERY`]
/// — so a never-pushed stream still ages out. `age_seconds` is therefore never
/// NULL.
#[derive(Debug, Clone, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct StreamAge {
    pub task_name: String,
    pub age_seconds: i64,
}

/// One CloudWatch datum: the stream it belongs to and its push-age value in
/// seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub stream: String,
    pub value: f64,
}

/// Map the queried per-stream ages to CloudWatch data — the age in seconds,
/// verbatim.
pub fn age_metrics(rows: &[StreamAge]) -> Vec<Metric> {
    rows.iter()
        .map(|row| Metric {
            stream: row.task_name.clone(),
            value: row.age_seconds as f64,
        })
        .collect()
}

/// SQL that reads each stream's push age (latest row per stream via `FINAL`).
/// Age is `now() - coalesce(last_push_at, started_at)` evaluated in ClickHouse:
/// seconds since the last push, or — for a stream that has never pushed — since
/// its registration (`started_at`, which the backfill sink preserves across row
/// updates). The `started_at` fallback is the fix for the never-first-pushed
/// blind spot: a NULL `last_push_at` used to publish a sentinel that sat
/// permanently below the threshold, so a first push that was overdue (or never
/// came) never fired the alarm. Now the stream keeps aging from registration,
/// so once it exceeds the freshness threshold the alarm fires — "alarm once the
/// Tranche-1 window opens" (§5.6). `started_at` is non-nullable, so `coalesce`
/// always yields a value and `age_seconds` is never NULL.
pub const AGE_QUERY: &str = "SELECT \
     task_name, \
     toInt64(toUnixTimestamp(now()) - toUnixTimestamp(coalesce(last_push_at, started_at))) \
       AS age_seconds \
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
            age_seconds: 604_801,
        }];
        let m = age_metrics(&rows);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].stream, SDEX_ARCHIVE_STREAM);
        assert_eq!(m[0].value, 604_801.0);
    }

    #[test]
    fn never_pushed_stream_ages_out_and_can_breach() {
        // A never-pushed stream no longer publishes a permanently-OK sentinel:
        // AGE_QUERY ages it from `started_at`, so once it has existed past the
        // freshness threshold the published age exceeds it and the alarm fires
        // ("alarm once the Tranche-1 window opens", §5.6). Here an 8-day-old
        // never-pushed stream is over the 7-day default and would breach.
        let eight_days = 8 * 86_400;
        let rows = vec![StreamAge {
            task_name: SDEX_ARCHIVE_STREAM.to_string(),
            age_seconds: eight_days,
        }];
        let m = age_metrics(&rows);
        assert_eq!(m[0].value, eight_days as f64);
        assert!(m[0].value > 7.0 * 86_400.0);
    }

    #[test]
    fn both_streams_published() {
        let rows = vec![
            StreamAge {
                task_name: SDEX_ARCHIVE_STREAM.to_string(),
                age_seconds: 10,
            },
            StreamAge {
                task_name: SOROBAN_AMM_STREAM.to_string(),
                age_seconds: 20,
            },
        ];
        let m = age_metrics(&rows);
        let by = |s: &str| m.iter().find(|x| x.stream == s).expect("stream present");
        assert_eq!(by(SDEX_ARCHIVE_STREAM).value, 10.0);
        assert_eq!(by(SOROBAN_AMM_STREAM).value, 20.0);
        assert_eq!(m.len(), 2);
    }
}
