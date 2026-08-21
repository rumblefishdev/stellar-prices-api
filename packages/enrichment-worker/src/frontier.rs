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
    swept_at: u32,
}

/// What the frontier remembers about one month: its state, and when that was
/// last confirmed. The timestamp is what keeps `exhausted` a *hint* rather than
/// a verdict — see [`stale_exhausted_months`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MonthState {
    state: FrontierState,
    swept_at: u32,
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
    /// Stale `exhausted` months re-confirmed against the data this run.
    pub months_rechecked: u32,
    /// Of those, how many had gained work and were re-opened to `pending`.
    /// Non-zero means something wrote into a partition the sweep had finished —
    /// a backfill, or a newly-priceable reference. Worth an eyebrow, not an
    /// alarm: it is the drift correction doing its job.
    pub months_reopened: u32,
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
    /// How long an `exhausted` mark is trusted before it is re-confirmed
    /// against the data. This is what makes the frontier a hint with an expiry
    /// rather than a permanent verdict — see [`stale_exhausted_months`].
    pub recheck_after_secs: u32,
    /// Exhausted months re-checked per invocation. Small, and separate from
    /// `max_months`, so drift correction can never starve the actual drain.
    pub max_rechecks: u32,
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
    async fn states(&self) -> Result<HashMap<u32, MonthState>, ChEnrichError> {
        let sql = format!(
            "SELECT month, CAST(state AS String) AS state, \
                    toUnixTimestamp(swept_at) AS swept_at \
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
            .filter_map(|r| {
                FrontierState::parse(&r.state).map(|state| {
                    (
                        r.month,
                        MonthState {
                            state,
                            swept_at: r.swept_at,
                        },
                    )
                })
            })
            .collect())
    }

    /// The ClickHouse server clock, as unix seconds.
    ///
    /// `swept_at` is written server-side, so staleness must be judged against
    /// the same clock. Comparing a server-written timestamp to the Lambda's wall
    /// clock would make the re-check cadence drift with host skew.
    async fn server_now(&self) -> Result<u32, ChEnrichError> {
        Ok(self
            .client
            .query("SELECT toUnixTimestamp(now())")
            .fetch_one::<u32>()
            .await?)
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
    states: &HashMap<u32, MonthState>,
) -> Vec<u32> {
    let (lo, hi) = span;
    let mut out = Vec::new();
    let mut m = lo;
    // `hi` bounds the walk as well as `live_start_month`: a table whose newest
    // data predates the live window must not walk forward into empty calendar
    // months forever.
    while m < live_start_month && m <= hi {
        if states.get(&m).map(|s| s.state) != Some(FrontierState::Exhausted) {
            out.push(m);
        }
        match add_months(m, 1) {
            Some(next) => m = next,
            None => break,
        }
    }
    out
}

