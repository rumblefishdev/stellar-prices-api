//! Coarse buckets that disagree with their source tier (task 0203 AC 4).
//!
//! The freshness alarm (task 0137) measures the **tip** — how old the newest
//! bucket of each tier is. On 2026-08-13 the tip was current while eight
//! buckets behind it were missing, and a tip check cannot see that: **a hole
//! behind a healthy tip is invisible to a staleness signal.** This module is
//! the completeness signal: per coarse tier, how many closed buckets in the
//! reconciliation window disagree with the tier directly below.
//!
//! The SQL is the generator's
//! [`prices_clickhouse::rollup_sql::reconcile_mismatch_select`] verbatim — the
//! reconciliation MV's own SELECT, counted — so the probe measures exactly
//! what the hourly pass would rewrite: missing or disagreeing on
//! `sum(trade_count)` / `sum(volume_base)`, never on `version`.
//!
//! ## Why only closed buckets past a grace
//!
//! The open bucket of every tier always disagrees with its source for a
//! while: the fast MVs roll it on their own cadence (1h every 15 min, 1d every
//! 4 h, 1w/1M daily), and a just-closed one may still lag by ingest delay. The
//! reconciliation pass itself therefore rewrites only buckets whose END is at
//! least [`prices_clickhouse::rollup_sql::MISMATCH_GRACE`] (2 h) old (review
//! WR-06) — the open bucket stays the fast MVs' — and since this count IS that
//! SELECT, it carries the same bound: without it the metric would never be
//! zero. The alarm then holds for 90 min on top, longer than one reconcile
//! cycle, so a back-dated arrival that the next pass heals does not page.
//!
//! ## Which tier shows a hole first
//!
//! Each tier is compared only with the tier directly below. A hole in `_1m`
//! leaves every coarse tier equally holed after the fast chain runs, so
//! 1h..1M *agree* with their sources and only `price_ohlcv_15m` mismatches.
//! An upper tier mismatches on its own only when the chain breaks part-way —
//! a reconcile MV stuck or STOPped (see [`crate::refresh_waits`]) or a loss at
//! a coarse level.
//!
//! ## Cost and order
//!
//! Each read is a `GROUP BY` over the 7-day window of the child tier. They run
//! LAST in the invocation, cheapest first ([`mismatch_queries`]: 1M, 1w, 1d,
//! 4h, 1h, 15m — the 15m read scans a week of `_1m`), each under an execution
//! bound, so a slow read loses only the tiers after it rather than any other
//! check.

use crate::Metric;
use prices_clickhouse::rollup_sql::{TIERS, reconcile_mismatch_select};

/// Per-tier count of closed coarse buckets that disagree with their source,
/// published with `Environment` + `Table` dimensions. Watched by
/// `prices-{env}-rollup-mismatch-<tier>`.
pub const RECONCILE_MISMATCH_METRIC: &str = "RollupMismatchBuckets";

/// The single row of [`reconcile_mismatch_select`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct MismatchCount {
    pub mismatched: u64,
}

/// One `(target table, SQL)` per coarse tier, cheapest first: 1M, 1w, 1d, 4h,
/// 1h, 15m. The SQL is qualified with the probe's constant `prices` database.
pub fn mismatch_queries() -> Vec<(&'static str, String)> {
    TIERS
        .iter()
        .rev()
        .map(|tier| {
            (
                tier.target,
                reconcile_mismatch_select(tier, "prices")
                    .expect("`prices` is a valid database identifier"),
            )
        })
        .collect()
}

/// Shape one tier's count into the datum [`publish_mismatch`] sends.
pub fn mismatch_metric(table: &str, count: &MismatchCount) -> Metric {
    Metric {
        table: table.to_string(),
        value: count.mismatched as f64,
    }
}

/// Publish per-tier mismatch counts to CloudWatch under
/// [`crate::METRIC_NAMESPACE`] as [`RECONCILE_MISMATCH_METRIC`], each tagged
/// with `Environment` + `Table` dimensions — the same keying as the freshness
/// metric, so each tier has its own alarm.
#[cfg(feature = "lambda")]
pub async fn publish_mismatch(
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
                .metric_name(RECONCILE_MISMATCH_METRIC)
                .value(m.value)
                .unit(StandardUnit::Count)
                .dimensions(env_dim.clone())
                .dimensions(Dimension::builder().name("Table").value(&m.table).build())
                .build()
        })
        .collect::<Vec<_>>();

    client
        .put_metric_data()
        .namespace(crate::METRIC_NAMESPACE)
        .set_metric_data(Some(data))
        .send()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_queries_one_per_coarse_target_cheapest_first() {
        let q = mismatch_queries();
        let tables: Vec<&str> = q.iter().map(|(t, _)| *t).collect();
        assert_eq!(
            tables,
            [
                "price_ohlcv_1M",
                "price_ohlcv_1w",
                "price_ohlcv_1d",
                "price_ohlcv_4h",
                "price_ohlcv_1h",
                "price_ohlcv_15m",
            ]
        );
    }

    /// The probe runs the generator's SQL verbatim — it must not fork the
    /// comparison the reconcile MV makes.
    #[test]
    fn each_query_is_the_generators_mismatch_select() {
        for (table, sql) in mismatch_queries() {
            let tier = TIERS
                .iter()
                .find(|t| t.target == table)
                .expect("a coarse target");
            assert_eq!(
                sql,
                reconcile_mismatch_select(tier, "prices").expect("a checked rendering")
            );
        }
    }

    #[test]
    fn the_metric_carries_the_table_and_the_count() {
        let m = mismatch_metric("price_ohlcv_15m", &MismatchCount { mismatched: 7 });
        assert_eq!(
            m,
            Metric {
                table: "price_ohlcv_15m".into(),
                value: 7.0
            }
        );
        let zero = mismatch_metric("price_ohlcv_1M", &MismatchCount { mismatched: 0 });
        assert_eq!(zero.table, "price_ohlcv_1M");
        assert_eq!(zero.value, 0.0);
    }
}
