//! Frontier-driven historical enrichment sweep (task 0111, phase 2).
//!
//! The scheduled pass is bounded to the newest few monthly partitions
//! ([`crate::live_window`]). That fixes the timeout but leaves the historical
//! backlog — 646 M unenriched rows below `202403` — with nothing draining it.
//! This module is what drains it: one monthly partition per step, remembering
//! where it got to in `prices.enrichment_frontier` so the walk survives across
//! invocations.
//!
//! ## 🔴 The frontier is ADVISORY, never authoritative
//!
//! Every month is re-confirmed against the data before it is worked: the pass's
//! own `candidates_before` is a partition-bounded `count_candidates` (~0.2 s),
//! and it — not the stored row — decides what happens. A wrong or stale
//! frontier row therefore costs one cheap query and can never cause skipped
//! rows.
//!
//! That distinction is the whole design. Skipped rows that read as healthy are
//! exactly the failure class that cost 26 days in task 0215, and an
//! authoritative cursor is how you get them. A performance hint is not.
//!
//! ## Termination, without a hard-coded cutoff
//!
//! A month where the pass enriches nothing is marked `exhausted` and never
//! revisited. That covers both empty calendar months and the ~5.34 M rows that
//! predate the XLM/USDC reference market (its first candle is 2021-02) and are
//! permanently unpriceable by this design. Neither needs a magic date constant —
//! "made no progress" is the signal, and it is measured each time.

use std::collections::HashMap;
use std::time::Instant;

use clickhouse::{Client, Row};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::ch_enrich::{ChEnrichConfig, ChEnrichError, ChEnrichmentPass};
use crate::live_window::{add_months, month_bounds};

/// Stored per-month sweep state. Deliberately only two values: the frontier
/// answers "is there believed to be work here", nothing more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontierState {
    Pending,
    Exhausted,
}

impl FrontierState {
    /// The `Enum8` name as spelled in `init.sql`.
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Exhausted => "exhausted",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "exhausted" => Some(Self::Exhausted),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Row, Deserialize)]
struct StateRow {
    month: u32,
    state: String,
}

#[derive(Debug, Clone, Row, Deserialize)]
struct SpanRow {
    lo: u32,
    hi: u32,
}

/// One month's sweep outcome.
#[derive(Debug, Clone, Serialize)]
pub struct MonthSweep {
    pub month: u32,
    pub zeros_before: u64,
    pub zeros_after: u64,
    pub rows_enriched: u64,
    pub state: &'static str,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SweepSummary {
    pub months: Vec<MonthSweep>,
    /// Months below the live window still believed to hold work, *after* this
    /// run — the drain-progress metric (task 0111 phase 4). Published so drain
    /// progress is a number on a dashboard rather than an archaeology dig.
    pub months_pending: u32,
    /// Oldest month still `pending` — the frontier position itself.
    pub frontier_month: Option<u32>,
    /// Set when the run stopped on its wall-clock budget rather than on
    /// `max_months`; distinguishes "nothing left to do" from "ran out of time".
    pub deadline_hit: bool,
}

impl SweepSummary {
    pub fn total_enriched(&self) -> u64 {
        self.months.iter().map(|m| m.rows_enriched).sum()
    }
}

/// One historical sweep run.
#[derive(Debug, Clone)]
pub struct HistoricalSweepConfig {
    /// Base enrichment config. `time_window` is overwritten per month; anything
    /// else (batch size, batch budget, oracle name) is shared with the live pass.
    pub base: ChEnrichConfig,
    /// First month the *live* pass owns. The sweep only ever touches months
    /// strictly below this, so the two never contend for the same partition.
    pub live_start_month: u32,
    /// Months worked per invocation. Small by design: the sweep runs after the
    /// live pass, on whatever budget is left.
    pub max_months: u32,
    /// Wall-clock stop, checked before each month.
    pub deadline: Option<Instant>,
}

pub struct EnrichmentFrontier {
    client: Client,
    database: String,
    table: String,
}

impl EnrichmentFrontier {
    pub fn new(client: Client, database: String, table: String) -> Self {
        Self {
            client,
            database,
            table,
        }
    }

