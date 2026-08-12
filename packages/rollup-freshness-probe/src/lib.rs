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

/// Lag published for a tier that is **empty when it should not be**.
///
/// Chosen to exceed every bound in [`ROLLUP_TIERS`] by an order of magnitude, so
/// it unambiguously breaches whatever threshold the operator has tuned, and is
/// obviously synthetic in a CloudWatch graph rather than looking like a real
/// measurement. See [`lag_metrics`] for when it is emitted — an empty tier is
/// *not* automatically anomalous.
pub const EMPTY_TIER_SENTINEL_SECONDS: i64 = 10 * 365 * 86_400;

/// One OHLCV granularity and the age bound beyond which its data is considered
/// stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollupTier {
    /// Unqualified table name. The probe binds its client to the `prices`
    /// database, so the query references these unqualified (see [`freshness_query`]).
    pub table: &'static str,
    /// Width of one bucket in this tier, in seconds. Not a threshold — it is
    /// part of the floor the threshold must clear. See the note on
    /// [`ROLLUP_TIERS`].
    pub bucket_seconds: i64,
    /// Refresh interval of the materialized view that *feeds* this tier
    /// (`schema/rollups.sql`), in seconds. The other half of the floor: a bucket
    /// is not merely `bucket_seconds` late in the worst case, it is that plus
    /// however long the feeding MV waits before it runs. `price_ohlcv_1m` is
    /// written by ingestion rather than by an MV, so its value is 0.
    pub mv_refresh_seconds: i64,
    /// Extra delay before this tier's newest bucket can exist at all, beyond
    /// bucket width and MV refresh. Zero for every tier except `price_ohlcv_1M`
    /// — see [`ROLLUP_TIERS`] for why that one needs six days.
    pub alignment_slack_seconds: i64,
    /// Alarm threshold: publish-to-breach age in seconds.
    pub lag_bound_seconds: i64,
}

