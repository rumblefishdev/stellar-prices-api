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
//! `sum(trade_count)` / `sum(volume_base)`, never on `version`. It is
//! source-driven, so an EXTRA target bucket (no source rows behind it) is not
//! counted — see the generator's doc (review IN-04).
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
//! ## Cost, order and time budget (review WR-04)
//!
//! Each read is a `GROUP BY` over the 7-day window of the child tier. They run
//! LAST in the invocation, so a slow read can cost only the tiers after it,
//! never another check.
//!
//! - **15m first** ([`mismatch_queries`]: 15m, 1h, 4h, 1d, 1w, 1M). It is the
//!   heaviest read (a week of `_1m`), and it is where a `_1m` hole shows
//!   first — the one tier the phase must not lose to a slow day.
//! - **A budget, not only a per-read bound.** Six reads at a 10 s bound are the
//!   whole 60 s Lambda timeout on their own, after the reads before them. So
//!   each read gets `min(10 s, time left − 5 s reserve)` as its server-side
//!   `max_execution_time` ([`mismatch_read_bound`]); a read that would get less
//!   than 2 s is SKIPPED and recorded as a failure of the invocation (the
//!   probe's `-errors` alarm), instead of a hard Lambda timeout that publishes
//!   nothing and leaves the MISSING-data mismatch alarms holding their old state.

use crate::Metric;
use prices_clickhouse::rollup_sql::{TIERS, reconcile_mismatch_select};
use std::time::Duration;

/// Per-tier count of closed coarse buckets that disagree with their source,
/// published with `Environment` + `Table` dimensions. Watched by
/// `prices-{env}-rollup-mismatch-<tier>`.
pub const RECONCILE_MISMATCH_METRIC: &str = "RollupMismatchBuckets";

/// The single row of [`reconcile_mismatch_select`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct MismatchCount {
    pub mismatched: u64,
}

/// The server-side bound on ONE mismatch read, in seconds, when time allows.
pub const MISMATCH_READ_BOUND_SECS: u64 = 10;

/// Kept back from the invocation deadline for publishing, logging and
/// returning — a read may never eat into it.
pub const MISMATCH_RESERVE: Duration = Duration::from_secs(5);

/// A read offered fewer seconds than this is skipped. `max_execution_time` is
/// whole seconds, and the measured 15m read ran 0.7–4 s on 26.3.10.60 (about
/// 3 M `_1m` rows), so less than this would only produce a timeout error.
pub const MISMATCH_MIN_READ_SECS: u64 = 2;

/// The `max_execution_time` for the next mismatch read, given the time left in
/// the invocation — or `None` to skip it (and record the skip as a failure).
pub fn mismatch_read_bound(remaining: Duration) -> Option<u64> {
    let usable = remaining.checked_sub(MISMATCH_RESERVE)?.as_secs();
    let bound = usable.min(MISMATCH_READ_BOUND_SECS);
    (bound >= MISMATCH_MIN_READ_SECS).then_some(bound)
}

/// One `(target table, SQL)` per coarse tier, fine to coarse: 15m FIRST (the
/// heaviest read, and where a `_1m` hole shows first), then 1h, 4h, 1d, 1w,
/// 1M. The SQL is qualified with the probe's constant `prices` database.
pub fn mismatch_queries() -> Vec<(&'static str, String)> {
    TIERS
        .iter()
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

    /// Review WR-04: the 15m read goes first — a slow day must not cost the
    /// one tier where a `_1m` hole shows first.
    #[test]
    fn six_queries_one_per_coarse_target_15m_first() {
        let q = mismatch_queries();
        let tables: Vec<&str> = q.iter().map(|(t, _)| *t).collect();
        assert_eq!(
            tables,
            [
                "price_ohlcv_15m",
                "price_ohlcv_1h",
                "price_ohlcv_4h",
                "price_ohlcv_1d",
                "price_ohlcv_1w",
                "price_ohlcv_1M",
            ]
        );
    }

    /// Plenty of time: the full per-read bound. Less: what is left after the
    /// reserve. Too little (or past the deadline): skip, never a 0 or 1 s bound.
    #[test]
    fn the_read_bound_is_capped_by_the_time_left_and_skips_below_the_minimum() {
        let s = Duration::from_secs;
        assert_eq!(mismatch_read_bound(s(55)), Some(MISMATCH_READ_BOUND_SECS));
        assert_eq!(mismatch_read_bound(s(15)), Some(10));
        assert_eq!(mismatch_read_bound(s(12)), Some(7));
        assert_eq!(mismatch_read_bound(Duration::from_millis(7_900)), Some(2));
        assert_eq!(mismatch_read_bound(Duration::from_millis(6_999)), None);
        assert_eq!(mismatch_read_bound(s(5)), None);
        assert_eq!(mismatch_read_bound(s(0)), None);
    }

    /// The whole phase fits the reserve: however the time is spent, reads that
    /// each take their full bound (plus the 2 s client guard `main.rs` puts
    /// around each) never run past the deadline.
    #[test]
    fn six_reads_at_their_bound_never_outlive_the_invocation() {
        for start in [60_u64, 45, 30, 20, 12, 8, 3] {
            let mut left = Duration::from_secs(start);
            for _ in mismatch_queries() {
                match mismatch_read_bound(left) {
                    Some(bound) => {
                        let spent = Duration::from_secs(bound + 2);
                        assert!(
                            spent < left,
                            "start {start}: a read outlived the invocation"
                        );
                        left -= spent;
                    }
                    None => break,
                }
            }
        }
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
