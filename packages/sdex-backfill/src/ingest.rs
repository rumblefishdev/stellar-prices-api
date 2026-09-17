use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use prices_ingest_core::{
    AssetRegistry, CandleAccumulator, OhlcvCandle, Registries, TradeTick, UnresolvedPoolSwap,
    extract_trades, ledger_sequence, offer_lookup_counts, process_ledger, raw_trade_to_tick,
};

use crate::error::BackfillError;
use crate::partition::Partition;
use crate::sink::{OracleSample, Sink};

const ORACLE_FLUSH_THRESHOLD: usize = 50_000;

/// Soroban activation ledger on pubnet (Protocol 20, 2024-02-20). Splits the
/// `SdexOnly` pre-Soroban tail `[1, activation)` from the `Combined` Soroban
/// era `[activation, tip]`. Authoritative value from the BE team
/// (`crates/backfill-runner/README.md` "Start ledger"); see
/// `lore/3-wiki/project/stellar-pubnet-ledger-archive.md`.
pub const SOROBAN_ACTIVATION_LEDGER: u32 = 50_457_424;

/// The `source` string classic SDEX candles are written under. Everything else
/// this backfill writes is an AMM venue (`soroswap`, `phoenix`, `aquarius`).
pub const SDEX_SOURCE: &str = "sdex";

#[derive(Debug, Clone, Default)]
pub struct PartitionStats {
    pub indexed: usize,
    pub skipped: usize,
    pub trade_ticks: usize,
    pub amm_ticks: usize,
    pub oracle_rows: usize,
    pub candles_written: usize,
    pub total_bytes: u64,
    pub wall_clock: Duration,
    /// Oldest / newest candle `minute_start` (unix seconds) landed this
    /// partition, **per stream**, or `None` if that stream wrote none. Merged up
    /// into each stream's `earliest_data_available` / `newest_data_available` —
    /// the covered time-window (§4.5 + task 0053).
    ///
    /// 🔴 **Split by stream deliberately (task 0264).** A single mixed window
    /// used to be stamped onto BOTH `backfill_progress` rows. In `Combined`
    /// mode the run lands SDEX and AMM candles from one parse, and SDEX
    /// predates AMM in every Soroban-era window, so `soroban_amm` inherited the
    /// earliest *SDEX* minute: production carried
    /// `soroban_amm.earliest_data_available = 2024-02-20 17:00`, the activation
    /// boundary, while the first real AMM candle is 2024-03-08 19:00 — 17 days
    /// of coverage claimed at no granularity. The value was a true observation
    /// of the wrong population.
    ///
    /// ⚠️ `sink.rs` merges these with `merge_min`, which only ever moves the
    /// stored value *older*. A window that is wrong-too-early is therefore
    /// permanent until something rewrites the row deliberately — fixing the
    /// writer does not repair the stored value.
    pub sdex_earliest: Option<u32>,
    pub sdex_latest: Option<u32>,
    pub amm_earliest: Option<u32>,
    pub amm_latest: Option<u32>,
    /// Per-ledger records of swaps dropped for an unregistered pool. Aggregated
    /// and re-checked against the final registry at run end (see `run.rs`).
    pub unresolved: Vec<UnresolvedPoolSwap>,
}

impl PartitionStats {
    /// Record a batch of just-written candles: bump the count and widen the
    /// [earliest, latest] minute window **of the stream that wrote them**.
    /// Single seam so every write site keeps the count and the window in sync.
    ///
    /// `source` is the same string handed to `Sink::write_candles`, so the
    /// classification cannot drift from what actually landed in the `source`
    /// column. `OhlcvCandle` carries no source of its own — the call site is
    /// the only place that knows, which is why this takes it as an argument
    /// (task 0264).
    pub(crate) fn note_candles(&mut self, candles: &[OhlcvCandle], source: &str) {
        self.candles_written += candles.len();
        let (earliest, latest) = if source == SDEX_SOURCE {
            (&mut self.sdex_earliest, &mut self.sdex_latest)
        } else {
            (&mut self.amm_earliest, &mut self.amm_latest)
        };
        if let Some(lo) = candles.iter().map(|c| c.minute_start).min() {
            *earliest = Some(earliest.map_or(lo, |cur| cur.min(lo)));
        }
        if let Some(hi) = candles.iter().map(|c| c.minute_start).max() {
            *latest = Some(latest.map_or(hi, |cur| cur.max(hi)));
        }
    }
}