    /// The `[oldest, newest]` `YYYYMM` partitions the table actually holds.
    ///
    /// Measured cheap, not assumed: `max(timestamp)` on this table reads **294
    /// rows in 0.00 s** because `PARTITION BY toYYYYMM(timestamp)` gives every
    /// part a `minmax_timestamp` index, so ClickHouse answers the aggregate from
    /// part metadata without touching data (task 0111, 2026-08-21). `min` uses
    /// the identical mechanism.
    ///
    /// Deliberately derived from the data rather than `system.parts`: the
    /// `system` database is closed to `prices_writer` on prod and cannot be
    /// granted, so anything reading it would deploy green and fail every real
    /// run.
    async fn span(&self) -> Result<Option<(u32, u32)>, ChEnrichError> {
        let sql = format!(
            "SELECT toYYYYMM(min(timestamp)) AS lo, toYYYYMM(max(timestamp)) AS hi \
             FROM {db}.{tbl}",
            db = self.database,
            tbl = self.table,
        );
        let row = self.client.query(&sql).fetch_one::<SpanRow>().await?;
        // An empty table yields the epoch (197001) on both ends.
        Ok((row.lo >= 190001 && row.hi >= row.lo).then_some((row.lo, row.hi)))
    }

    /// Stored state per month for this table. One scan of a table that holds a
    /// few hundred rows.
    async fn states(&self) -> Result<HashMap<u32, FrontierState>, ChEnrichError> {
        let sql = format!(
            "SELECT month, CAST(state AS String) AS state \
             FROM {db}.enrichment_frontier FINAL \
             WHERE tbl = ?",
            db = self.database,
        );
        let rows = self
            .client
            .query(&sql)
            .bind(&self.table)
            .fetch_all::<StateRow>()
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| FrontierState::parse(&r.state).map(|s| (r.month, s)))
            .collect())
    }

    /// Record a month's outcome.
    ///
    /// `version` is `toUnixTimestamp64Milli(now64(3))` evaluated **server-side**,
    /// so every writer reads one clock and concurrent invocations cannot tie or
    /// disagree by host skew. `ReplacingMergeTree(version)` keeps the highest,
    /// which makes the row monotonic-forward exactly like `ingest_cursor` — an
    /// operator rewind needs a `DELETE`, not a lower `INSERT`. Since the frontier
    /// is advisory, a lost race costs duplicated work, never lost work.
    async fn record(
        &self,
        month: u32,
        state: FrontierState,
        zeros_seen: u64,
    ) -> Result<(), ChEnrichError> {
        let sql = format!(
            "INSERT INTO {db}.enrichment_frontier \
                 (tbl, month, state, zeros_seen, swept_at, version) \
             SELECT ?, ?, '{state}', ?, now(), toUnixTimestamp64Milli(now64(3))",
            db = self.database,
            state = state.as_str(),
        );
        self.client
            .query(&sql)
            .bind(&self.table)
            .bind(month)
            .bind(zeros_seen)
            .execute()
            .await?;
        Ok(())
    }
}

/// Months below `live_start_month` that are not yet `exhausted`, oldest first.
///
/// Oldest-first because that is where the backlog is (93.7% of unenriched rows
/// sit below `202403`) and because a month, once exhausted, is never revisited —
/// so the walk converges from the bottom rather than re-treading the top.
///
/// Pure, so the walk order is testable without ClickHouse.
fn months_to_sweep(
    span: (u32, u32),
    live_start_month: u32,
    states: &HashMap<u32, FrontierState>,
) -> Vec<u32> {
    let (lo, hi) = span;
    let mut out = Vec::new();
    let mut m = lo;
    // `hi` bounds the walk as well as `live_start_month`: a table whose newest
    // data predates the live window must not walk forward into empty calendar
    // months forever.
    while m < live_start_month && m <= hi {
        if states.get(&m) != Some(&FrontierState::Exhausted) {
            out.push(m);
        }
        match add_months(m, 1) {
            Some(next) => m = next,
            None => break,
        }
    }
    out
}