/// `exhausted` months whose last confirmation is older than `recheck_after_secs`,
/// oldest-confirmation first, capped at `limit`.
///
/// 🔴 **This is what stops `exhausted` becoming a verdict.** A month is marked
/// exhausted because nothing there could be priced *at that moment*. Two things
/// falsify that later: a backfill writes new rows into a historical partition
/// (which is exactly what tasks 0088 and 0201 do), or a new reference/oracle
/// price makes previously unpriceable candles priceable. Without a re-check
/// those rows are never revisited and read as healthy forever — the failure
/// class that cost 26 days in task 0215.
///
/// Re-checking is deliberately cheap: a pass with `max_batches = 0` computes
/// the month's bounded candidate count and does no work, so a month that really
/// is exhausted costs a few sub-second queries and gets re-stamped.
///
/// Oldest-confirmation-first so the rotation is fair — every exhausted month is
/// revisited on a predictable cycle rather than the same few being re-checked
/// while others go stale indefinitely.
fn stale_exhausted_months(
    states: &HashMap<u32, MonthState>,
    now: u32,
    recheck_after_secs: u32,
    limit: u32,
) -> Vec<u32> {
    let mut stale: Vec<(u32, u32)> = states
        .iter()
        .filter(|(_, s)| s.state == FrontierState::Exhausted)
        .filter(|(_, s)| now.saturating_sub(s.swept_at) >= recheck_after_secs)
        .map(|(m, s)| (s.swept_at, *m))
        .collect();
    stale.sort_unstable();
    stale
        .into_iter()
        .take(limit as usize)
        .map(|(_, m)| m)
        .collect()
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

        // Terminal on EITHER condition, and both matter:
        //
        //   * `candidates_after == 0` — the month is fully drained. Marking it
        //     pending would cost a whole extra visit per month just to learn
        //     there is nothing left, doubling the walk over ~102 partitions.
        //   * `rows_enriched == 0` — no progress, so nothing here has a USD
        //     reference of any kind (or the month is empty). This is what lets
        //     the pre-2021 unpriceable floor terminate with no cutoff date.
        //
        // Neither is permanent: `stale_exhausted_months` re-confirms both on a
        // cycle, so a month that later gains work is re-opened.
        let state = if stats.candidates_after == 0 || stats.rows_enriched == 0 {
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

    // Drift correction. Runs after the real work and on the same deadline, so
    // it can only ever use budget the drain did not need.
    //
    // `now` comes from the frontier rows' own clock domain — `swept_at` is
    // written server-side by ClickHouse, so comparing it against the Lambda's
    // wall clock would drift. Reading the server clock keeps one clock domain.
    let now = frontier.server_now().await?;
    for month in stale_exhausted_months(&states, now, cfg.recheck_after_secs, cfg.max_rechecks) {
        if let Some(deadline) = cfg.deadline
            && Instant::now() >= deadline
        {
            summary.deadline_hit = true;
            break;
        }
        let Some(window) = month_bounds(month) else {
            continue;
        };

        // `max_batches = 0` is the whole trick: the pass counts the month's
        // bounded candidates and does no work, because `max_batches` keeps its
        // literal meaning in both modes (never a hidden unbounded drain). So the
        // re-check reuses the pass's own tested counting path rather than
        // hand-rolling a second candidate query that could drift from it.
        let mut ecfg = cfg.base.clone();
        ecfg.time_window = Some(window);
        ecfg.one_shot = false;
        ecfg.max_batches = 0;
        let stats = ChEnrichmentPass::with_client(client.clone(), ecfg)
            .run()
            .await?;

        summary.months_rechecked += 1;
        let still_exhausted = stats.candidates_before == 0;
        if still_exhausted {
            // Re-stamp so `swept_at` advances and the rotation moves on.
            frontier.record(month, FrontierState::Exhausted, 0).await?;
        } else {
            summary.months_reopened += 1;
            warn!(
                month,
                candidates = stats.candidates_before,
                "historical sweep: re-opening an exhausted month — it gained work \
                 since it was last confirmed (a backfill wrote here, or a new \
                 reference made these candles priceable)"
            );
            frontier
                .record(month, FrontierState::Pending, stats.candidates_before)
                .await?;
        }
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frontier rows confirmed "now" (`swept_at = 1_000_000`), so nothing is
    /// stale unless a test says so.
    fn states(pairs: &[(u32, FrontierState)]) -> HashMap<u32, MonthState> {
        pairs
            .iter()
            .map(|(m, state)| {
                (
                    *m,
                    MonthState {
                        state: *state,
                        swept_at: 1_000_000,
                    },
                )
            })
            .collect()
    }

    fn stamped(pairs: &[(u32, FrontierState, u32)]) -> HashMap<u32, MonthState> {
        pairs
            .iter()
            .map(|(m, state, swept_at)| {
                (
                    *m,
                    MonthState {
                        state: *state,
                        swept_at: *swept_at,
                    },
                )
            })
            .collect()
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

    // --- drift correction ---------------------------------------------------

    const DAY: u32 = 86_400;

    #[test]
    fn a_fresh_exhausted_month_is_not_rechecked() {
        let s = stamped(&[(202101, FrontierState::Exhausted, 100 * DAY)]);
        assert!(stale_exhausted_months(&s, 100 * DAY + 60, 7 * DAY, 4).is_empty());
    }

    #[test]
    fn a_stale_exhausted_month_is_rechecked() {
        let s = stamped(&[(202101, FrontierState::Exhausted, 100 * DAY)]);
        assert_eq!(
            stale_exhausted_months(&s, 108 * DAY, 7 * DAY, 4),
            vec![202101]
        );
    }

    #[test]
    fn pending_months_are_never_rechecked() {
        // They are already in the work queue; re-checking them would duplicate
        // the count the pass is about to do anyway.
        let s = stamped(&[(202101, FrontierState::Pending, 0)]);
        assert!(stale_exhausted_months(&s, 999 * DAY, 7 * DAY, 4).is_empty());
    }

    #[test]
    fn rechecks_rotate_oldest_confirmation_first() {
        let s = stamped(&[
            (202103, FrontierState::Exhausted, 30 * DAY),
            (202101, FrontierState::Exhausted, 10 * DAY),
            (202102, FrontierState::Exhausted, 20 * DAY),
        ]);
        // Fair rotation: the least recently confirmed goes first, so no month
        // can go stale indefinitely while others are re-checked repeatedly.
        assert_eq!(
            stale_exhausted_months(&s, 999 * DAY, 7 * DAY, 3),
            vec![202101, 202102, 202103]
        );
    }

    #[test]
    fn rechecks_are_capped_so_they_cannot_starve_the_drain() {
        let all: Vec<(u32, FrontierState, u32)> = (1..=12)
            .map(|m| (202000 + m, FrontierState::Exhausted, m * DAY))
            .collect();
        let picked = stale_exhausted_months(&stamped(&all), 999 * DAY, 7 * DAY, 4);
        assert_eq!(picked.len(), 4);
        assert_eq!(picked, vec![202001, 202002, 202003, 202004]);
    }

    #[test]
    fn frontier_state_round_trips_through_the_enum8_name() {
        for s in [FrontierState::Pending, FrontierState::Exhausted] {
            assert_eq!(FrontierState::parse(s.as_str()), Some(s));
        }
        assert_eq!(FrontierState::parse("nonsense"), None);
    }
}