impl RollupTier {
    /// Worst-case lag a **healthy** tier reaches: one full bucket width, plus one
    /// refresh interval of the MV feeding it, plus any bucket-alignment slack.
    /// Any bound at or below this false-fires on ordinary cadence, forever.
    pub const fn healthy_peak_seconds(&self) -> i64 {
        self.bucket_seconds + self.mv_refresh_seconds + self.alignment_slack_seconds
    }
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
/// The healthy peak is **not** just the bucket width — it is the bucket width
/// plus the refresh interval of the MV feeding the tier (`schema/rollups.sql`),
/// because the bucket cannot appear until that MV next runs:
///
/// | tier | bucket | feeding MV | healthy peak | bound | true headroom |
/// |---|---|---|---|---|---|
/// | `price_ohlcv_1m` | 1 min | *(ingestion)* | 1 min | 15 min | 14 min |
/// | `price_ohlcv_15m` | 15 min | 1 min | 16 min | 1 h | 44 min |
/// | `price_ohlcv_1h` | 1 h | 15 min | 1 h 15 min | 3 h | 1 h 45 min |
/// | `price_ohlcv_4h` | 4 h | 1 h | 5 h | 12 h | 7 h |
/// | `price_ohlcv_1d` | 1 d | 4 h | 28 h | 48 h | 20 h |
/// | `price_ohlcv_1w` | 7 d | 1 d | 8 d | 10 d | 2 d |
/// | `price_ohlcv_1M` | ~31 d | 1 d | **38 d** | 45 d | 7 d |
///
/// ⚠️ **`1M` carries six extra days of alignment slack**, which is why its peak
/// is 38 d and not 32 d. Buckets are weeks-attributed-by-start, not calendar
/// months: a month's bucket does not exist until a week actually *starts* inside
/// that month, and since weeks start every 7 days that can be up to 6 days into
/// the month. Until then the newest bucket is still the previous month's, so the
/// lag is `6 d + previous month length + MV refresh`. It is the tightest tier
/// here — 7 d of headroom against a 45 d bound — so treat any proposal to lower
/// it with suspicion.
///
/// The bounds are deliberately loose. This alarm exists to catch *a number that
/// stopped moving* — 0136 ran for nine days — not to measure rollup latency. A
/// tight bound that pages on ordinary cadence jitter would be turned off, and an
/// alarm that is off is the state 0137 was filed to fix. Task 0104 owns the
/// cadence-vs-window question; these bounds are set not to contradict it.
/// ⚠️ **Order is load-bearing: fine → coarse.** [`lag_metrics`] relies on it to
/// decide whether an empty tier is anomalous.
pub const ROLLUP_TIERS: &[RollupTier] = &[
    RollupTier {
        table: "price_ohlcv_1m",
        bucket_seconds: 60,
        // Written directly by ingestion, not by a rollup MV.
        mv_refresh_seconds: 0,
        alignment_slack_seconds: 0,
        lag_bound_seconds: 15 * 60,
    },
    RollupTier {
        table: "price_ohlcv_15m",
        bucket_seconds: 15 * 60,
        // mv_ohlcv_1m_to_15m REFRESH EVERY 1 MINUTE
        mv_refresh_seconds: 60,
        alignment_slack_seconds: 0,
        lag_bound_seconds: 60 * 60,
    },
    RollupTier {
        table: "price_ohlcv_1h",
        bucket_seconds: 60 * 60,
        // mv_ohlcv_15m_to_1h REFRESH EVERY 15 MINUTE
        mv_refresh_seconds: 15 * 60,
        alignment_slack_seconds: 0,
        lag_bound_seconds: 3 * 60 * 60,
    },
    RollupTier {
        table: "price_ohlcv_4h",
        bucket_seconds: 4 * 60 * 60,
        // mv_ohlcv_1h_to_4h REFRESH EVERY 1 HOUR
        mv_refresh_seconds: 60 * 60,
        alignment_slack_seconds: 0,
        lag_bound_seconds: 12 * 60 * 60,
    },
    RollupTier {
        table: "price_ohlcv_1d",
        bucket_seconds: 86_400,
        // mv_ohlcv_4h_to_1d REFRESH EVERY 4 HOUR
        mv_refresh_seconds: 4 * 60 * 60,
        alignment_slack_seconds: 0,
        lag_bound_seconds: 48 * 60 * 60,
    },
    RollupTier {
        table: "price_ohlcv_1w",
        bucket_seconds: 7 * 86_400,
        // mv_ohlcv_1d_to_1w REFRESH EVERY 1 DAY
        mv_refresh_seconds: 86_400,
        alignment_slack_seconds: 0,
        lag_bound_seconds: 10 * 86_400,
    },
    RollupTier {
        table: "price_ohlcv_1M",
        bucket_seconds: 31 * 86_400,
        // mv_ohlcv_1w_to_1M REFRESH EVERY 1 DAY
        mv_refresh_seconds: 86_400,
        // A month's 1M bucket does not exist until a week actually STARTS inside
        // that month, which can be up to 6 days in.
        alignment_slack_seconds: 6 * 86_400,
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

/// Map the queried per-tier lags to CloudWatch data, and synthesise a breaching
/// datum for any tier that is **empty when it should not be**.
///
/// ## Why an empty tier cannot simply be skipped
///
/// [`freshness_query`] emits no row for a tier with zero rows, and the alarms are
/// `treatMissingData: NOT_BREACHING`. Publishing nothing for an empty tier
/// therefore scores it **healthy** — which re-opens a version of the very blind
/// spot task 0137 exists to close:
///
/// - `price_ohlcv_15m` is retained for **30 days** and `price_ohlcv_1m` for
///   **7 days**, by `cleanup-worker` dropping whole monthly partitions. A
///   0136-style freeze alarms correctly at first, but once cleanup has dropped
///   the last remaining partition the table is *empty*, the datum disappears,
///   and the alarm transitions back to **OK — firing a false "recovered" into
///   Slack while the tier is still frozen.**
/// - A coarse tier left empty by a `DETACH`/`ATTACH` recovery, or an MV that was
///   never created, would likewise never be able to alarm.
///
/// ## But an empty tier is not automatically broken either
///
/// A freshly-provisioned environment has every tier empty, and stays that way
/// for the coarse tiers for a long time — `price_ohlcv_1M` cannot have a bucket
/// until a week *starts* inside the current month. Treating "empty" as
/// "breaching" would page on all seven alarms during bootstrap and keep `1M`
/// firing for up to a month.
///
/// ## The rule
///
/// Data flows fine → coarse, so a coarser tier can only hold data if every finer
/// tier held data first. Therefore:
///
/// > **An empty tier is anomalous if and only if some COARSER tier is populated.**
///
/// - fresh environment, everything empty → nothing published → all OK ✅
/// - bootstrap: `1m` filling, coarse tiers not yet rolled → no coarser tier is
///   populated → nothing synthesised → no false page ✅
/// - `15m` frozen past its 30-day retention until empty, while `1h`/`1d`/`1w`
///   still hold history → a coarser tier IS populated → sentinel → **the alarm
///   stays firing instead of falsely recovering** ✅
/// - `1d` emptied by a botched `DETACH`/`ATTACH` while `1w`/`1M` are intact →
///   sentinel ✅
///
/// ⚠️ **Known limit: the coarsest tier (`price_ohlcv_1M`) has no coarser tier**,
/// so an empty `1M` can never be flagged this way. Catching that needs a
/// different signal — see task 0179.
pub fn lag_metrics(rows: &[TableLag]) -> Vec<Metric> {
    let populated = |table: &str| rows.iter().any(|r| r.table_name == table);

    let mut metrics: Vec<Metric> = rows
        .iter()
        .map(|row| Metric {
            table: row.table_name.clone(),
            value: row.lag_seconds as f64,
        })
        .collect();

    for (i, tier) in ROLLUP_TIERS.iter().enumerate() {
        if populated(tier.table) {
            continue;
        }
        // ROLLUP_TIERS is ordered fine → coarse, so everything after `i` is
        // coarser than this tier.
        let coarser_is_populated = ROLLUP_TIERS[i + 1..].iter().any(|t| populated(t.table));
        if coarser_is_populated {
            metrics.push(Metric {
                table: tier.table.to_string(),
                value: EMPTY_TIER_SENTINEL_SECONDS as f64,
            });
        }
    }

    metrics.sort_by(|a, b| a.table.cmp(&b.table));
    metrics
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
        // Every tier populated, so no sentinel is synthesised and each measured
        // lag passes through unchanged.
        let rows: Vec<TableLag> = ROLLUP_TIERS
            .iter()
            .enumerate()
            .map(|(i, tier)| TableLag {
                table_name: tier.table.to_string(),
                lag_seconds: 100 + i as i64,
            })
            .collect();
        let m = lag_metrics(&rows);
        assert_eq!(m.len(), ROLLUP_TIERS.len());
        for (i, tier) in ROLLUP_TIERS.iter().enumerate() {
            let got = m
                .iter()
                .find(|x| x.table == tier.table)
                .expect("tier present");
            assert_eq!(got.value, (100 + i as i64) as f64);
        }
    }

    #[test]
    fn every_bound_exceeds_its_healthy_peak() {
        // The sawtooth invariant. `timestamp` is the bucket START, so a healthy
        // tier's lag climbs to one full bucket width — PLUS the refresh interval
        // of the MV feeding it, since the bucket cannot appear until that MV
        // next runs. A bound at or below that false-fires once per bucket,
        // forever — which is how an alarm gets muted, and a muted alarm is the
        // state 0137 exists to fix.
        for tier in ROLLUP_TIERS {
            assert!(
                tier.lag_bound_seconds > tier.healthy_peak_seconds(),
                "{}: bound {}s must exceed healthy peak {}s (bucket {}s + MV refresh {}s)",
                tier.table,
                tier.lag_bound_seconds,
                tier.healthy_peak_seconds(),
                tier.bucket_seconds,
                tier.mv_refresh_seconds
            );
        }
    }

    #[test]
    fn sentinel_breaches_every_bound() {
        // The synthesised empty-tier value has to breach whatever threshold the
        // operator tuned, or the sentinel is decorative.
        for tier in ROLLUP_TIERS {
            assert!(
                EMPTY_TIER_SENTINEL_SECONDS > tier.lag_bound_seconds,
                "sentinel must breach {}'s bound",
                tier.table
            );
        }
    }

    #[test]
    fn fresh_environment_publishes_nothing() {
        // Every tier empty: a newly provisioned environment. Publishing a
        // breaching sentinel here would page on all seven alarms at once.
        assert!(lag_metrics(&[]).is_empty());
    }

    #[test]
    fn bootstrap_does_not_synthesise_for_unrolled_coarse_tiers() {
        // `1m` is filling but nothing has rolled up yet. Every empty tier is
        // COARSER than the only populated one, so none is anomalous.
        let rows = vec![TableLag {
            table_name: "price_ohlcv_1m".to_string(),
            lag_seconds: 120,
        }];
        let m = lag_metrics(&rows);
        assert_eq!(m.len(), 1, "bootstrap must not synthesise sentinels: {m:?}");
        assert_eq!(m[0].table, "price_ohlcv_1m");
    }

    #[test]
    fn emptied_fine_tier_still_breaches_when_coarser_tiers_hold_data() {
        // The false-recovery this rule exists to prevent: `15m` froze, cleanup
        // dropped its last partition (30-day retention), so the query returns no
        // row for it — while the forever-tables still hold history. Without the
        // sentinel the alarm would flip to OK and announce a recovery that did
        // not happen.
        let rows = vec![
            TableLag {
                table_name: "price_ohlcv_1h".to_string(),
                lag_seconds: 40 * 86_400,
            },
            TableLag {
                table_name: "price_ohlcv_1d".to_string(),
                lag_seconds: 40 * 86_400,
            },
        ];
        let m = lag_metrics(&rows);
        let fifteen = m
            .iter()
            .find(|x| x.table == "price_ohlcv_15m")
            .expect("emptied 15m must still publish a datum");
        assert_eq!(fifteen.value, EMPTY_TIER_SENTINEL_SECONDS as f64);
        // `1m` is finer than the populated `1h`, so it is flagged too.
        assert!(m.iter().any(|x| x.table == "price_ohlcv_1m"));
        // `1w`/`1M` are COARSER than everything populated — they may simply not
        // have rolled yet, so they must NOT be synthesised.
        assert!(!m.iter().any(|x| x.table == "price_ohlcv_1w"));
        assert!(!m.iter().any(|x| x.table == "price_ohlcv_1M"));
    }

    #[test]
    fn tiers_are_ordered_fine_to_coarse() {
        // lag_metrics' "is any COARSER tier populated" test is positional, so a
        // reordering of ROLLUP_TIERS would silently invert the rule.
        for pair in ROLLUP_TIERS.windows(2) {
            assert!(
                pair[1].bucket_seconds > pair[0].bucket_seconds,
                "{} must be coarser than {}",
                pair[1].table,
                pair[0].table
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
