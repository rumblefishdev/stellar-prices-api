//! Rollup freshness probe (task 0137).
//!
//! Task 0136 froze `price_ohlcv_15m` through `_1M` for **nine days** and nothing
//! alarmed. Eight of the nine refreshable MVs reported `status = Scheduled` with
//! an empty `exception` every single cycle — only `mv_ohlcv_1m_to_15m` carried
//! the error, and everything downstream of it rolled up stale input and called
//! that success. **Rolling up nothing is not a failure, so no failure was
//! reported.** The freeze was found by accident, when task 0072's rollout
//! verification noticed `change_7d_pct` was 0 for every asset.
//!
//! The health signal therefore has to be **data freshness** — how old the newest
//! bucket in each table is — not MV exit status. This probe reads that age per
//! tier and republishes it as the `Prices/Rollup` `RollupLagSeconds` CloudWatch
//! metric, tagged with a `Table` dimension, which the per-tier alarms watch.
//!
//! The age is computed **server-side** (`now() - max(timestamp)` in ClickHouse)
//! so it is immune to clock skew between the Lambda and ClickHouse.
//!
//! ## Why a dedicated probe and not the enrichment worker
//!
//! Task 0137 as filed preferred folding this into an existing scheduled worker.
//! That was reversed before implementation. Task 0112 found that three scheduled
//! workers each had exactly one alarm, and each of those read a custom metric the
//! worker publishes **only if it survives to the end of a pass** — so none of
//! them could detect its own worker dying. Publishing rollup lag from
//! `enrichment-worker` would rebuild that blind spot one layer up: an enrichment
//! stall (the component behind task 0111's four-day outage) would publish no
//! datum, the `NOT_BREACHING` alarms would sit silently OK, and a frozen rollup
//! would again go unreported. A dedicated probe mirrors the tested
//! `backfill-freshness-probe` (task 0056) and earns dead-probe cover from
//! `addWorkerHealthAlarms`.
//!
//! Split for testability: the pure metric-shaping ([`lag_metrics`]) and the query
//! construction ([`freshness_query`]) are compiled in every build and unit-tested
//! without the AWS SDK; the actual CloudWatch publish ([`publish`]) is gated
//! behind the `lambda` feature.

/// CloudWatch namespace for the rollup freshness metric. Must match the
/// `cloudwatch:namespace` condition on the Lambda role's `PutMetricData` grant
/// and the alarm wiring in `infra/`.
pub const METRIC_NAMESPACE: &str = "Prices/Rollup";

/// Custom metric name published per tier. One metric, disambiguated by the
/// `Table` dimension, so a new granularity publishes without a code change on
/// the CloudWatch side.
pub const METRIC_NAME: &str = "RollupLagSeconds";

/// One OHLCV granularity and the age bound beyond which its data is considered
/// stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollupTier {
    /// Unqualified table name. The probe binds its client to the `prices`
    /// database, so the query references these unqualified (see [`freshness_query`]).
    pub table: &'static str,
    /// Width of one bucket in this tier, in seconds. Not a threshold — it is the
    /// floor the threshold must clear, and the reason the coarse bounds look so
    /// generous. See the module note on [`ROLLUP_TIERS`].
    pub bucket_seconds: i64,
    /// Alarm threshold: publish-to-breach age in seconds.
    pub lag_bound_seconds: i64,
}

/// The seven granularities, with their lag bounds and the rationale AC #5
/// requires be recorded.
///
/// ## Why every bound exceeds its own bucket width
///
/// `timestamp` is the bucket **start**, so a tier's lag does not sit near zero —
/// it sawtooths from `0` up to one full bucket width as the newest bucket ages,
/// then drops when the next one opens. A `1w` tier whose rollup is perfectly
/// healthy still reports a six-day lag the day before its bucket rolls over.
/// Any bound at or below the bucket width therefore false-fires once per bucket,
/// every bucket, forever.
///
/// Each bound is `bucket_seconds` plus headroom for rollup latency:
///
/// | tier | bucket | bound | headroom |
/// |---|---|---|---|
/// | `price_ohlcv_1m` | 1 min | 15 min | 14 min |
/// | `price_ohlcv_15m` | 15 min | 1 h | 45 min |
/// | `price_ohlcv_1h` | 1 h | 3 h | 2 h |
/// | `price_ohlcv_4h` | 4 h | 12 h | 8 h |
/// | `price_ohlcv_1d` | 1 d | 48 h | 24 h |
/// | `price_ohlcv_1w` | 7 d | 10 d | 3 d |
/// | `price_ohlcv_1M` | ~31 d | 45 d | 14 d |
///
/// ⚠️ **`1M` buckets are weeks-attributed-by-start, not calendar months**, so a
/// `1M` bucket can span up to ~31 days. The 45-day bound is sized off that, not
/// off a 30-day month.
///
/// The bounds are deliberately loose. This alarm exists to catch *a number that
/// stopped moving* — 0136 ran for nine days — not to measure rollup latency. A
/// tight bound that pages on ordinary cadence jitter would be turned off, and an
/// alarm that is off is the state 0137 was filed to fix. Task 0104 owns the
/// cadence-vs-window question; these bounds are set not to contradict it.
pub const ROLLUP_TIERS: &[RollupTier] = &[
    RollupTier {
        table: "price_ohlcv_1m",
        bucket_seconds: 60,
        lag_bound_seconds: 15 * 60,
    },
    RollupTier {
        table: "price_ohlcv_15m",
        bucket_seconds: 15 * 60,
        lag_bound_seconds: 60 * 60,
    },
    RollupTier {
        table: "price_ohlcv_1h",
        bucket_seconds: 60 * 60,
        lag_bound_seconds: 3 * 60 * 60,
    },
    RollupTier {
        table: "price_ohlcv_4h",
        bucket_seconds: 4 * 60 * 60,
        lag_bound_seconds: 12 * 60 * 60,
    },
    RollupTier {
        table: "price_ohlcv_1d",
        bucket_seconds: 86_400,
        lag_bound_seconds: 48 * 60 * 60,
    },
    RollupTier {
        table: "price_ohlcv_1w",
        bucket_seconds: 7 * 86_400,
        lag_bound_seconds: 10 * 86_400,
    },
    RollupTier {
        table: "price_ohlcv_1M",
        bucket_seconds: 31 * 86_400,
        lag_bound_seconds: 45 * 86_400,
    },
];

