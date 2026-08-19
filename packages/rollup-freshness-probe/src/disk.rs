//! ClickHouse host disk headroom (task 0204, gap 1).
//!
//! The 2026-08-13 stall ran **11.5 hours** and was found by reading Lambda panic
//! logs after the fact. `asset-discovery`, `supply` and `ledger-processor` all
//! failed with ClickHouse `Code: 243` (not enough space); nothing watched the
//! condition itself. The answer was sitting in ClickHouse the entire time.
//!
//! ## Why an alarm is the only lever we have here
//!
//! The Hetzner volume is **shared with the block-explorer team** and we are
//! 3.3% of it (58.93 GiB of 1.72 TiB; their `default` database is 951 GiB). We
//! cannot control what fills it and we cannot free a meaningful amount of space
//! ourselves — so the entire value of this signal is **warning time**. The
//! incident consumed ~150 GiB, which is why the default bound is set in the
//! tens of percent rather than the single digits: it has to fire while there is
//! still room to act, not once writes are already failing.
//!
//! ## ⚠️ Why this reads functions and not `system.disks`
//!
//! The obvious query is `SELECT free_space, total_space FROM system.disks`. It
//! **cannot be used from this probe**, and the failure would not have shown up
//! until it was live on prod:
//!
//! ```text
//! Code: 497. DB::Exception: Not enough privileges. To execute this query,
//! it's necessary to have the grant SELECT ON system.disks. (ACCESS_DENIED)
//! ```
//!
//! Measured on 26.3.10.60 against a user holding exactly `GRANT SELECT ON
//! prices.*` — the shape of the `ingestion` mTLS identity (`prices_writer`) this
//! probe connects as. The grant cannot simply be added either: `prices_writer`
//! is **XML-defined**, and that access storage is read-only, so a runtime
//! `GRANT` fails with `ACCESS_STORAGE_READONLY` (the same wall task 0182 hit
//! trying to give it `ALTER FREEZE PARTITION`). Fixing it would mean an edit to
//! the block-explorer team's `users.xml` and a reload — a cross-team dependency
//! for a metric we can read without one.
//!
//! [`filesystemAvailable`] and [`filesystemCapacity`] are ordinary **functions**,
//! so they carry no table grant at all. Verified on 26.3.10.60 that the same
//! restricted user reads both, and that they return the same numbers as
//! `system.disks` for the default disk (256 786 214 912 vs 256 786 149 376 — the
//! drift is concurrent write activity between the two reads, not a different
//! measurement).
//!
//! ⚠️ This used to say the probe touches no `system.*` table at all. Task 0204
//! gap 3 reads `system.tables`, which is fine and does not weaken the argument
//! above: `system.tables` is grant-**filtered** (a prices-only user simply sees
//! fewer rows), while `system.disks` is grant-**denied** and cannot be granted.
//! The distinction is the whole reason this module reads functions.
//!
//! ⚠️ `filesystemFree()` does **not exist** on 26.3.10.60 (`UNKNOWN_FUNCTION`) —
//! the three that do are `filesystemAvailable`, `filesystemUnreserved` and
//! `filesystemCapacity`. Do not reach for it.
//!
//! ## Which "free" this reports
//!
//! [`filesystemAvailable`] — space available to an unprivileged writer, i.e. it
//! already excludes the root-reserved blocks. That is the honest answer to "can
//! ClickHouse still write", which is the question the alarm exists to answer.
//! `filesystemUnreserved()` additionally subtracts space ClickHouse has
//! earmarked for in-flight merges; it is a *tighter* number that moves with
//! merge activity, so it would make the alarm jitter for reasons unrelated to
//! the disk filling up.
//!
//! [`filesystemAvailable`]: https://clickhouse.com/docs/en/sql-reference/functions/other-functions
//! [`filesystemCapacity`]: https://clickhouse.com/docs/en/sql-reference/functions/other-functions

/// Percent-free metric published to CloudWatch. Watched by the
/// `prices-{env}-ch-disk-free` alarm.
pub const DISK_FREE_PERCENT_METRIC: &str = "ClickHouseDiskFreePercent";

/// Absolute free-bytes metric. Not alarmed on — it exists so the graph next to
/// the alarm answers "how much room is that?" without arithmetic, and so a
/// future absolute-floor alarm has a datum to read.
pub const DISK_FREE_BYTES_METRIC: &str = "ClickHouseDiskFreeBytes";

