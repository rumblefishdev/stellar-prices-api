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
    /// `price_ohlcv_1h`); `time_window` is set per month by the driver and any
    /// pre-set value is overwritten. `one_shot` on this inner config is ignored —
    /// the driver drives it from [`Self::one_shot`] below.
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
    /// Per-month drain mode.
    ///
    /// - `true` — **one-shot**: each month drains its *entire* backlog in the one
    ///   pass, ignoring `enrich.max_batches`. This is the manual historical
    ///   repair (the operator CLI): bounded run time is not a concern and the
    ///   goal is to reach the `no_reference` floor in a single invocation.
    /// - `false` — **bounded**: each month runs at most `enrich.max_batches`
    ///   batches, then stops; any overflow defers to the next run. This is the
    ///   recurring sweep folded into the hourly enrichment Lambda, where an
    ///   unbounded per-month drain could exceed the function timeout. In steady
    ///   state the recent backlog is near-empty, so a bounded pass early-exits
    ///   cheaply.
    pub one_shot: bool,
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

    /// Cold-start health check. When snapshots are enabled this also probes the
    /// `FREEZE` grant, because the alternative is discovering it at the first
    /// write — after the operator has already committed to a long run.
    pub async fn preflight(&self) -> Result<(), ChEnrichError> {
        self.client.query("SELECT 1").execute().await?;
        if self.cfg.snapshot && !self.cfg.dry_run {
            self.warn_if_no_freeze_grant().await;
        }
        Ok(())
    }

    /// Warn — never fail — when the connected user shows no grant that could
    /// cover `ALTER FREEZE PARTITION`. Deliberately advisory: grant text has
    /// several shapes (`ALL`, a bare `ALTER`, the explicit privilege), so a
    /// hard gate here risks refusing a legitimate run over a parsing miss. The
    /// authoritative check remains the statement itself, which now fails with
    /// [`ChEnrichError::FreezeDenied`] and its remedy.
    async fn warn_if_no_freeze_grant(&self) {
        let grants = match self
            .client
            .query("SHOW GRANTS FOR CURRENT_USER")
            .fetch_all::<String>()
            .await
        {
            Ok(rows) => rows.join("; "),
            // Probe failure is not itself a problem — proceed and let the real
            // FREEZE report the truth.
            Err(e) => {
                warn!(error = %e, "coarse repair: could not read grants; skipping FREEZE pre-check");
                return;
            }
        };
        let upper = grants.to_uppercase();
        if !(upper.contains("FREEZE") || upper.contains("GRANT ALL")) {
            warn!(
                grants = %grants,
                "coarse repair: connected user shows no ALTER FREEZE PARTITION grant — \
                 per-partition snapshots will likely fail. Either grant it, or pre-FREEZE \
                 as CH admin and re-run with --skip-snapshot (see the 0114 runbook)."
            );
        }
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
        self.client
            .query(&sql)
            .execute()
            .await
            .map_err(|source| ChEnrichError::FreezeDenied {
                table: self.cfg.enrich.table.clone(),
                month,
                source,
            })?;
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

            // Partition-bounded pass over this one month. `one_shot` decides
            // whether it drains the month fully (manual repair) or a bounded
            // `max_batches` chunk with overflow deferring to the next run (the
            // recurring hourly sweep) — see [`CoarseRepairConfig::one_shot`].
            let mut ecfg = self.cfg.enrich.clone();
            ecfg.time_window = Some((mw.start_ts, mw.end_ts));
            ecfg.one_shot = self.cfg.one_shot;
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

// ---------------------------------------------------------------------------
// Recurring bounded sweep (task 0114 — folded into the hourly enrichment Lambda)
// ---------------------------------------------------------------------------

/// Configuration for the recurring coarse-table sweep folded into the hourly
/// enrichment Lambda (task 0114).
///
/// This is the *going-forward guard* against the rollup path re-freezing
/// un-enriched USD values, distinct from the operator [`CoarseRepairConfig`]
/// historical one-off. Where the historical repair drains a fixed
/// `[start_month, end_month]` fully and FREEZE-snapshots each partition, the
/// sweep:
///
///   * recomputes a **trailing** month window from `now()` every run, so it only
///     ever touches recent partitions — task 0111: partition-bounded, never a
///     full-history scan;
///   * runs **bounded** (`one_shot = false`) so a single run cannot exceed the
///     Lambda timeout — any overflow defers to the next hourly run;
///   * takes **no snapshot** — recent live-era partitions are not the sole copy
///     (1m still holds them) and `prices_writer` cannot FREEZE anyway.
///
/// It reuses the exact enrichment tiers and the additive `INSERT … SELECT` the
/// historical repair proved, so it is non-destructive by the same construction.
#[derive(Debug, Clone)]
pub struct CoarseSweepConfig {
    /// Shared enrichment base (oracle name, forward-fill / pivot windows, batch
    /// size, database). `table`, `time_window`, `one_shot` and `max_batches` are
    /// set per table/month by the sweep; any preset values are overwritten.
    pub base: ChEnrichConfig,
    /// Coarse tables to sweep, e.g. `["price_ohlcv_1h", "price_ohlcv_4h", …]`.
    /// `price_ohlcv_1m` and non-`price_ohlcv_*` names are rejected (skipped with
    /// a warning) — the sweep only ever touches coarse rollups.
    pub tables: Vec<String>,
    /// Months the trailing window covers, **inclusive of the current month**
    /// (clamped to `>= 1`). `2` → current + previous month; covers month-boundary
    /// rollups plus multi-day enrichment lag.
    pub lookback_months: u32,
    /// Per-tier batch budget for each month's bounded pass (the `max_batches` the
    /// driver hands the enrichment pass in `one_shot = false` mode).
    pub max_batches: u32,
}

/// One table's sweep outcome — its name and per-month [`RepairSummary`].
#[derive(Debug, Clone, Serialize)]
pub struct TableSweep {
    pub table: String,
    pub summary: RepairSummary,
}

/// Whole-sweep result: the trailing window actually swept, each table's outcome,
/// and any tables whose pass errored (isolated — one bad table never starves the
/// rest; see [`run_coarse_sweep`]).
#[derive(Debug, Clone, Default, Serialize)]
pub struct CoarseSweepSummary {
    pub start_month: u32,
    pub end_month: u32,
    pub tables: Vec<TableSweep>,
    pub failed_tables: Vec<String>,
}

impl CoarseSweepSummary {
    pub fn total_enriched(&self) -> u64 {
        self.tables.iter().map(|t| t.summary.total_enriched()).sum()
    }
    pub fn total_remaining(&self) -> u64 {
        self.tables
            .iter()
            .map(|t| t.summary.total_remaining())
            .sum()
    }
}

/// A coarse rollup the sweep may touch: `price_ohlcv_*` except the live base
/// table `price_ohlcv_1m`. Mirrors the operator CLI's table guard.
fn is_coarse_table(table: &str) -> bool {
    table.starts_with("price_ohlcv_") && table != "price_ohlcv_1m"
}

/// Resolve the trailing `[start_month, end_month]` window (inclusive `YYYYMM`)
/// from the **ClickHouse server clock**, so the sweep's month boundaries align
/// with the same `now()` the refreshable rollup MVs use. `lookback_months` counts
/// inclusively (1 = current month only), so it subtracts `lookback_months - 1`.
async fn current_month_window(
    client: &Client,
    lookback_months: u32,
) -> Result<(u32, u32), ChEnrichError> {
    #[derive(Row, Deserialize)]
    struct Window {
        start_m: u32,
        end_m: u32,
    }
    let back = lookback_months.max(1) - 1;
    let sql = format!(
        "SELECT toYYYYMM(now() - INTERVAL {back} MONTH) AS start_m, \
                toYYYYMM(now()) AS end_m"
    );
    let w = client.query(&sql).fetch_one::<Window>().await?;
    Ok((w.start_m, w.end_m))
}

/// Run the recurring bounded sweep over the configured coarse tables for the
/// trailing month window. Non-destructive and additive — the same
/// [`CoarseRepairDriver`] as the historical repair, just bounded (`one_shot =
/// false`) and snapshotless.
///
/// Per-table failure isolation: a table whose pass errors is recorded in
/// [`CoarseSweepSummary::failed_tables`] and the sweep continues with the rest,
/// so one unhealthy table cannot starve the others. Only a failure of the window
/// query itself (nothing can proceed without a window) returns `Err`. The Lambda
/// treats the whole call as best-effort regardless.
pub async fn run_coarse_sweep(
    client: &Client,
    cfg: &CoarseSweepConfig,
) -> Result<CoarseSweepSummary, ChEnrichError> {
    let (start_month, end_month) = current_month_window(client, cfg.lookback_months).await?;
    info!(
        start_month,
        end_month,
        tables = cfg.tables.len(),
        max_batches = cfg.max_batches,
        "coarse sweep: starting over trailing window"
    );

    let mut summary = CoarseSweepSummary {
        start_month,
        end_month,
        ..Default::default()
    };

    for table in &cfg.tables {
        if !is_coarse_table(table) {
            warn!(
                table = %table,
                "coarse sweep: refusing non-coarse table (price_ohlcv_1m or non-OHLCV) — skipped"
            );
            summary.failed_tables.push(table.clone());
            continue;
        }

        let mut enrich = cfg.base.clone();
        enrich.table = table.clone();
        enrich.max_batches = cfg.max_batches;
        let repair_cfg = CoarseRepairConfig {
            enrich,
            start_month,
            end_month,
            snapshot: false,
            dry_run: false,
            one_shot: false,
        };

        match CoarseRepairDriver::with_client(client.clone(), repair_cfg)
            .run()
            .await
        {
            Ok(table_summary) => {
                info!(
                    table = %table,
                    enriched = table_summary.total_enriched(),
                    remaining = table_summary.total_remaining(),
                    "coarse sweep: table done"
                );
                summary.tables.push(TableSweep {
                    table: table.clone(),
                    summary: table_summary,
                });
            }
            Err(e) => {
                // Isolated — log and carry on so a single unhealthy table does
                // not block the sweep of the others this run.
                warn!(table = %table, error = %e, "coarse sweep: table failed (continuing)");
                summary.failed_tables.push(table.clone());
            }
        }
    }

    Ok(summary)
}
