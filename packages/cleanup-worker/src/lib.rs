//! Prices cleanup worker (task 0039) — daily retention by partition drop.
//!
//! Per general-overview §3.6, the fine-grained OHLCV tables and the oracle
//! table are pruned by dropping whole monthly partitions (`PARTITION BY
//! toYYYYMM(timestamp)`); the coarse tables (1h…1M) are kept forever. Dropping
//! a partition is an instant metadata op, not a row-by-row `DELETE`.
//!
//! A monthly partition is droppable only once *all* its data is past the
//! retention window — i.e. its month is strictly older than the month of
//! `now() - retention`. That keeps up to ~1 extra month rather than ever
//! deleting in-window data; correct for monthly partitions.
//!
//! ClickHouse creates partitions implicitly on the first INSERT into a new
//! month, so there is no create-partition-ahead step (a Postgres-ism).

use clickhouse::{Client, Row};
use serde::Deserialize;

/// `(table, retention interval SQL)` per §3.6. Only these tables are pruned;
/// `price_ohlcv_{1h,4h,1d,1w,1M}` are retained forever.
///
/// ⚠️ **`prices.usd_rate` is deliberately ABSENT and must stay that way**
/// (task 0167). This list is opt-in, so an unlisted table is retained forever —
/// which is the entire point of that table. It exists precisely *because*
/// `oracle_prices` expires at 13 months and takes the earliest depeg-aware
/// history with it; `usd_rate` is the forever-retained snapshot that escapes
/// that. Adding it here would silently re-create the problem it was built to
/// solve, and the loss would be unrecoverable rather than merely wrong.
/// See the block comment above `CREATE TABLE prices.usd_rate` in `init.sql`.
pub const RETENTION: &[(&str, &str)] = &[
    ("price_ohlcv_1m", "INTERVAL 7 DAY"),
    ("price_ohlcv_15m", "INTERVAL 30 DAY"),
    ("oracle_prices", "INTERVAL 13 MONTH"),
];

#[derive(Debug, thiserror::Error)]
pub enum CleanupError {
    #[error(transparent)]
    Clickhouse(#[from] clickhouse::error::Error),
}

#[derive(Debug, Row, Deserialize)]
struct PartitionRow {
    partition: String,
}

/// What a [`run_cleanup`] pass dropped (e.g. `"price_ohlcv_1m=202405"`).
#[derive(Debug, Default, Clone)]
pub struct CleanupStats {
    pub dropped: Vec<String>,
}

/// Drop every monthly partition older than its table's retention window.
///
/// Idempotent: a second run on the same day finds nothing left to drop. The
/// table names + intervals are compile-time constants (no injection surface);
/// partition values come from `system.parts` (ClickHouse-controlled).
pub async fn run_cleanup(client: &Client) -> Result<CleanupStats, CleanupError> {
    let mut stats = CleanupStats::default();

    for (table, interval) in RETENTION {
        // Partitions whose whole month is older than now() - retention.
        let query = format!(
            "SELECT DISTINCT partition FROM system.parts \
             WHERE database = 'prices' AND table = '{table}' AND active = 1 \
               AND toUInt32(partition) < toYYYYMM(now() - {interval})"
        );
        let partitions: Vec<PartitionRow> = client.query(&query).fetch_all().await?;

        for p in partitions {
            // For toYYYYMM partitioning the partition id IS the numeric YYYYMM.
            let drop = format!("ALTER TABLE prices.{table} DROP PARTITION {}", p.partition);
            client.query(&drop).execute().await?;
            tracing::info!(table, partition = %p.partition, "dropped expired partition");
            stats.dropped.push(format!("{table}={}", p.partition));
        }
    }

    Ok(stats)
}

#[cfg(test)]
mod usd_rate_retention_tests {
    use super::RETENTION;

    /// Task 0167. `prices.usd_rate` exists precisely BECAUSE `oracle_prices` is
    /// pruned at 13 months and takes the earliest depeg-aware history with it —
    /// unrecoverably, since the readings cannot be re-derived after the fact.
    ///
    /// `RETENTION` is an opt-in allowlist, so the protection is an ABSENCE, and
    /// an absence is exactly the invariant a future reader breaks by adding one
    /// tidy-looking line. This test is that line's tripwire.
    #[test]
    fn usd_rate_is_never_pruned() {
        let listed: Vec<&str> = RETENTION.iter().map(|(t, _)| *t).collect();
        assert!(
            !listed.contains(&"usd_rate"),
            "usd_rate must never be pruned — it is the forever-retained snapshot \
             of oracle_prices, which IS pruned. Adding it here re-creates the \
             exact unrecoverable data loss the table was built to escape. \
             Listed: {listed:?}"
        );
        assert!(
            listed.contains(&"oracle_prices"),
            "sanity: oracle_prices must still be pruned, or this test proves nothing"
        );
    }
}
