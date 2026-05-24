use std::path::Path;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::canonical::AssetRegistry;
use crate::error::BackfillError;
use crate::ingest::{PartitionStats, index_partition};
use crate::partition::{Partition, partitions_for_range};
use crate::sink::Sink;
use crate::sync::{SyncOutcome, sync_partition};

pub async fn execute(
    sink: &Sink,
    temp_dir: &Path,
    start: u32,
    end: u32,
    keep_partitions: bool,
) -> Result<(), BackfillError> {
    assert!(start <= end, "invalid range: start ({start}) > end ({end})");

    tokio::fs::create_dir_all(temp_dir).await?;

    preflight_aws().await;
    sink.preflight().await.unwrap_or_else(|e| panic!("pre-flight: sink unreachable: {e}"));
    info!("pre-flight: all checks passed");

    let partitions = partitions_for_range(start, end);
    if partitions.is_empty() {
        info!("no partitions in range");
        return Ok(());
    }

    let completed = sink.load_completed(start, end).await?;

    let todo: Vec<&Partition> = partitions
        .iter()
        .filter(|p| !partition_fully_done(p, start, end, &completed))
        .collect();

    info!(
        start, end,
        total_partitions = partitions.len(),
        already_done = partitions.len() - todo.len(),
        to_process = todo.len(),
        "backfill starting"
    );

    if todo.is_empty() {
        info!("nothing to do — all partitions fully indexed");
        return Ok(());
    }

    let run_start = Instant::now();
    let mut totals = PartitionStats::default();
    let mut partitions_skipped_s3: usize = 0;
    let mut registry = AssetRegistry::new();

    sync_partition(todo[0], temp_dir).await?;

    for (i, partition) in todo.iter().enumerate() {
        let next_handle: Option<JoinHandle<Result<SyncOutcome, BackfillError>>> =
            if let Some(next) = todo.get(i + 1) {
                let next = (*next).clone();
                let temp = temp_dir.to_path_buf();
                Some(tokio::spawn(async move {
                    sync_partition(&next, &temp).await
                }))
            } else {
                None
            };

        let stats = index_partition(
            partition,
            temp_dir,
            sink,
            start,
            end,
            &completed,
            &mut registry,
        )
        .await?;

        totals.indexed += stats.indexed;
        totals.skipped += stats.skipped;
        totals.trade_ticks += stats.trade_ticks;
        totals.candles_written += stats.candles_written;
        totals.total_bytes += stats.total_bytes;

        if !keep_partitions {
            let local = partition.local_folder(temp_dir);
            match tokio::fs::remove_dir_all(&local).await {
                Ok(()) => info!(partition = partition.start, "cleaned up local folder"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(BackfillError::Io(e)),
            }
        }

        if let Some(h) = next_handle {
            match h.await.expect("prefetch task panicked")? {
                SyncOutcome::Complete => {}
                SyncOutcome::S3Incomplete { local, s3, need } => {
                    warn!(local, s3, need, "next partition S3 incomplete — will skip");
                    partitions_skipped_s3 += 1;
                }
            }
        }
    }

    sink.write_assets(&registry).await?;

    let elapsed = run_start.elapsed();
    print_run_summary(todo.len(), &totals, partitions_skipped_s3, elapsed);

    Ok(())
}

fn partition_fully_done(
    partition: &Partition,
    start: u32,
    end: u32,
    completed: &std::collections::HashSet<u32>,
) -> bool {
    let (first, last) = partition.clamped(start, end);
    (first..=last).all(|s| completed.contains(&s))
}

fn print_run_summary(
    partitions_processed: usize,
    totals: &PartitionStats,
    partitions_skipped_s3: usize,
    elapsed: Duration,
) {
    println!();
    println!("=== sdex-backfill complete ===");
    println!("partitions processed:      {partitions_processed}");
    println!("partitions skipped (S3):   {partitions_skipped_s3}");
    println!("ledgers indexed:           {}", totals.indexed);
    println!("ledgers already in DB:     {}", totals.skipped);
    println!("trade ticks emitted:       {}", totals.trade_ticks);
    println!("price_ohlcv_1m rows:       {}", totals.candles_written);
    println!("total bytes downloaded:    {}", totals.total_bytes);
    println!("elapsed:                   {} s", elapsed.as_secs());
}

async fn preflight_aws() {
    let out = Command::new("aws")
        .arg("--version")
        .output()
        .await
        .unwrap_or_else(|err| {
            panic!("pre-flight: failed to spawn `aws --version`: {err}");
        });
    if !out.status.success() {
        panic!("pre-flight: `aws --version` exited non-zero");
    }
    info!(
        version = %String::from_utf8_lossy(&out.stdout).trim(),
        "pre-flight: aws CLI present"
    );
}