/// The candle accumulators of a WHOLE RUN, not of one partition (task 0286,
/// review A BL-01 / D F1).
///
/// ⚠️ They used to be partition-local, so the minute straddling every
/// 64k-ledger partition boundary was written TWICE: once from the fills of the
/// partition that ended, once from the fills of the one that followed.
/// `price_ohlcv_1m` is a `ReplacingMergeTree(version)` and the second write
/// carries the later ledger's version, so it REPLACES the first rather than
/// merging with it. Under ADR 0287 that is no longer a lost half of a volume —
/// if the tail half of the minute holds only dust the replacement row is
/// `open = high = low = close = 0, pf_trade_count = 0`, and a correctly priced
/// minute is erased. Phase 3 re-ingests ~64 M ledgers, i.e. ~1 000 such
/// boundaries.
///
/// `events-backfill` already keeps its open minute across chunk boundaries
/// (`run.rs`); this is the same shape for the partitioned path: only CLOSED
/// minutes are written as ledgers advance, and the open one is drained once,
/// at the end of the run.
#[derive(Default)]
pub struct RunAccumulators {
    sdex: CandleAccumulator,
    /// One accumulator per AMM venue source (phoenix / soroswap / aquarius).
    amm: HashMap<&'static str, CandleAccumulator>,
}

impl RunAccumulators {
    pub fn new() -> Self {
        Self::default()
    }

    fn merge_sdex(&mut self, tick: &TradeTick) {
        self.sdex.merge(tick);
    }

    fn merge_amm(&mut self, source: &'static str, tick: &TradeTick) {
        self.amm.entry(source).or_default().merge(tick);
    }

    /// Every minute strictly older than `current_minute`, per source — the
    /// minutes no later ledger of this run can add a fill to. The open minute
    /// stays behind, partition boundary or not.
    fn drain_closed(&mut self, current_minute: u32) -> Vec<(&'static str, Vec<OhlcvCandle>)> {
        let mut out = vec![(SDEX_SOURCE, self.sdex.flush_older_than(current_minute))];
        for (source, acc) in self.amm.iter_mut() {
            out.push((*source, acc.flush_older_than(current_minute)));
        }
        out.retain(|(_, candles)| !candles.is_empty());
        out
    }

    /// Everything still open, per source. The ONLY caller that may lose
    /// nothing by it is the end of the run.
    fn drain_all(&mut self) -> Vec<(&'static str, Vec<OhlcvCandle>)> {
        let mut out = vec![(SDEX_SOURCE, self.sdex.flush_all())];
        for (source, acc) in self.amm.iter_mut() {
            out.push((*source, acc.flush_all()));
        }
        out.retain(|(_, candles)| !candles.is_empty());
        out
    }
}

/// What a partition's last in-range ledger means for the minute still open.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PartitionEnd {
    /// Another partition follows: the open minute is carried into it, because
    /// its remaining fills are in the next partition's first ledgers.
    Carry,
    /// The run ends here: nothing can be carried, so everything is written.
    Drain,
}

/// The candles a partition boundary must write — nothing while the run
/// continues, everything still open when it ends.
///
/// ⚠️ This is the whole of review A BL-01's fix and it is deliberately a named
/// function rather than an `if` inside [`index_partition`]: "a partition
/// boundary is not a flush point" is a claim about behaviour, and a claim about
/// behaviour needs somewhere a test can stand.
fn candles_at_partition_end(
    accs: &mut RunAccumulators,
    end: PartitionEnd,
) -> Vec<(&'static str, Vec<OhlcvCandle>)> {
    match end {
        PartitionEnd::Carry => Vec::new(),
        PartitionEnd::Drain => accs.drain_all(),
    }
}

/// Write everything the run still holds open. Called once, after the last
/// partition — including when that partition was skipped (S3-incomplete) and
/// therefore never reached [`PartitionEnd::Drain`]. A no-op when the
/// accumulators are already empty.
pub async fn flush_open_minutes(
    accs: &mut RunAccumulators,
    sink: &Sink,
    totals: &mut PartitionStats,
) -> Result<(), BackfillError> {
    for (source, candles) in accs.drain_all() {
        sink.write_candles(&candles, source).await?;
        totals.note_candles(&candles, source);
    }
    Ok(())
}