/// One tier's rollup lag as read from its table, in seconds. A tier with **no
/// rows at all** never reaches this struct — it is filtered out in SQL (see
/// [`freshness_query`]) and simply produces no row.
#[derive(Debug, Clone, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct TableLag {
    pub table_name: String,
    pub lag_seconds: i64,
}

/// One CloudWatch datum: the tier it belongs to and its lag value in seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub table: String,
    pub value: f64,
}

/// Map the queried per-tier lags to CloudWatch data — the lag in seconds,
/// verbatim.
pub fn lag_metrics(rows: &[TableLag]) -> Vec<Metric> {
    rows.iter()
        .map(|row| Metric {
            table: row.table_name.clone(),
            value: row.lag_seconds as f64,
        })
        .collect()
}

/// Build the SQL that reads every tier's rollup lag in one round-trip.
///
/// Generated from [`ROLLUP_TIERS`] rather than written out as a literal, so a
/// tier cannot be added to the threshold table and forgotten in the query (or
/// vice versa) — the list is the single source of truth for both.
///
/// Shape, per tier, `UNION ALL`-ed and wrapped:
///
/// ```sql
/// SELECT 'price_ohlcv_1h' AS table_name,
///        toInt64(toUnixTimestamp(now()) - toUnixTimestamp(max(timestamp))) AS lag_seconds
/// FROM price_ohlcv_1h HAVING count() > 0
/// ```
///
/// Four things here are load-bearing, three of them verified against ClickHouse
/// **26.3.10.60** (the pinned prod version) before this shipped:
///
/// - **`HAVING count() > 0` is mandatory, not defensive.** `max()` over zero rows
///   returns the DateTime zero value, `1970-01-01`, **not** NULL and not an empty
///   result — measured on 26.3.10.60, an empty tier yields a lag of
///   `1_786_526_859` seconds (~56 years). Without this gate every tier of a
///   freshly-provisioned environment breaches every threshold on the first run.
///   Gated, an empty tier produces no row → no datum → `NOT_BREACHING` → OK. This
///   is the same "absent means unknown, not broken" shape as task 0056's
///   finding-A gate on `last_push_at IS NOT NULL`.
///
/// - **The union must be wrapped in a subquery for `ORDER BY` to apply.** Written
///   flat, `… UNION ALL SELECT … ORDER BY table_name` binds the `ORDER BY` to the
///   final branch only and the result comes back unsorted — observed directly on
///   26.3.10.60. Ordering is cosmetic (CloudWatch does not care), but an
///   unsorted result makes the probe's own log line and the integration test's
///   assertions order-dependent.
///
/// - **No `FINAL`.** These are `ReplacingMergeTree` tables, but duplicate or
///   superseded rows cannot change a `max()` — an unmerged older version of a row
///   carries the same `timestamp`. Skipping `FINAL` avoids forcing a merge pass
///   on tables of up to 735M rows every 15 minutes for an answer that cannot
///   differ.
///
/// - **`max(timestamp)` is answered from part metadata, not a column scan.**
///   `timestamp` is only the *fourth* sort-key column, so this looks like it
///   should read the whole column — it does not. Measured on 26.3.10.60 against a
///   2M-row, 47-part table: **47 rows / 1.10 KiB / 1 ms**, i.e. one read per part
///   from the per-part min/max index. It was also measured *cheaper* than the
///   equivalent `SELECT max(max_time) FROM system.parts` (15 ms), which is why
///   this reads the tables directly and needs no `system.*` grant at all — see
///   the task's §Access note on BE's XML-managed runtime users.
pub fn freshness_query() -> String {
    let branches: Vec<String> = ROLLUP_TIERS
        .iter()
        .map(|tier| {
            format!(
                "SELECT '{table}' AS table_name, \
                 toInt64(toUnixTimestamp(now()) - toUnixTimestamp(max(timestamp))) AS lag_seconds \
                 FROM {table} HAVING count() > 0",
                table = tier.table
            )
        })
        .collect();

    format!(
        "SELECT table_name, lag_seconds FROM ({}) ORDER BY table_name",
        branches.join(" UNION ALL ")
    )
}