/// Free-space floor, in percent, below which the alarm fires.
///
/// ⚠️ **This constant is documentation and a test fixture. The deployed
/// threshold lives in `config.opsAlarms.chDiskFreePercent`** and is what the
/// alarm actually compares against — the same split (and the same drift hazard)
/// as `ROLLUP_TIERS` vs `opsAlarms.rollupLagSeconds`. Change both, or neither.
///
/// 20% of the 1.72 TiB volume is ~352 GiB. The 2026-08-13 incident consumed
/// ~150 GiB, so this fires with roughly twice that still free — hours of warning
/// at the rate that event moved. It also sits below the 2026-08-17 measurement
/// of 430.6 GiB free (25.0%), so it does **not** fire on the current steady
/// state; a bound at 25% would have been alarming from the day it shipped.
pub const DISK_FREE_PERCENT_BOUND: f64 = 20.0;

/// One reading of the filesystem backing ClickHouse's data path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct DiskUsage {
    /// Bytes an unprivileged writer can still consume (`filesystemAvailable()`).
    pub available_bytes: u64,
    /// Total size of the filesystem (`filesystemCapacity()`).
    pub capacity_bytes: u64,
}

/// Unit of a published disk datum. Mirrors the CloudWatch units without
/// dragging the AWS SDK into the default (non-`lambda`) build, which is what
/// keeps this module unit-testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskUnit {
    Percent,
    Bytes,
}

/// One CloudWatch datum about the ClickHouse host's disk.
#[derive(Debug, Clone, PartialEq)]
pub struct DiskMetric {
    pub name: &'static str,
    pub value: f64,
    pub unit: DiskUnit,
}

/// The one-row query behind [`DiskUsage`].
///
/// No `FROM`, so it reads no table and needs no grant beyond the ability to run
/// a query — see the module docs for why that matters.
pub fn disk_query() -> &'static str {
    "SELECT filesystemAvailable() AS available_bytes, \
     filesystemCapacity() AS capacity_bytes"
}

/// Free space as a percentage of capacity.
///
/// `None` when capacity reads as zero. That is not a disk that is full — it is a
/// **reading that did not work**, and the two must not be conflated: returning
/// `0.0` would page as if the volume were exhausted, while silently publishing
/// nothing would let the `NOT_BREACHING` alarm score a broken probe as healthy,
/// which is the false-OK failure this whole task exists to close. The caller
/// turns `None` into a failed invocation so the probe's own `-errors` alarm
/// carries it instead.
pub fn free_percent(usage: &DiskUsage) -> Option<f64> {
    if usage.capacity_bytes == 0 {
        return None;
    }
    Some((usage.available_bytes as f64 / usage.capacity_bytes as f64) * 100.0)
}

/// Shape one [`DiskUsage`] reading into the CloudWatch data to publish.
///
/// `None` propagates the unreadable-capacity case from [`free_percent`].
pub fn disk_metrics(usage: &DiskUsage) -> Option<Vec<DiskMetric>> {
    let percent = free_percent(usage)?;
    Some(vec![
        DiskMetric {
            name: DISK_FREE_PERCENT_METRIC,
            value: percent,
            unit: DiskUnit::Percent,
        },
        DiskMetric {
            name: DISK_FREE_BYTES_METRIC,
            value: usage.available_bytes as f64,
            unit: DiskUnit::Bytes,
        },
    ])
}