/// Walk the frontier, working up to `max_months` monthly partitions.
pub async fn run_historical_sweep(
    client: &Client,
    cfg: &HistoricalSweepConfig,
) -> Result<SweepSummary, ChEnrichError> {
    let frontier = EnrichmentFrontier::new(
        client.clone(),
        cfg.base.database.clone(),
        cfg.base.table.clone(),
    );

    let Some(span) = frontier.span().await? else {
        info!(table = %cfg.base.table, "historical sweep: table is empty — nothing to do");
        return Ok(SweepSummary::default());
    };
    let states = frontier.states().await?;
    let queue = months_to_sweep(span, cfg.live_start_month, &states);

    let mut summary = SweepSummary {
        months_pending: queue.len() as u32,
        frontier_month: queue.first().copied(),
        ..Default::default()
    };
    info!(
        table = %cfg.base.table,
        span_lo = span.0,
        span_hi = span.1,
        live_start_month = cfg.live_start_month,
        pending = summary.months_pending,
        frontier = summary.frontier_month,
        "historical sweep: frontier read"
    );

    for month in queue.into_iter().take(cfg.max_months as usize) {
        if let Some(deadline) = cfg.deadline
            && Instant::now() >= deadline
        {
            summary.deadline_hit = true;
            info!(
                next_month = month,
                "historical sweep: time budget reached — deferring to the next run"
            );
            break;
        }

        let Some(window) = month_bounds(month) else {
            warn!(
                month,
                "historical sweep: unparseable partition id — skipping"
            );
            continue;
        };

        // The pass's own `candidates_before` IS the partition-bounded
        // re-confirmation: a month with nothing to do costs exactly one cheap
        // count and the tiers are skipped. That is why no separate confirm query
        // exists — the authoritative read and the work share one statement, so
        // they cannot disagree.
        let mut ecfg = cfg.base.clone();
        ecfg.time_window = Some(window);
        ecfg.one_shot = false;
        let stats = ChEnrichmentPass::with_client(client.clone(), ecfg)
            .run()
            .await?;

        // No progress ⇒ nothing here has a USD reference of any kind (or the
        // month is empty). Terminal, and the reason the sweep needs no cutoff
        // date for the pre-2021 unpriceable floor.
        let state = if stats.rows_enriched == 0 {
            FrontierState::Exhausted
        } else {
            FrontierState::Pending
        };
        frontier
            .record(month, state, stats.candidates_after)
            .await?;

        info!(
            month,
            before = stats.candidates_before,
            after = stats.candidates_after,
            enriched = stats.rows_enriched,
            state = state.as_str(),
            "historical sweep: month done"
        );
        summary.months.push(MonthSweep {
            month,
            zeros_before: stats.candidates_before,
            zeros_after: stats.candidates_after,
            rows_enriched: stats.rows_enriched,
            state: state.as_str(),
        });
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states(pairs: &[(u32, FrontierState)]) -> HashMap<u32, FrontierState> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn an_empty_frontier_sweeps_every_month_below_the_live_window() {
        let q = months_to_sweep((202601, 202608), 202607, &states(&[]));
        assert_eq!(q, vec![202601, 202602, 202603, 202604, 202605, 202606]);
    }

    #[test]
    fn the_live_window_is_never_swept() {
        let q = months_to_sweep((202601, 202608), 202607, &states(&[]));
        assert!(!q.contains(&202607), "the live pass owns 202607");
        assert!(!q.contains(&202608), "the live pass owns 202608");
    }

    #[test]
    fn exhausted_months_are_never_revisited() {
        let s = states(&[
            (202601, FrontierState::Exhausted),
            (202602, FrontierState::Exhausted),
            (202603, FrontierState::Pending),
        ]);
        assert_eq!(
            months_to_sweep((202601, 202608), 202607, &s),
            vec![202603, 202604, 202605, 202606]
        );
    }

    #[test]
    fn the_walk_is_oldest_first() {
        let q = months_to_sweep((202411, 202602), 202602, &states(&[]));
        assert_eq!(q.first(), Some(&202411));
        assert!(q.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn the_walk_crosses_year_boundaries() {
        let q = months_to_sweep((202511, 202602), 202602, &states(&[]));
        assert_eq!(q, vec![202511, 202512, 202601]);
    }

    #[test]
    fn a_fully_exhausted_history_yields_an_empty_queue() {
        let s = states(&[
            (202601, FrontierState::Exhausted),
            (202602, FrontierState::Exhausted),
        ]);
        assert!(months_to_sweep((202601, 202603), 202603, &s).is_empty());
    }

    /// The pre-2021 floor: months that predate the XLM/USDC reference market
    /// enrich nothing, get marked `exhausted` on first visit, and drop out —
    /// with no hard-coded cutoff date anywhere in the code.
    #[test]
    fn the_unpriceable_floor_falls_out_once_marked() {
        let floor: Vec<(u32, FrontierState)> = (1..=12)
            .map(|m| (202000 + m, FrontierState::Exhausted))
            .collect();
        let q = months_to_sweep((202001, 202103), 202103, &states(&floor));
        assert_eq!(q, vec![202101, 202102]);
    }

    #[test]
    fn a_table_whose_data_ends_before_the_live_window_stops_at_its_own_span() {
        // Guards the walk against running forward through empty calendar months
        // to reach a live window the table has no data anywhere near.
        let q = months_to_sweep((202401, 202403), 202608, &states(&[]));
        assert_eq!(q, vec![202401, 202402, 202403]);
    }

    #[test]
    fn frontier_state_round_trips_through_the_enum8_name() {
        for s in [FrontierState::Pending, FrontierState::Exhausted] {
            assert_eq!(FrontierState::parse(s.as_str()), Some(s));
        }
        assert_eq!(FrontierState::parse("nonsense"), None);
    }
}
