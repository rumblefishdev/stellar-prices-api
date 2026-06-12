use std::path::Path;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tracing::{info, warn};

use crate::error::BackfillError;
use crate::partition::{PARTITION_SIZE, Partition};

const RETRY_ATTEMPTS: u32 = 3;
const RETRY_BASE_DELAY: Duration = Duration::from_secs(2);
const RETRY_MAX_DELAY: Duration = Duration::from_secs(30);
const RETRY_MULTIPLIER: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOutcome {
    Complete,
    S3Incomplete {
        local: usize,
        s3: usize,
        need: usize,
    },
}

pub async fn sync_partition(
    partition: &Partition,
    temp_dir: &Path,
) -> Result<SyncOutcome, BackfillError> {
    let local = partition.local_folder(temp_dir);
    tokio::fs::create_dir_all(&local).await?;

    if let Some((file_count, total_bytes)) = local_partition_complete(&local).await? {
        info!(
            partition = partition.start,
            file_count,
            total_bytes,
            "partition local folder already complete — skipping aws s3 sync"
        );
        return Ok(SyncOutcome::Complete);
    }

    let start = Instant::now();
    run_sync_with_retry(partition, &local).await?;
    let duration = start.elapsed();
    let (file_count, total_bytes) = dir_stats(&local).await?;

    if file_count == PARTITION_SIZE as usize {
        info!(
            partition = partition.start,
            sync_duration_ms = duration.as_millis(),
            file_count,
            total_bytes,
            "partition sync complete"
        );
        return Ok(SyncOutcome::Complete);
    }

    // Archive tail-lag: the public archive is routinely a few ledgers short of
    // a full 64k partition at its tail, and the live tip partition is only
    // partially published. Rather than skip the whole partition, index whatever
    // the archive currently exposes — we treat it as ready once our local copy
    // holds every object the archive lists; `index_partition` skips any
    // individually-missing ledger. This also lets the run pick up the partial
    // tip partition instead of dropping its (already-published) ledgers.
    let s3_count = s3_object_count(partition).await?;
    if s3_count == 0 {
        // The archive listing came back empty — either a genuinely-unpublished
        // partition or a transient `aws s3 ls` hiccup. Either way we cannot
        // confirm completeness, so defer this partition (skipped this run,
        // retried next) rather than aborting the whole backfill with a
        // `need: 0` hard error.
        warn!(
            partition = partition.start,
            local_files = file_count,
            "archive listing empty/unavailable — deferring partition"
        );
        return Ok(SyncOutcome::S3Incomplete {
            local: file_count,
            s3: 0,
            need: PARTITION_SIZE as usize,
        });
    }
    if file_count >= s3_count {
        info!(
            partition = partition.start,
            sync_duration_ms = duration.as_millis(),
            file_count,
            s3_count,
            total_bytes,
            "partition synced — have all archive objects (tail-lag tolerated)"
        );
        return Ok(SyncOutcome::Complete);
    }

    warn!(
        partition = partition.start,
        local_files = file_count,
        s3_files = s3_count,
        "local sync behind archive — retrying"
    );
    run_sync_once(partition, &local).await?;
    let (file_count_retry, _) = dir_stats(&local).await?;
    if file_count_retry >= s3_count && s3_count > 0 {
        info!(
            partition = partition.start,
            file_count_retry, s3_count, "partition sync complete after retry"
        );
        return Ok(SyncOutcome::Complete);
    }

    Err(BackfillError::PartitionSyncFailed {
        partition_start: partition.start,
        local: file_count_retry,
        s3: s3_count,
        need: s3_count,
    })
}

async fn local_partition_complete(dir: &Path) -> Result<Option<(usize, u64)>, BackfillError> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let mut count = 0usize;
    let mut bytes = 0u64;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        if !name.to_string_lossy().ends_with(".xdr.zst") {
            continue;
        }
        count += 1;
        bytes += entry.metadata().await?.len();
    }
    if count == PARTITION_SIZE as usize {
        Ok(Some((count, bytes)))
    } else {
        Ok(None)
    }
}

async fn run_sync_with_retry(partition: &Partition, local: &Path) -> Result<(), BackfillError> {
    let mut delay = RETRY_BASE_DELAY;
    for attempt in 1..=RETRY_ATTEMPTS {
        match run_sync_once(partition, local).await {
            Ok(()) => return Ok(()),
            Err(err) if attempt == RETRY_ATTEMPTS => return Err(err),
            Err(err) => {
                warn!(
                    partition = partition.start,
                    attempt, error = %err,
                    retry_in_secs = delay.as_secs(),
                    "aws s3 sync failed, retrying"
                );
                tokio::time::sleep(delay).await;
                delay = delay.saturating_mul(RETRY_MULTIPLIER).min(RETRY_MAX_DELAY);
            }
        }
    }
    unreachable!()
}

async fn run_sync_once(partition: &Partition, local: &Path) -> Result<(), BackfillError> {
    let s3 = partition.s3_folder();
    info!(
        partition = partition.start,
        s3 = %s3, local = %local.display(),
        "running aws s3 sync"
    );

    let output = Command::new("aws")
        .args(["s3", "sync", &s3])
        .arg(local)
        .args(["--no-sign-request", "--quiet"])
        .output()
        .await?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(BackfillError::AwsSyncFailed {
        partition: partition.start,
        exit_code: output.status.code().unwrap_or(-1),
        stderr: stderr.chars().take(2_000).collect(),
    })
}

async fn s3_object_count(partition: &Partition) -> Result<usize, BackfillError> {
    let s3 = partition.s3_folder();
    let output = Command::new("aws")
        .args(["s3", "ls", "--recursive", "--no-sign-request", &s3])
        .output()
        .await
        .map_err(|source| BackfillError::S3LsFailed {
            partition_start: partition.start,
            source,
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BackfillError::S3LsFailed {
            partition_start: partition.start,
            source: std::io::Error::other(format!(
                "aws s3 ls exited {:?}: {}",
                output.status.code(),
                stderr.chars().take(500).collect::<String>(),
            )),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let count = stdout
        .lines()
        .filter(|line| {
            line.split_whitespace()
                .next_back()
                .is_some_and(|tok| tok.ends_with(".xdr.zst"))
        })
        .count();
    Ok(count)
}

async fn dir_stats(dir: &Path) -> Result<(usize, u64), BackfillError> {
    let mut entries = tokio::fs::read_dir(dir).await?;
    let mut count = 0usize;
    let mut bytes = 0u64;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        if !name.to_string_lossy().ends_with(".xdr.zst") {
            continue;
        }
        count += 1;
        bytes += entry.metadata().await?.len();
    }
    Ok((count, bytes))
}
