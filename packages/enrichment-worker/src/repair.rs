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

use std::time::Instant;

use clickhouse::{Client, Row};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::ch_enrich::{ChEnrichConfig, ChEnrichError, ChEnrichmentPass, repair_target_pred};

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
    /// Optional wall-clock stop time. When set, [`CoarseRepairDriver::run`]
    /// checks it before each month and stops cleanly once reached, returning what
    /// it finished; the unreached months defer to the next run. `None` = no
    /// deadline (the manual CLI historical repair, which must drain in full). The
    /// recurring sweep sets this so a slow catch-up can never run past the Lambda
    /// timeout — a hard timeout is not a Rust `Err` and would otherwise escape the
    /// best-effort handler and fail the invocation.
    pub deadline: Option<Instant>,
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
    /// Rows a [`ChEnrichConfig::usd_reset`] re-opened this month (task 0182); 0
    /// on every ordinary repair.
    ///
    /// Carried up here, not just logged, because it is half of the check the
    /// runbook tells the operator to perform: `rows_reset` ≫ `rows_enriched`
    /// means the reset discarded values it could not recompute, and the remedy
    /// is `ATTACH PARTITION` from the snapshot. A number the operator is told to
    /// compare has to appear in the thing they are told to read.
    pub rows_reset: u64,
    /// The `FREEZE … WITH NAME` label, when `snapshot` was set.
    pub snapshot_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RepairSummary {
    pub months: Vec<MonthRepair>,
    /// This table's month walk stopped early on the wall-clock budget.
    ///
    /// Without it a truncation *inside* a table is silent: the caller sees a
    /// table that simply had fewer months, which reads identical to one that
    /// finished. That was the residual AC 4 blind spot in task 0218 — the
    /// per-table flag closes it, the per-sweep flag alone did not.
    pub deadline_hit: bool,
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
    /// Rows re-opened by a USD reset across all months (task 0182). Compare
    /// against [`Self::total_enriched`]: a large gap means values were discarded
    /// and not recomputed.
    pub fn total_reset(&self) -> u64 {
        self.months.iter().map(|m| m.rows_reset).sum()
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
             WHERE ({pred}) \
               AND toYYYYMM(timestamp) BETWEEN {start} AND {end} \
             GROUP BY month ORDER BY month",
            db = self.cfg.enrich.database,
            tbl = self.cfg.enrich.table,
            pred = repair_target_pred(self.cfg.enrich.usd_reset.as_ref()),
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
            // Wall-clock budget (recurring sweep): stop cleanly before the Lambda
            // timeout would hard-kill the process. The unreached months defer to
            // the next run. No-op for the CLI (deadline = None).
            if let Some(deadline) = self.cfg.deadline
                && Instant::now() >= deadline
            {
                info!(
                    table = %self.cfg.enrich.table,
                    next_month = mw.month,
                    "coarse repair: time budget reached — deferring remaining months to the next run"
                );
                summary.deadline_hit = true;
                break;
            }

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
                    rows_reset: 0,
                    snapshot_name: None,
                });
                continue;
            }

            let snapshot_name = if self.cfg.snapshot {
                Some(self.freeze_partition(mw.month).await?)
            } else {
                // debug!, not warn!: the recurring sweep runs snapshotless by
                // design (recent partitions are not the sole copy), so this fires
                // every month every run — a warn here would bury real warnings.
                // The operator CLI's own top-level `--skip-snapshot` warning
                // (coarse-repair.rs) covers the deliberate manual case.
                debug!(
                    month = mw.month,
                    "coarse repair: snapshot disabled for this partition"
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
                reset = stats.rows_reset,
                "coarse repair: month done"
            );
            summary.months.push(MonthRepair {
                month: mw.month,
                zeros_before: stats.candidates_before,
                zeros_after: stats.candidates_after,
                rows_enriched: stats.rows_enriched,
                rows_reset: stats.rows_reset,
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

/// Build a [`CoarseSweepConfig`] from the environment, or `None` when the sweep
/// is off.
///
/// `COARSE_SWEEP_TABLES` (comma-separated) is the **on switch**: empty or unset
/// disables the sweep entirely, so the code ships inert and the CDK env turns it
/// on with no code change.
///
/// The table list is validated **once here**, not per run: a typo, or the
/// off-limits live base `price_ohlcv_1m`, is dropped with a loud warning at cold
/// start. Rejecting it per-invocation instead would permanently mark every run
/// as having a skipped table, which is a standing false alarm rather than a
/// signal (task 0218).
///
/// Shared by the enrichment Lambda and the standalone coarse-sweep Lambda so the
/// two can never disagree about which tables are in scope — the failure mode the
/// reuse-over-reimplementation rule exists to prevent.
pub fn sweep_config_from_env(base: ChEnrichConfig) -> Option<CoarseSweepConfig> {
    let raw = prices_clickhouse::env::env_or("COARSE_SWEEP_TABLES", "");
    let tables = coarse_tables_from_list(&raw);
    if tables.is_empty() {
        return None;
    }
    Some(CoarseSweepConfig {
        base,
        tables,
        lookback_months: prices_clickhouse::env::env_parse_or("COARSE_SWEEP_LOOKBACK_MONTHS", 2),
        max_batches: prices_clickhouse::env::env_parse_or("COARSE_SWEEP_MAX_BATCHES", 20),
    })
}

/// Split a comma-separated table list, keeping only coarse rollups.
///
/// Pure and separately testable: the env plumbing above is the only untestable
/// part, and this is where the actual guard lives.
pub fn coarse_tables_from_list(raw: &str) -> Vec<String> {
    let mut tables = Vec::new();
    for name in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if is_coarse_table(name) {
            tables.push(name.to_string());
        } else {
            warn!(
                table = %name,
                "coarse sweep: ignoring non-coarse table in COARSE_SWEEP_TABLES \
                 (expected price_ohlcv_15m … _1M; price_ohlcv_1m is the live base and off-limits)"
            );
        }
    }
    tables
}

/// One table's sweep outcome — its name and per-month [`RepairSummary`].
#[derive(Debug, Clone, Serialize)]
pub struct TableSweep {
    pub table: String,
    pub summary: RepairSummary,
}

/// Whole-sweep result: the trailing window actually swept, each table's outcome,
/// and the two disjoint problem buckets — kept separate so a benign static
/// mis-config cannot masquerade as a runtime failure on the alarm series.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CoarseSweepSummary {
    pub start_month: u32,
    pub end_month: u32,
    pub tables: Vec<TableSweep>,
    /// Tables whose enrichment pass **errored** this run (isolated — one bad
    /// table never starves the rest). This is the true dead-sweep signal.
    pub failed_tables: Vec<String>,
    /// Tables **refused** because they are not coarse rollups (`price_ohlcv_1m`
    /// or a non-`price_ohlcv_*` name left in the config). A static, benign
    /// condition — NOT a runtime failure, so it is reported on its own series and
    /// never counted as a `failed_table` (which would false-fire the alarm every
    /// run for a config typo).
    pub skipped_tables: Vec<String>,
    /// The run stopped early because its wall-clock budget ran out, leaving
    /// [`Self::deferred_tables`] untouched this invocation.
    ///
    /// Recorded because a starved run and a short run are otherwise
    /// indistinguishable from outside: both simply report fewer swept tables.
    /// Task 0218 AC 4 requires a starved run to be *visible*, and this is the
    /// field the `CoarseSweepDeadlineHit` metric is built from.
    pub deadline_hit: bool,
    /// Tables not reached this run because the budget ran out.
    ///
    /// ⚠️ A table that was *started* and truncated mid-walk is NOT listed here —
    /// it appears in [`Self::tables`] with its own `deadline_hit` set. So this
    /// list is "never started", not "not finished"; use [`Self::deadline_hit`],
    /// which covers both, to answer "was this run starved".
    pub deferred_tables: Vec<String>,
}

impl CoarseSweepSummary {
    pub fn total_enriched(&self) -> u64 {
        self.tables.iter().map(|t| t.summary.total_enriched()).sum()
    }
    /// Whether this run was starved, by any of the three routes: it ran out of
    /// budget before a later table, during the final table, or inside any
    /// table's month walk. The metric `CoarseSweepDeadlineHit` is built from
    /// this, so all three are visible (task 0218 AC 4).
    pub fn was_starved(&self) -> bool {
        self.deadline_hit || self.tables.iter().any(|t| t.summary.deadline_hit)
    }
    pub fn total_remaining(&self) -> u64 {
        self.tables
            .iter()
            .map(|t| t.summary.total_remaining())
            .sum()
    }
}

/// A coarse rollup the sweep may touch: `price_ohlcv_*` except the live base
/// table `price_ohlcv_1m`. Mirrors the operator CLI's table guard. Public so the
/// Lambda entrypoint can drop mis-configured names at cold start (a loud,
/// once-per-container signal) rather than every hourly run.
pub fn is_coarse_table(table: &str) -> bool {
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
/// `deadline` is an optional wall-clock stop time: the sweep checks it before
/// each table (and the driver before each month), so a slow catch-up run stops
/// cleanly and defers the rest rather than being hard-killed by the Lambda
/// timeout — which, being an invocation-level error rather than a Rust `Err`,
/// would escape the caller's best-effort handling. `None` = no limit.
///
/// Two disjoint problem buckets: a table whose pass **errors** goes to
/// [`CoarseSweepSummary::failed_tables`] (the dead-sweep signal), while a
/// non-coarse table left in the config goes to
/// [`CoarseSweepSummary::skipped_tables`] (benign, never alarmed). Either way the
/// sweep continues, so one bad table never starves the rest. Only a failure of
/// the window query itself returns `Err`; the Lambda treats the call as
/// best-effort regardless.
pub async fn run_coarse_sweep(
    client: &Client,
    cfg: &CoarseSweepConfig,
    deadline: Option<Instant>,
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
        // Wall-clock budget: stop before the Lambda timeout would hard-kill us.
        if let Some(deadline) = deadline
            && Instant::now() >= deadline
        {
            info!(
                next_table = %table,
                "coarse sweep: time budget reached — deferring remaining tables to the next run"
            );
            // Record it. Without this a starved run looks identical to a short
            // one from outside — the blind spot task 0218 AC 4 closes.
            summary.deadline_hit = true;
            summary.deferred_tables = cfg
                .tables
                .iter()
                .skip_while(|t| *t != table)
                .cloned()
                .collect();
            break;
        }

        if !is_coarse_table(table) {
            warn!(
                table = %table,
                "coarse sweep: refusing non-coarse table (price_ohlcv_1m or non-OHLCV) — skipped"
            );
            summary.skipped_tables.push(table.clone());
            continue;
        }

        let mut enrich = cfg.base.clone();
        enrich.table = table.clone();
        enrich.max_batches = cfg.max_batches;
        // Task 0182: the reset discards computed values and is not a fixed point
        // across runs (see `repair_target_pred`), so an hourly sweep carrying one
        // would re-zero and recompute the same rows every hour, forever. Pinned
        // here rather than merely left unset, so a reset can never reach this
        // path by inheriting from `cfg.base`.
        enrich.usd_reset = None;
        let repair_cfg = CoarseRepairConfig {
            enrich,
            start_month,
            end_month,
            snapshot: false,
            dry_run: false,
            one_shot: false,
            deadline,
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

#[cfg(test)]
mod sweep_config_tests {
    use super::*;

    /// The live base table is the one name that must never reach the sweep: it
    /// is enrichment's own target, and a sweep writing it would make two
    /// schedules write the same rows. Guarded in `is_coarse_table`; asserted
    /// here because the guard is what keeps the two Lambdas' write sets disjoint
    /// after the task 0218 split.
    #[test]
    fn the_live_base_table_is_refused() {
        assert!(!is_coarse_table("price_ohlcv_1m"));
        assert!(coarse_tables_from_list("price_ohlcv_1m").is_empty());
    }

    #[test]
    fn every_coarse_rollup_is_accepted() {
        for t in [
            "price_ohlcv_15m",
            "price_ohlcv_1h",
            "price_ohlcv_4h",
            "price_ohlcv_1d",
            "price_ohlcv_1w",
            "price_ohlcv_1M",
        ] {
            assert!(is_coarse_table(t), "{t} should be sweepable");
        }
        assert_eq!(
            coarse_tables_from_list(
                "price_ohlcv_15m,price_ohlcv_1h,price_ohlcv_4h,price_ohlcv_1d,price_ohlcv_1w,price_ohlcv_1M"
            )
            .len(),
            6
        );
    }

    /// A non-OHLCV name (a typo, or a state table someone pasted in) is dropped
    /// rather than swept — the sweep must never touch anything outside the
    /// rollup family.
    #[test]
    fn non_ohlcv_names_are_dropped() {
        assert!(!is_coarse_table("assets"));
        assert!(!is_coarse_table("enrichment_frontier"));
        assert!(coarse_tables_from_list("assets,enrichment_frontier,usd_rate").is_empty());
    }

    /// Mixed input keeps only the valid coarse names and drops the rest, in
    /// order — a single bad entry must not discard the whole list.
    #[test]
    fn a_bad_entry_does_not_discard_the_good_ones() {
        assert_eq!(
            coarse_tables_from_list("price_ohlcv_1h, price_ohlcv_1m, oops, price_ohlcv_1d"),
            vec!["price_ohlcv_1h".to_string(), "price_ohlcv_1d".to_string()],
        );
    }

    /// Whitespace and empty segments are tolerated: the value is hand-written in
    /// the CDK env, so a trailing comma or a space after one must not silently
    /// produce an empty-named table.
    #[test]
    fn whitespace_and_empty_segments_are_ignored() {
        assert_eq!(
            coarse_tables_from_list("  price_ohlcv_1h , , price_ohlcv_4h ,"),
            vec!["price_ohlcv_1h".to_string(), "price_ohlcv_4h".to_string()],
        );
        assert!(coarse_tables_from_list("").is_empty());
        assert!(coarse_tables_from_list(",,,").is_empty());
    }
}
