use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use tracing::info;

use crate::bucket::CandleAccumulator;
use crate::canonical::AssetRegistry;
use crate::error::BackfillError;
use crate::filter::extract_trades;
use crate::partition::Partition;
use crate::sink::Sink;
use crate::tick::raw_trade_to_tick;

#[derive(Debug, Clone, Default)]
pub struct PartitionStats {
    pub indexed: usize,
    pub skipped: usize,
    pub trade_ticks: usize,
    pub candles_written: usize,
    pub total_bytes: u64,
    pub wall_clock: Duration,
}

pub async fn index_partition(
    partition: &Partition,
    temp_dir: &Path,
    sink: &Sink,
    range_start: u32,
    range_end: u32,
    completed: &HashSet<u32>,
    registry: &mut AssetRegistry,
) -> Result<PartitionStats, BackfillError> {
    let (first, last) = partition.clamped(range_start, range_end);
    info!(
        partition = partition.start,
        first, last, "partition indexing started"
    );

    let wall_start = Instant::now();
    let mut stats = PartitionStats::default();
    let mut accumulator = CandleAccumulator::new();
    let mut ledgers_in_partition: Vec<u32> = Vec::new();

    for seq in first..=last {
        if completed.contains(&seq) {
            stats.skipped += 1;
            continue;
        }

        let path = partition.local_ledger_path(seq, temp_dir);
        if !path.exists() {
            return Err(BackfillError::LedgerFileMissing {
                partition: partition.start,
                seq,
                path: path.display().to_string(),
            });
        }

        let compressed = tokio::fs::read(&path).await?;
        stats.total_bytes += compressed.len() as u64;

        let xdr_bytes = xdr_parser::decompress_zstd(&compressed)?;
        let batch = xdr_parser::deserialize_batch(&xdr_bytes)?;

        for lcm in batch.ledger_close_metas.iter() {
            let trades = extract_trades(lcm);
            for trade in &trades {
                let tick = raw_trade_to_tick(trade, registry);
                accumulator.merge(&tick);
                stats.trade_ticks += 1;
            }

            let current_minute = ledger_minute(lcm);
            let candles = accumulator.flush_older_than(current_minute);
            if !candles.is_empty() {
                sink.write_candles(&candles).await?;
                stats.candles_written += candles.len();
            }
        }

        ledgers_in_partition.push(seq);
        stats.indexed += 1;
    }

    let remaining = accumulator.flush_all();
    if !remaining.is_empty() {
        sink.write_candles(&remaining).await?;
        stats.candles_written += remaining.len();
    }

    sink.write_completed_ledgers(&ledgers_in_partition).await?;

    stats.wall_clock = wall_start.elapsed();
    info!(
        partition = partition.start,
        indexed = stats.indexed,
        skipped = stats.skipped,
        trade_ticks = stats.trade_ticks,
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
