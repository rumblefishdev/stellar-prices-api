use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use prices_ingest_core::{
    AssetRegistry, CandleAccumulator, OhlcvCandle, Registries, UnresolvedPoolSwap, extract_trades,
    ledger_sequence, process_ledger, raw_trade_to_tick,
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
    /// partition, or `None` if none were written. Merged up into the run's
    /// `earliest_data_available` / `newest_data_available` — the covered
    /// time-window (§4.5 + task 0053). Both advance monotonically in the forward
    /// pass, so per-partition updates stay truthful for either stream.
    pub earliest_minute: Option<u32>,
    pub latest_minute: Option<u32>,
    /// Per-ledger records of swaps dropped for an unregistered pool. Aggregated
    /// and re-checked against the final registry at run end (see `run.rs`).
    pub unresolved: Vec<UnresolvedPoolSwap>,
}

impl PartitionStats {
    /// Record a batch of just-written candles: bump the count and widen the
    /// [earliest, latest] minute window. Single seam so every write site keeps
    /// the count and the window in sync.
    fn note_candles(&mut self, candles: &[OhlcvCandle]) {
        self.candles_written += candles.len();
        if let Some(lo) = candles.iter().map(|c| c.minute_start).min() {
            self.earliest_minute = Some(self.earliest_minute.map_or(lo, |cur| cur.min(lo)));
        }
        if let Some(hi) = candles.iter().map(|c| c.minute_start).max() {
            self.latest_minute = Some(self.latest_minute.map_or(hi, |cur| cur.max(hi)));
        }
    }
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
) -> Result<PartitionStats, BackfillError> {
    let (first, last) = partition.clamped(range_start, range_end);
    info!(
        partition = partition.start,
        first, last, "partition indexing started"
    );

    let wall_start = Instant::now();
    let mut stats = PartitionStats::default();
    let mut sdex = CandleAccumulator::new();
    // One accumulator per AMM venue source (phoenix / soroswap / aquarius).
    let mut amm: HashMap<&'static str, CandleAccumulator> = HashMap::new();
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
                sdex.merge(&tick);
                stats.trade_ticks += 1;
            }

            // Soroban events from the same ledger: AMM candles + oracle samples.
            // Only in Combined mode — pre-Soroban ledgers carry no Soroban
            // events, so SdexOnly skips the decode entirely.
            if mode == ExtractMode::Combined {
                let sob = process_ledger(lcm, reg, registry);
                for (source, tick) in &sob.amm_ticks {
                    amm.entry(*source)
                        .or_insert_with(CandleAccumulator::new)
                        .merge(tick);
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
            let candles = sdex.flush_older_than(current_minute);
            if !candles.is_empty() {
                sink.write_candles(&candles, "sdex").await?;
                stats.note_candles(&candles);
            }
            for (source, acc) in amm.iter_mut() {
                let c = acc.flush_older_than(current_minute);
                if !c.is_empty() {
                    sink.write_candles(&c, *source).await?;
                    stats.note_candles(&c);
                }
            }
            if oracle_buf.len() >= ORACLE_FLUSH_THRESHOLD {
                sink.write_oracle(&oracle_buf).await?;
                oracle_buf.clear();
            }
        }

        ledgers_in_partition.push(seq);
        stats.indexed += 1;
    }

    let remaining = sdex.flush_all();
    if !remaining.is_empty() {
        sink.write_candles(&remaining, "sdex").await?;
        stats.note_candles(&remaining);
    }
    for (source, acc) in amm.iter_mut() {
        let c = acc.flush_all();
        if !c.is_empty() {
            sink.write_candles(&c, *source).await?;
            stats.note_candles(&c);
        }
    }
    if !oracle_buf.is_empty() {
        sink.write_oracle(&oracle_buf).await?;
    }

    sink.write_completed_ledgers(&ledgers_in_partition).await?;

    stats.wall_clock = wall_start.elapsed();
    info!(
        partition = partition.start,
        indexed = stats.indexed,
        skipped = stats.skipped,
        trade_ticks = stats.trade_ticks,
        amm_ticks = stats.amm_ticks,
        oracle_rows = stats.oracle_rows,
        candles = stats.candles_written,
        bytes = stats.total_bytes,
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