/// Publish disk data to CloudWatch under [`crate::METRIC_NAMESPACE`], tagged
/// with the `Environment` dimension. One `PutMetricData` call for the batch.
///
/// ## Why these land in the `Prices/Rollup` namespace
///
/// They are not rollup metrics and in isolation a `Prices/ClickHouse` namespace
/// would read better. It is deliberate anyway: the probe Lambda's
/// `PutMetricData` grant is conditioned on `cloudwatch:namespace` equalling
/// `Prices/Rollup` (`eventbridge-stack.ts`), so a new namespace requires editing
/// that stack — **and `eventbridge-stack.ts` is where `CleanupRule` lives**.
/// Every deploy of it can silently re-enable `prices-{env}-cleanup`, which CDK
/// asserts is ENABLED while the live rule is DISABLED, and cleanup running
/// during the 0182/0201 repair campaign destroys that campaign's output as fast
/// as it is written (it cost five days once already). Reusing the namespace
/// keeps this change inside `observability-stack.ts` and off the stack that
/// carries that hazard. Revisit once the campaign has landed.
#[cfg(feature = "lambda")]
pub async fn publish_disk(
    client: &aws_sdk_cloudwatch::Client,
    environment: &str,
    metrics: &[DiskMetric],
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
                .metric_name(m.name)
                .value(m.value)
                .unit(match m.unit {
                    DiskUnit::Percent => StandardUnit::Percent,
                    DiskUnit::Bytes => StandardUnit::Bytes,
                })
                .dimensions(env_dim.clone())
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

    /// The 2026-08-17 measurement: 430.6 GiB free of 1.72 TiB.
    fn measured_2026_08_17() -> DiskUsage {
        DiskUsage {
            available_bytes: 462_320_000_000,
            capacity_bytes: 1_891_000_000_000,
        }
    }

    #[test]
    fn percent_is_available_over_capacity() {
        let u = DiskUsage {
            available_bytes: 250,
            capacity_bytes: 1000,
        };
        assert_eq!(free_percent(&u), Some(25.0));
    }

    #[test]
    fn current_steady_state_does_not_breach() {
        // A bound that fires on the disk's ordinary resting state is an alarm
        // that gets muted, and a muted alarm is the state this task exists to
        // fix. 2026-08-17 measured 25.0% free against a 20% bound.
        let pct = free_percent(&measured_2026_08_17()).expect("capacity readable");
        assert!(
            pct > DISK_FREE_PERCENT_BOUND,
            "measured steady state {pct:.1}% must not breach the {DISK_FREE_PERCENT_BOUND}% bound"
        );
    }

    #[test]
    fn a_repeat_of_the_august_incident_breaches() {
        // The condition the alarm exists for: the 2026-08-13 event consumed
        // ~150 GiB. Starting from the 2026-08-17 steady state, that much again
        // must put us under the bound while there is still room to act.
        let start = measured_2026_08_17();
        let consumed = 150 * 1024_u64.pow(3);
        let after = DiskUsage {
            available_bytes: start.available_bytes - consumed,
            capacity_bytes: start.capacity_bytes,
        };
        let pct = free_percent(&after).expect("capacity readable");
        assert!(
            pct < DISK_FREE_PERCENT_BOUND,
            "a repeat of the incident ({pct:.1}% free) must breach the {DISK_FREE_PERCENT_BOUND}% bound"
        );
        // …and must still leave real headroom, or the warning is worthless.
        assert!(
            after.available_bytes > 100 * 1024_u64.pow(3),
            "the alarm must fire with >100 GiB still free, got {} bytes",
            after.available_bytes
        );
    }

    #[test]
    fn unreadable_capacity_is_not_a_full_disk() {
        // Zero capacity is a broken reading. Reporting it as 0% free would page
        // falsely; publishing nothing would let NOT_BREACHING score it healthy.
        // It must be neither — the caller fails the invocation instead.
        let u = DiskUsage {
            available_bytes: 0,
            capacity_bytes: 0,
        };
        assert_eq!(free_percent(&u), None);
        assert_eq!(disk_metrics(&u), None);
    }

    #[test]
    fn a_genuinely_full_disk_is_zero_not_none() {
        // The other side of the previous test: no space left, but the reading
        // worked. This must publish, and must breach.
        let u = DiskUsage {
            available_bytes: 0,
            capacity_bytes: 1_891_000_000_000,
        };
        assert_eq!(free_percent(&u), Some(0.0));
        let m = disk_metrics(&u).expect("a full disk still publishes");
        assert!(m[0].value < DISK_FREE_PERCENT_BOUND);
    }

    #[test]
    fn publishes_percent_and_bytes() {
        let m = disk_metrics(&measured_2026_08_17()).expect("capacity readable");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].name, DISK_FREE_PERCENT_METRIC);
        assert_eq!(m[0].unit, DiskUnit::Percent);
        assert_eq!(m[1].name, DISK_FREE_BYTES_METRIC);
        assert_eq!(m[1].unit, DiskUnit::Bytes);
        assert_eq!(m[1].value, 462_320_000_000.0);
    }

    #[test]
    fn query_reads_no_table_and_no_system_grant() {
        let q = disk_query();
        // A `FROM` here would mean a table read, and the only table carrying
        // this data is `system.disks`, which the probe's user cannot select
        // from (ACCESS_DENIED, measured on 26.3.10.60) and cannot be granted.
        assert!(
            !q.to_uppercase().contains(" FROM "),
            "disk query must not read a table: {q}"
        );
        assert!(!q.contains("system."), "must not touch system.*: {q}");
        assert!(q.contains("filesystemAvailable()"));
        assert!(q.contains("filesystemCapacity()"));
        // filesystemFree() does not exist on 26.3.10.60.
        assert!(!q.contains("filesystemFree"), "no such function: {q}");
    }
}
