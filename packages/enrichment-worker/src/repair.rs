//! Coarse-table USD repair driver (task 0114).
//!
//! Enrichment only ever writes `price_ohlcv_1m`; the coarse rollups
//! (`_15m … _1M`) capture whatever USD value 1m held *at roll-up time* and are
//! never revisited, so `close_usd` / `volume_quote_usd` froze at zero for the
//! Soroban era (task 0114 defect 1) and the 2025-02 → 2026-02 historical march
//! never ran (defect 2). This driver re-runs the existing enrichment tiers
//! **directly against a coarse table**, one monthly partition at a time.
//!
//! ## Partition-bounded (task 0111 option 1)
//!
//! Each month is repaired with [`ChEnrichConfig::time_window`] set to that
//! month's `[start, end)`, so every candidate scan prunes to a single
//! `toYYYYMM(timestamp)` partition instead of full-scanning the table. Per-month
//! cost is therefore independent of how many historical partitions the table
//! holds — the same property 0111 needs. One fix shape covers both tasks.
//!
//! ## Non-destructive (task 0114 data-safety AC)
//!
//! The repair is a pure additive enrichment `INSERT … SELECT`: OHLC/volume are
//! carried through verbatim, only the USD columns are computed, and the corrected
//! row wins by `version + 1` (proven in `ch_enrich_it.rs`). There is **no**
//! `TRUNCATE`/rebuild path here — for 2025-02 → 2026-02 the 1m source was dropped
//! by cleanup and the coarse tables are the sole surviving copy, so a
//! truncate-rebuild would destroy them. With `snapshot = true` each partition is
//! `FREEZE`d first (a cheap server-side hardlink under `shadow/`) before it is
//! touched, so a bad run can be reverted with `ATTACH PARTITION`.

use clickhouse::{Client, Row};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::ch_enrich::{ChEnrichConfig, ChEnrichError, ChEnrichmentPass};

/// One coarse-repair run over `[start_month, end_month]` (inclusive `YYYYMM`).
#[derive(Debug, Clone)]
pub struct CoarseRepairConfig {
    /// Base enrichment config. `table` must be a coarse table (e.g.
    /// `price_ohlcv_1h`); `time_window` and `one_shot` are set per month by the
    /// driver and any pre-set values are overwritten.
    pub enrich: ChEnrichConfig,
    /// First month to repair, inclusive, as `YYYYMM` (e.g. `202502`).
    pub start_month: u32,
    /// Last month to repair, inclusive, as `YYYYMM` (e.g. `202602`).
    pub end_month: u32,
    /// `FREEZE` each partition before repairing it (sole-copy safety). Leave
    /// `true` for any run against a span whose 1m source is gone.
    pub snapshot: bool,
    /// Enumerate the months-with-zeros and report their counts, but write
    /// nothing (no snapshot, no enrichment). The operator's preview before
    /// committing a real run against the shared prod cluster.
    pub dry_run: bool,
}

/// A month that still holds enrichable zeros, with the exact `[start, end)`
/// unix-second window the driver hands to [`ChEnrichConfig::time_window`].
#[derive(Debug, Clone, Row, Deserialize)]
struct MonthWindow {
    month: u32,
    start_ts: u32,
    end_ts: u32,
    zeros: u64,
}

/// Per-month outcome. `zeros_after > 0` is expected and correct — it is the
/// genuine `no_reference` floor (exotic quotes with no USD path), not a failure.
#[derive(Debug, Clone, Serialize)]
pub struct MonthRepair {
    pub month: u32,
    pub zeros_before: u64,
    pub zeros_after: u64,
    pub rows_enriched: u64,
    /// The `FREEZE … WITH NAME` label, when `snapshot` was set.
    pub snapshot_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RepairSummary {
    pub months: Vec<MonthRepair>,
}

impl RepairSummary {
    pub fn total_enriched(&self) -> u64 {
        self.months.iter().map(|m| m.rows_enriched).sum()
    }
    /// Rows still at zero after the run — the `no_reference` floor across all
    /// months. Not an error; recorded so the caller can log the residual.
    pub fn total_remaining(&self) -> u64 {
        self.months.iter().map(|m| m.zeros_after).sum()
    }
}

pub struct CoarseRepairDriver {
    client: Client,
    cfg: CoarseRepairConfig,
}

impl CoarseRepairDriver {
    pub fn new(cfg: CoarseRepairConfig) -> Self {
        let client = Client::default()
            .with_url(&cfg.enrich.url)
            .with_database(&cfg.enrich.database);
        Self { client, cfg }
    }

    /// Build from a pre-constructed client (e.g. the mTLS client used against
    /// prod). Mirrors [`ChEnrichmentPass::with_client`].
    pub fn with_client(client: Client, cfg: CoarseRepairConfig) -> Self {
        let client = client.with_database(&cfg.enrich.database);
        Self { client, cfg }
    }

    /// Cold-start health check.
    pub async fn preflight(&self) -> Result<(), ChEnrichError> {
        self.client.query("SELECT 1").execute().await?;
        Ok(())
    }

