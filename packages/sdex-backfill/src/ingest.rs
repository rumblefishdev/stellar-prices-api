use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use prices_ingest_core::{
    AssetRegistry, CandleAccumulator, Registries, extract_trades, process_ledger, raw_trade_to_tick,
};

use crate::error::BackfillError;
use crate::partition::Partition;
use crate::sink::{OracleSample, Sink};

const ORACLE_FLUSH_THRESHOLD: usize = 50_000;

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
            }

            let current_minute = ledger_minute(lcm);
            let candles = sdex.flush_older_than(current_minute);
            if !candles.is_empty() {
                sink.write_candles(&candles, "sdex").await?;
                stats.candles_written += candles.len();
            }
            for (source, acc) in amm.iter_mut() {
                let c = acc.flush_older_than(current_minute);
                if !c.is_empty() {
                    sink.write_candles(&c, *source).await?;
                    stats.candles_written += c.len();
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
        stats.candles_written += remaining.len();
    }
    for (source, acc) in amm.iter_mut() {
        let c = acc.flush_all();
        if !c.is_empty() {
            sink.write_candles(&c, *source).await?;
            stats.candles_written += c.len();
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

fn ledger_minute(lcm: &stellar_xdr::curr::LedgerCloseMeta) -> u32 {
    let closed_at = match lcm {
        stellar_xdr::curr::LedgerCloseMeta::V0(v) => v.ledger_header.header.scp_value.close_time.0,
        stellar_xdr::curr::LedgerCloseMeta::V1(v) => v.ledger_header.header.scp_value.close_time.0,
        stellar_xdr::curr::LedgerCloseMeta::V2(v) => v.ledger_header.header.scp_value.close_time.0,
    };
    ((closed_at as u32) / 60) * 60
}