/// What to extract from each ledger.
///
/// The backfill runs as two disjoint range invocations of the same engine so
/// every ledger is downloaded exactly once (download is the bottleneck, not
/// parsing): `Combined` over the Soroban era `[activation, tip]` and
/// `SdexOnly` over the pre-Soroban tail `[1, activation)`, where no Soroban
/// AMM pools can exist.
#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum ExtractMode {
    /// SDEX trades + Soroban AMM swaps + oracle samples, from one parse.
    Combined,
    /// Classic SDEX trades only.
    SdexOnly,
}

#[allow(clippy::too_many_arguments)]
pub async fn index_partition(
    partition: &Partition,
    temp_dir: &Path,
    sink: &Sink,
    range_start: u32,
    range_end: u32,
    completed: &HashSet<u32>,
    registry: &mut AssetRegistry,
    reg: &mut Registries,
    mode: ExtractMode,
    accs: &mut RunAccumulators,
    end: PartitionEnd,
) -> Result<PartitionStats, BackfillError> {
    let (first, last) = partition.clamped(range_start, range_end);
    info!(
        partition = partition.start,
        first, last, "partition indexing started"
    );

    let wall_start = Instant::now();
    // Task 0286 phase 2 (S4): the share of order-book fills priced from the
    // offer they crossed rather than from their own amounts. An era whose metas
    // carry no `State` pre-image falls back wholesale and reproduces the dust
    // pricing 0286 exists to fix — invisible to every volume and trade-count
    // reconciliation, because only OHLC is wrong.
    let offer_lookups_before = offer_lookup_counts();
    let mut stats = PartitionStats::default();
    // ⚠️ The candle accumulators are the RUN's, not this partition's
    // ([`RunAccumulators`]): the minute straddling the boundary is carried into
    // the next partition instead of being written twice.
    let mut oracle_buf: Vec<OracleSample> = Vec::new();
    let mut ledgers_in_partition: Vec<u32> = Vec::new();

    for seq in first..=last {
        if completed.contains(&seq) {
            stats.skipped += 1;
            continue;
        }

        let path = partition.local_ledger_path(seq, temp_dir);
        if !path.exists() {
            // Archive tail-lag: a few ledgers in the partition may not be
            // published yet. Skip rather than abort the whole run.
            warn!(
                partition = partition.start,
                seq, "ledger file absent in archive — skipping"
            );
            stats.skipped += 1;
            continue;
        }

        let compressed = tokio::fs::read(&path).await?;
        stats.total_bytes += compressed.len() as u64;

        let xdr_bytes = xdr_parser::decompress_zstd(&compressed)?;
        let batch = xdr_parser::deserialize_batch(&xdr_bytes)?;

        for lcm in batch.ledger_close_metas.iter() {
            // SDEX trades from operation results.
            let trades = extract_trades(lcm);
            for trade in &trades {
                let tick = raw_trade_to_tick(trade, registry);
                accs.merge_sdex(&tick);
                stats.trade_ticks += 1;
            }

            // Soroban events from the same ledger: AMM candles + oracle samples.
            // Only in Combined mode — pre-Soroban ledgers carry no Soroban
            // events, so SdexOnly skips the decode entirely.
            if mode == ExtractMode::Combined {
                let sob = process_ledger(lcm, reg, registry);
                for (source, tick) in &sob.amm_ticks {
                    accs.merge_amm(source, tick);
                    stats.amm_ticks += 1;
                }
                if !sob.oracle.is_empty() {
                    stats.oracle_rows += sob.oracle.len();
                    oracle_buf.extend(sob.oracle);
                }
                // Swaps decoded for pools not (yet) in the registry — carried up
                // for the run-end re-check rather than silently dropped.
                stats.unresolved.extend(sob.unresolved);
            }

            let current_minute = ledger_minute(lcm);
            for (source, candles) in accs.drain_closed(current_minute) {
                sink.write_candles(&candles, source).await?;
                stats.note_candles(&candles, source);
            }
            if oracle_buf.len() >= ORACLE_FLUSH_THRESHOLD {
                sink.write_oracle(&oracle_buf).await?;
                oracle_buf.clear();
            }
        }

        ledgers_in_partition.push(seq);
        stats.indexed += 1;
    }

    for (source, candles) in candles_at_partition_end(accs, end) {
        sink.write_candles(&candles, source).await?;
        stats.note_candles(&candles, source);
    }
    if !oracle_buf.is_empty() {
        sink.write_oracle(&oracle_buf).await?;
    }

    // ⚠️ The resume markers are written for the WHOLE partition, including the
    // ledgers whose minute is still open and therefore not yet in ClickHouse
    // (task 0286 S5). A crash between here and the run's final
    // [`flush_open_minutes`] leaves those fills unwritten while a re-run skips
    // their ledgers — the boundary minute then reads short rather than wrong.
    // The trade is deliberate: the alternative, writing the partial minute here
    // as the code did before, REPLACES a priced row with a dust-only one under
    // `ReplacingMergeTree`, which is silent and unrecoverable without a
    // re-ingest. Recovery for the crash case is documented as a hard gate in
    // `docs/runbooks/0286-candle-definitions-rollout.md`.
    sink.write_completed_ledgers(&ledgers_in_partition).await?;

    stats.wall_clock = wall_start.elapsed();
    let offer_lookups = offer_lookup_counts().since(offer_lookups_before);
    info!(
        partition = partition.start,
        indexed = stats.indexed,
        skipped = stats.skipped,
        trade_ticks = stats.trade_ticks,
        amm_ticks = stats.amm_ticks,
        oracle_rows = stats.oracle_rows,
        candles = stats.candles_written,
        bytes = stats.total_bytes,
        order_book_fills = offer_lookups.order_book_fills,
        offer_lookup_misses = offer_lookups.offer_lookup_misses,
        pool_fills = offer_lookups.pool_fills,
        wall_secs = format!("{:.1}", stats.wall_clock.as_secs_f64()),
        "partition indexing complete"
    );

    Ok(stats)
}