    /// The months in the span that still hold enrichable zeros, each with its
    /// exact `[start, end)` window. A single grouped scan; months with no zeros
    /// (already fully enriched, or exotic-only handled below) are simply absent.
    /// `min(toStartOfMonth(...))` is constant within a `YYYYMM` group, so it pins
    /// the month's first instant; `addMonths(..., 1)` is the exclusive upper
    /// bound. Both cast to `UInt32` to match [`MonthWindow`].
    async fn months_with_zeros(&self) -> Result<Vec<MonthWindow>, ChEnrichError> {
        let sql = format!(
            "SELECT toYYYYMM(timestamp) AS month, \
                    toUInt32(toUnixTimestamp(toDateTime(min(toStartOfMonth(timestamp))))) AS start_ts, \
                    toUInt32(toUnixTimestamp(toDateTime(addMonths(min(toStartOfMonth(timestamp)), 1)))) AS end_ts, \
                    count() AS zeros \
             FROM {db}.{tbl} FINAL \
             WHERE (volume_quote_usd = 0 OR close_usd = 0) AND volume_quote > 0 \
               AND toYYYYMM(timestamp) BETWEEN {start} AND {end} \
             GROUP BY month ORDER BY month",
            db = self.cfg.enrich.database,
            tbl = self.cfg.enrich.table,
            start = self.cfg.start_month,
            end = self.cfg.end_month,
        );
        Ok(self.client.query(&sql).fetch_all::<MonthWindow>().await?)
    }

    /// `FREEZE` one monthly partition to a named server-side hardlink snapshot
    /// under `shadow/`. Non-destructive and cheap (hardlinks, no data copy).
    /// Revert with `ALTER TABLE … ATTACH PARTITION … FROM …` (operator step).
    /// For `toYYYYMM` partitioning the partition id IS the numeric `YYYYMM`.
    async fn freeze_partition(&self, month: u32) -> Result<String, ChEnrichError> {
        // Include the database so the server-global shadow namespace does not
        // collide across tenants/tests. Revert-cleanup: `SYSTEM UNFREEZE WITH
        // NAME '<name>'`.
        let name = format!(
            "repair_0114_{}_{}_{month}",
            self.cfg.enrich.database, self.cfg.enrich.table
        );
        let sql = format!(
            "ALTER TABLE {db}.{tbl} FREEZE PARTITION {month} WITH NAME '{name}'",
            db = self.cfg.enrich.database,
            tbl = self.cfg.enrich.table,
        );
        self.client.query(&sql).execute().await?;
        info!(month, backup = %name, table = %self.cfg.enrich.table,
              "coarse repair: partition frozen (server-side snapshot)");
        Ok(name)
    }

    /// Repair every month in the span, one partition at a time. Snapshots first
    /// (if enabled), then runs a partition-bounded one-shot enrichment for the
    /// month. Returns the per-month before/after counts.
    pub async fn run(&self) -> Result<RepairSummary, ChEnrichError> {
        let months = self.months_with_zeros().await?;
        info!(
            count = months.len(),
            table = %self.cfg.enrich.table,
            start = self.cfg.start_month,
            end = self.cfg.end_month,
            "coarse repair: months with enrichable zeros"
        );

        let mut summary = RepairSummary::default();
        for mw in months {
            if self.cfg.dry_run {
                info!(
                    month = mw.month,
                    zeros = mw.zeros,
                    "coarse repair: DRY RUN — would repair"
                );
                summary.months.push(MonthRepair {
                    month: mw.month,
                    zeros_before: mw.zeros,
                    zeros_after: mw.zeros,
                    rows_enriched: 0,
                    snapshot_name: None,
                });
                continue;
            }

            let snapshot_name = if self.cfg.snapshot {
                Some(self.freeze_partition(mw.month).await?)
            } else {
                warn!(
                    month = mw.month,
                    "coarse repair: snapshot DISABLED for this partition"
                );
                None
            };

            // Partition-bounded, one-shot: drain this one month's backlog.
            let mut ecfg = self.cfg.enrich.clone();
            ecfg.time_window = Some((mw.start_ts, mw.end_ts));
            ecfg.one_shot = true;
            let stats = ChEnrichmentPass::with_client(self.client.clone(), ecfg)
                .run()
                .await?;

            info!(
                month = mw.month,
                before = stats.candidates_before,
                after = stats.candidates_after,
                enriched = stats.rows_enriched,
                "coarse repair: month done"
            );
            summary.months.push(MonthRepair {
                month: mw.month,
                zeros_before: stats.candidates_before,
                zeros_after: stats.candidates_after,
                rows_enriched: stats.rows_enriched,
                snapshot_name,
            });
            // Defensive cross-check: the enumeration count and the pass's own
            // pre-count should agree (both are the month's zero population).
            if mw.zeros != stats.candidates_before {
                warn!(
                    month = mw.month,
                    enumerated = mw.zeros,
                    pass_counted = stats.candidates_before,
                    "coarse repair: month zero-count drifted between enumeration and pass \
                     (concurrent writes?) — before/after still reported from the pass"
                );
            }
        }
        Ok(summary)
    }
}