/// Publish `metrics` to CloudWatch under [`METRIC_NAMESPACE`] as
/// [`METRIC_NAME`], each tagged with `Environment` + `Table` dimensions. One
/// `PutMetricData` call for the whole batch.
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
                .dimensions(Dimension::builder().name("Table").value(&m.table).build())
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
    fn lag_maps_verbatim() {
        let rows = vec![TableLag {
            table_name: "price_ohlcv_1h".to_string(),
            lag_seconds: 1_728_000,
        }];
        let m = lag_metrics(&rows);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].table, "price_ohlcv_1h");
        assert_eq!(m[0].value, 1_728_000.0);
    }

    #[test]
    fn every_bound_exceeds_its_bucket_width() {
        // The sawtooth invariant. `timestamp` is the bucket START, so a healthy
        // tier's lag climbs to one full bucket width before the next bucket
        // opens. A bound at or below the width false-fires once per bucket,
        // forever — which is how an alarm gets muted, and a muted alarm is the
        // state 0137 exists to fix.
        for tier in ROLLUP_TIERS {
            assert!(
                tier.lag_bound_seconds > tier.bucket_seconds,
                "{}: bound {}s must exceed bucket width {}s",
                tier.table,
                tier.lag_bound_seconds,
                tier.bucket_seconds
            );
        }
    }

    #[test]
    fn bounds_increase_monotonically_with_granularity() {
        // Tiers are listed fine → coarse; a bound that dipped below its
        // predecessor would mean a coarse tier pages before the fine tier that
        // feeds it, inverting the diagnosis. In 0136 the failure was at
        // `mv_ohlcv_1m_to_15m` and everything downstream inherited it, so the
        // fine tiers must always be the first to report.
        for pair in ROLLUP_TIERS.windows(2) {
            assert!(
                pair[1].lag_bound_seconds > pair[0].lag_bound_seconds,
                "{} bound must exceed {}",
                pair[1].table,
                pair[0].table
            );
        }
    }

    #[test]
    fn query_covers_every_tier_exactly_once() {
        let q = freshness_query();
        for tier in ROLLUP_TIERS {
            assert_eq!(
                q.matches(&format!("FROM {} HAVING", tier.table)).count(),
                1,
                "{} must be selected from exactly once",
                tier.table
            );
        }
        assert_eq!(
            q.matches("UNION ALL").count(),
            ROLLUP_TIERS.len() - 1,
            "n tiers must be joined by n-1 UNION ALLs"
        );
    }

    #[test]
    fn query_gates_empty_tiers_and_wraps_the_union() {
        let q = freshness_query();
        // Empty-tier gate: without it, max() over zero rows yields 1970-01-01
        // and a ~56-year lag that breaches every threshold on a fresh env.
        assert_eq!(
            q.matches("HAVING count() > 0").count(),
            ROLLUP_TIERS.len(),
            "every branch must gate on a non-empty tier"
        );
        // The union must be wrapped or ORDER BY binds to the last branch only.
        assert!(
            q.starts_with("SELECT table_name, lag_seconds FROM ("),
            "union must be wrapped in a subquery, got: {q}"
        );
        assert!(q.trim_end().ends_with(") ORDER BY table_name"));
        // FINAL cannot change a max() and would force a merge pass on up to
        // 735M rows every 15 minutes.
        assert!(!q.contains("FINAL"), "must not use FINAL: {q}");
    }

    #[test]
    fn stalled_tier_breaches_and_fresh_tier_does_not() {
        // The 0136 scenario, in the units the alarm compares: `1h` frozen for 20
        // days against its 3-hour bound, while `1m` keeps flowing.
        let bound = |t: &str| {
            ROLLUP_TIERS
                .iter()
                .find(|x| x.table == t)
                .expect("tier present")
                .lag_bound_seconds
        };
        let rows = vec![
            TableLag {
                table_name: "price_ohlcv_1m".to_string(),
                lag_seconds: 120,
            },
            TableLag {
                table_name: "price_ohlcv_1h".to_string(),
                lag_seconds: 20 * 86_400,
            },
        ];
        let m = lag_metrics(&rows);
        let by = |t: &str| m.iter().find(|x| x.table == t).expect("metric present");
        assert!(by("price_ohlcv_1m").value < bound("price_ohlcv_1m") as f64);
        assert!(by("price_ohlcv_1h").value > bound("price_ohlcv_1h") as f64);
    }
}
