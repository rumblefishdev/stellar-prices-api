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
//! ## Only an *active, already-pushing* backfill is watched (task 0056 finding A)
//!
//! [`AGE_QUERY`] publishes an age **only** for a stream that is
//! `status = 'running'` **and** has pushed at least once (`last_push_at IS NOT
//! NULL`). A live-only deployment with no historical backfill running (the
//! post-go-live reality: `backfill_progress` seeded, `backfill_sdex_ledgers`
//! empty) therefore publishes nothing, the metric goes missing, and the
//! `NOT_BREACHING` alarm stays OK instead of false-firing on the seed rows'
//! age. This **supersedes** the earlier `coalesce(last_push_at, started_at)`
//! fallback: the freshness alarm measures a *push cadence that has stalled*
//! (Tranche-1 AC #5 — "skip a scheduled push cycle"), which only has meaning
//! once pushing has begun. A backfill that is *expected but silent* is now an
//! operator concern of the manual chunked run, not a metric that could not tell
//! "backfill overdue" apart from "no backfill at all" — that ambiguity was the
//! go-live false-page (finding A).
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

/// One stream's push age as read from `backfill_progress`, in seconds. Only
/// streams that are `status = 'running'` and have pushed at least once reach
/// this struct (see [`AGE_QUERY`]); a paused/completed stream, or one that has
/// never pushed, is filtered out in SQL and simply produces no row. `age_seconds`
/// is `now() - last_push_at` and is therefore never NULL.
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

/// SQL that reads each *active, already-pushing* stream's push age (latest row
/// per stream via `FINAL`). Age is `now() - last_push_at` evaluated in
/// ClickHouse: seconds since the last push.
///
/// The `WHERE status = 'running' AND last_push_at IS NOT NULL` guard is the
/// task 0056 finding-A fix. It excludes:
/// - **live-only seed rows** — `backfill_progress` is seeded `status='running'`
///   with a NULL `last_push_at`; with no backfill actually pushing, the old
///   `coalesce(…, started_at)` fallback aged them from seed time and sat the
///   alarm permanently in ALARM (the go-live false-page). A NULL `last_push_at`
///   now yields no row → no metric → `NOT_BREACHING` → OK.
/// - **paused streams** — the documented `status='paused'` seam *between* the
///   two backfill runs (§5.6) no longer needs to lean on `NOT_BREACHING`.
/// - **completed streams** — a finished backfill legitimately stops pushing and
///   must not page.
///
/// A running backfill that *has* pushed and then stalls keeps `status='running'`
/// with a stale `last_push_at`, so its age climbs and the alarm fires — exactly
/// Tranche-1 AC #5. Because the filter guarantees a non-NULL `last_push_at`,
/// `age_seconds` is never NULL.
pub const AGE_QUERY: &str = "SELECT \
     task_name, \
     toInt64(toUnixTimestamp(now()) - toUnixTimestamp(last_push_at)) \
       AS age_seconds \
   FROM backfill_progress FINAL \
   WHERE status = 'running' AND last_push_at IS NOT NULL \
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
    fn running_pushed_stream_that_stalls_can_breach() {
        // A running backfill that has pushed and then stalls keeps aging its
        // `last_push_at`; AGE_QUERY returns that climbing age (only running +
        // already-pushed streams survive the WHERE guard), so an 8-day-old push
        // is over the 7-day default and would breach (Tranche-1 AC #5).
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
    fn age_query_gates_on_running_and_pushed() {
        // Finding A: only an active, already-pushing backfill is watched, so a
        // live-only deployment's seed rows (running, NULL last_push_at) and the
        // paused/completed seams never publish an age → no false-fire.
        assert!(AGE_QUERY.contains("status = 'running'"));
        assert!(AGE_QUERY.contains("last_push_at IS NOT NULL"));
        // The superseded started_at fallback must be gone (it was the false-fire
        // source in live-only).
        assert!(!AGE_QUERY.contains("coalesce"));
        assert!(!AGE_QUERY.contains("started_at"));
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
