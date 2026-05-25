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

    let s3_count = s3_object_count(partition).await?;
    if s3_count < PARTITION_SIZE as usize {
        warn!(
            partition = partition.start,
            local_files = file_count,
            s3_files = s3_count,
            need = PARTITION_SIZE,
            "S3 archive lag — partition incomplete on S3, skipping"
        );
        return Ok(SyncOutcome::S3Incomplete {
            local: file_count,
            s3: s3_count,
            need: PARTITION_SIZE as usize,
        });
    }

    warn!(
        partition = partition.start,
        local_files = file_count,
        s3_files = s3_count,
        "local sync partial despite full S3 — retrying"
    );
    run_sync_once(partition, &local).await?;
    let (file_count_retry, _) = dir_stats(&local).await?;
    if file_count_retry == PARTITION_SIZE as usize {
        info!(
            partition = partition.start,
            "partition sync complete after retry"
        );
        return Ok(SyncOutcome::Complete);
    }

    Err(BackfillError::PartitionSyncFailed {
        partition_start: partition.start,
        local: file_count_retry,
        s3: s3_count,
        need: PARTITION_SIZE as usize,
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