fn ledger_minute(lcm: &stellar_xdr::LedgerCloseMeta) -> u32 {
    let closed_at = match lcm {
        stellar_xdr::LedgerCloseMeta::V0(v) => v.ledger_header.header.scp_value.close_time.0,
        stellar_xdr::LedgerCloseMeta::V1(v) => v.ledger_header.header.scp_value.close_time.0,
        stellar_xdr::LedgerCloseMeta::V2(v) => v.ledger_header.header.scp_value.close_time.0,
    };
    ((closed_at as u32) / 60) * 60
}

/// Best-effort decode of a single ledger's candle-minute straight from its
/// on-disk partition file, without going through the indexing path. Used by the
/// minute-alignment guard to peek at the ledger on the *other* side of the
/// activation split — which is present on disk (the whole partition folder is
/// synced) even though it is outside the run's in-range window. Returns `None`
/// if the file is absent or the ledger isn't in it (archive tail-lag, a
/// partition-aligned split), so the guard simply skips rather than fails.
pub async fn peek_ledger_minute(partition: &Partition, seq: u32, temp_dir: &Path) -> Option<u32> {
    let path = partition.local_ledger_path(seq, temp_dir);
    let compressed = tokio::fs::read(&path).await.ok()?;
    let xdr_bytes = xdr_parser::decompress_zstd(&compressed).ok()?;
    let batch = xdr_parser::deserialize_batch(&xdr_bytes).ok()?;
    batch
        .ledger_close_metas
        .iter()
        .find(|lcm| ledger_sequence(lcm) == seq)
        .map(ledger_minute)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    const MINUTE_M: i64 = 1_700_000_040; // minute start 1_700_000_040
    const MINUTE_M_LATE: i64 = 1_700_000_070; // same minute, later second
    const MINUTE_M_PLUS_1: i64 = 1_700_000_100;

    fn minute_of(closed_at: i64) -> u32 {
        (closed_at as u32 / 60) * 60
    }

    fn fill(ledger: u32, closed_at: i64, price: Decimal, price_forming: bool) -> TradeTick {
        TradeTick {
            ledger_sequence: ledger,
            closed_at,
            transaction_index: 0,
            operation_index: 0,
            claim_index: 0,
            base_id: 1,
            quote_id: 2,
            price,
            volume_base: Decimal::from(1),
            volume_quote: price,
            price_forming,
        }
    }

    /// Review A BL-01: a minute straddling a 64k-ledger partition boundary must
    /// be written ONCE, from all of its fills.
    ///
    /// The tail half here is dust, which is what makes the old behaviour
    /// destructive rather than merely wasteful: the second write carried the
    /// later ledger's `version`, so `ReplacingMergeTree` kept it — a row with
    /// `pf_trade_count = 0` and no price at all, in place of the priced one the
    /// first write had put there.
    #[test]
    fn a_minute_straddling_a_partition_boundary_is_written_once_with_its_price() {
        let mut accs = RunAccumulators::new();

        // Last ledgers of partition A: an ordinary, price-forming fill.
        accs.merge_sdex(&fill(100, MINUTE_M, Decimal::from(2), true));
        assert!(
            accs.drain_closed(minute_of(MINUTE_M)).is_empty(),
            "the minute the last ledger closed in is still open"
        );

        // The partition boundary. Another partition follows, so nothing is due.
        let at_boundary = candles_at_partition_end(&mut accs, PartitionEnd::Carry);
        assert!(
            at_boundary.is_empty(),
            "a partition boundary is not a flush point: the open minute is \
             carried, or its second half replaces its first in ClickHouse"
        );

        // First ledgers of partition B: only dust in the same minute.
        accs.merge_sdex(&fill(
            101,
            MINUTE_M_LATE,
            Decimal::new(588_235_294, 10), // 1/17
            false,
        ));

        // A ledger in the next minute finally closes M.
        let drained = accs.drain_closed(minute_of(MINUTE_M_PLUS_1));
        assert_eq!(drained.len(), 1, "one source wrote");
        let (source, candles) = &drained[0];
        assert_eq!(*source, SDEX_SOURCE);
        assert_eq!(candles.len(), 1, "ONE candle for the boundary minute");
        let c = &candles[0];
        assert_eq!(c.minute_start, minute_of(MINUTE_M));
        assert_eq!(c.trade_count, 2, "both halves of the minute counted");
        assert_eq!(c.pf_trade_count, 1, "the priced fill still forms the price");
        assert_eq!(c.open, Decimal::from(2));
        assert_eq!(c.close, Decimal::from(2), "the dust fill does not close it");
    }

    /// The other half of the rule: what is carried must still be written. The
    /// last partition of a run has nothing to carry into, so it drains.
    #[test]
    fn the_last_partition_of_a_run_writes_the_minute_it_still_holds_open() {
        let mut accs = RunAccumulators::new();
        accs.merge_sdex(&fill(100, MINUTE_M, Decimal::from(2), true));

        let drained = candles_at_partition_end(&mut accs, PartitionEnd::Drain);
        assert_eq!(drained.len(), 1, "the open minute is written, not dropped");
        assert_eq!(drained[0].1[0].minute_start, minute_of(MINUTE_M));
        assert!(
            candles_at_partition_end(&mut accs, PartitionEnd::Drain).is_empty(),
            "draining twice writes nothing twice"
        );
    }

    /// AMM venues are carried by the same accumulator set, so the boundary rule
    /// cannot hold for SDEX and quietly not hold for soroswap.
    #[test]
    fn an_amm_venues_open_minute_is_carried_across_the_boundary_too() {
        let mut accs = RunAccumulators::new();
        accs.merge_amm("soroswap", &fill(100, MINUTE_M, Decimal::from(3), true));

        assert!(candles_at_partition_end(&mut accs, PartitionEnd::Carry).is_empty());

        accs.merge_amm(
            "soroswap",
            &fill(101, MINUTE_M_LATE, Decimal::from(4), true),
        );
        let drained = accs.drain_closed(minute_of(MINUTE_M_PLUS_1));
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].0, "soroswap");
        assert_eq!(drained[0].1.len(), 1);
        assert_eq!(drained[0].1[0].trade_count, 2);
        assert_eq!(drained[0].1[0].close, Decimal::from(4));
    }
}
