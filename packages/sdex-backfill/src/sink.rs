//! Backfill ClickHouse sink — a thin wrapper over the shared
//! [`prices_ingest_core::OhlcvWriter`].
//!
//! The candle / asset / oracle writes (and the asset-registry load) are shared
//! with the live Lambda and live in the core writer, so both paths emit
//! byte-identical `prices.*` rows. This wrapper adds only the **backfill-only**
//! resume bookkeeping against `prices.backfill_sdex_ledgers` (the live Lambda
//! uses its own doorbell cursor instead). `OracleSample` is re-exported so the
//! rest of the backfill keeps importing it from `crate::sink`.

use std::collections::HashSet;

use clickhouse::Row;
use prices_ingest_core::canonical::AssetIdentity;
use prices_ingest_core::{
    AssetRegistry, DEFAULT_BACKOFF_MS, OhlcvCandle, OhlcvWriter, PoolRegistryRow, Registries,
    UnresolvedPool, retry_with_backoff,
};
use serde::{Deserialize, Serialize};
use tracing::info;

pub use prices_ingest_core::OracleSample;

use crate::error::BackfillError;
use crate::progress::{Current, ProgressStatus, ProgressUpdate, SDEX_ARCHIVE, SOROBAN_AMM};

pub struct Sink {
    writer: OhlcvWriter,
}

impl Sink {
    pub fn new(url: &str) -> Self {
        Self {
            writer: OhlcvWriter::plaintext(url),
        }
    }

    /// Direct-write sink to the Hetzner `prices.*` cluster over mTLS (ADR 0009).
    ///
    /// Unlike the live Lambdas' `client_from_lambda_env` — which fetches the
    /// bundle from the Parameters & Secrets Extension on `localhost:2773` — the
    /// backfill runs on an operator workstation, so it reads the client bundle
    /// from on-disk PEM files and builds the task-0052 client with
    /// [`prices_clickhouse::mtls::client_with_mtls`] (the same workstation entry
    /// point the 0052 round-trip smoke test uses). The key is read straight into
    /// rustls and never logged.
    #[cfg(feature = "aws-mtls")]
    pub fn mtls(
        domain: &str,
        cert_path: &std::path::Path,
        key_path: &std::path::Path,
        ca_path: &std::path::Path,
        database: &str,
    ) -> Result<Self, BackfillError> {
        use prices_clickhouse::mtls::{MtlsBundle, client_with_mtls};

        let read = |p: &std::path::Path| -> Result<String, BackfillError> {
            std::fs::read_to_string(p)
                .map_err(|e| BackfillError::Mtls(format!("read PEM at `{}`: {e}", p.display())))
        };
        let bundle = MtlsBundle {
            cert_pem: read(cert_path)?,
            key_pem: read(key_path)?,
            ca_pem: read(ca_path)?,
        };
        let client = client_with_mtls(domain, &bundle, database)
            .map_err(|e| BackfillError::Mtls(e.to_string()))?;
        Ok(Self {
            writer: OhlcvWriter::new(client),
        })
    }

    pub async fn preflight(&self) -> Result<(), BackfillError> {
        self.writer.preflight().await?;
        Ok(())
    }

    pub async fn load_assets(&self) -> Result<Vec<(u32, AssetIdentity)>, BackfillError> {
        Ok(self.writer.load_assets().await?)
    }

    // All `prices.*` writes below are idempotent (ReplacingMergeTree keyed by
    // `version`), so a retried INSERT can only replace, never duplicate. That
    // lets the sink retry every failure as transient (`|_| true`) — a bounded
    // `[50, 200, 800] ms` backoff so a passing CH/network blip does not abort a
    // multi-hour backfill. Same envelope and classifier as the live sink. Every
    // write goes through `retry_write` so the policy lives in exactly one place.

    /// Run one idempotent `prices.*` write under the shared retry envelope: the
    /// bounded `[50, 200, 800] ms` backoff, every error treated as transient
    /// (safe because the writes are ReplacingMergeTree-idempotent). Any error
    /// type that maps into [`BackfillError`] works, so both `IngestError` and
    /// raw `clickhouse::error::Error` closures share it.
    async fn retry_write<F, Fut, E>(&self, op: F) -> Result<(), BackfillError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<(), E>>,
        BackfillError: From<E>,
    {
        retry_with_backoff(&DEFAULT_BACKOFF_MS, |_| true, op)
            .await
            .map(|_| ())
            .map_err(BackfillError::from)
    }

    pub async fn write_candles(
        &self,
        candles: &[OhlcvCandle],
        source: &str,
    ) -> Result<(), BackfillError> {
        self.retry_write(|| async { self.writer.write_candles(candles, source).await })
            .await
    }

    pub async fn write_assets(&self, registry: &AssetRegistry) -> Result<(), BackfillError> {
        self.retry_write(|| async { self.writer.write_assets(registry).await })
            .await
    }

    pub async fn write_oracle(&self, samples: &[OracleSample]) -> Result<(), BackfillError> {
        self.retry_write(|| async { self.writer.write_oracle(samples).await })
            .await
    }

    pub async fn write_unresolved_pools(
        &self,
        pools: &[UnresolvedPool],
    ) -> Result<(), BackfillError> {
        self.retry_write(|| async { self.writer.write_unresolved_pools(pools).await })
            .await
    }

    /// Persist the discovered AMM pool registry to `prices.pool_registry` (task
    /// 0053, decision #4) — the durable output a partial re-backfill or the live
    /// processor can [`load_pool_registry`](Self::load_pool_registry) instead of
    /// re-deriving from activation. Idempotent: ReplacingMergeTree on
    /// `contract_id`, so a re-run replaces rather than duplicates.
    pub async fn write_pool_registry(&self, reg: &Registries) -> Result<(), BackfillError> {
        let rows = reg.to_pool_rows();
        if rows.is_empty() {
            return Ok(());
        }
        self.retry_write(|| async {
            let mut insert = self.writer.client().insert("prices.pool_registry")?;
            for row in &rows {
                insert.write(row).await?;
            }
            insert.end().await?;
            Ok::<(), clickhouse::error::Error>(())
        })
        .await?;
        info!(pools = rows.len(), "persisted discovered pool registry");
        Ok(())
    }

    /// Rehydrate a [`Registries`] from `prices.pool_registry` so a run over a
    /// window that starts after activation still resolves pools created earlier
    /// (decision #4 inverts task 0069). Returns empty registries when the table
    /// is empty — the fresh full-run case.
    pub async fn load_pool_registry(&self) -> Result<Registries, BackfillError> {
        // Retried like the writes: this runs once at startup, and a transient CH
        // blip here would otherwise abort the whole multi-hour run before a
        // single ledger is indexed.
        let rows = retry_with_backoff(
            &DEFAULT_BACKOFF_MS,
            |_| true,
            || async {
                self.writer
                    .client()
                    .query(
                        "SELECT contract_id, venue, token0, token1, pool_type, wasm_hash \
                     FROM prices.pool_registry FINAL",
                    )
                    .fetch_all::<PoolRegistryRow>()
                    .await
            },
        )
        .await
        .map(|(rows, _tries)| rows)
        .map_err(BackfillError::from)?;
        let mut reg = Registries::new();
        reg.load_pool_rows(&rows);
        info!(
            entries = rows.len(),
            "loaded discovered pool registry from prices.pool_registry"
        );
        Ok(reg)
    }

    /// Upsert one `prices.backfill_progress` row from a [`ProgressUpdate`]
    /// (task 0053 / decision 6). ReplacingMergeTree keyed by `task_name`, so we
    /// read-modify-write: the new row (fresh `updated_at = now()`) replaces the
    /// old one on merge.
    ///
    /// Read-modify preserves `started_at`; merges `current_ledger` monotonically
    /// against the stored value ([`resolve_current`]: forward stream keeps the
    /// max, backward stream the min, [`Current::Keep`] leaves it untouched) so a
    /// partial / resumed / out-of-order run never regresses it; never downgrades
    /// a stored `completed` back to `running` ([`resolve_status`]); and widens
    /// the `[earliest, newest]_data_available` window monotonically (min of
    /// earliest, max of newest) so the covered span never narrows. `last_push_at`
    /// `last_push_at` is stamped server-side with `now()`; `updated_at` (the RMT
    /// version) is forced strictly past the stored value so back-to-back writes
    /// in the same second can't tie; `completed_at` is stamped once on the
    /// transition into `completed` and preserved thereafter, so no wall-clock is
    /// read in-process.
    pub async fn write_progress(&self, update: &ProgressUpdate) -> Result<(), BackfillError> {
        self.retry_write(|| async { self.write_progress_once(update).await })
            .await
    }

    async fn write_progress_once(
        &self,
        update: &ProgressUpdate,
    ) -> Result<(), clickhouse::error::Error> {
        // `task_name` is interpolated straight into the INSERT below, so hard-gate
        // it to the two known stream constants. Today's callers only ever pass
        // these, but this keeps a future dynamic caller from opening an injection
        // hole through the string-built SQL. All other interpolated values are
        // numeric or fixed-string enums.
        if update.task_name != SDEX_ARCHIVE && update.task_name != SOROBAN_AMM {
            return Err(clickhouse::error::Error::Custom(format!(
                "refusing to write progress for unknown task_name {:?}",
                update.task_name
            )));
        }

        let existing = self
            .writer
            .client()
            .query(
                "SELECT current_ledger, \
                        toString(status) AS status, \
                        toUnixTimestamp(started_at) AS started_at, \
                        toUnixTimestamp(completed_at) AS completed, \
                        toUnixTimestamp(earliest_data_available) AS earliest, \
                        toUnixTimestamp(newest_data_available) AS newest, \
                        toUnixTimestamp(updated_at) AS updated \
                 FROM prices.backfill_progress FINAL WHERE task_name = ?",
            )
            .bind(update.task_name)
            .fetch_all::<ExistingProgress>()
            .await?
            .into_iter()
            .next();

        // current_ledger merged monotonically against the stored value: a
        // forward stream keeps the max, a backward stream the min, so a partial
        // / resumed / out-of-order run can never move it the wrong way (nor
        // un-do an archive a prior run already carried down to genesis).
        let current = resolve_current(
            update.current_ledger,
            existing.as_ref().map(|e| e.current_ledger),
        );
        // Never downgrade a stored `completed` back to `running` — a separate
        // run may have already finished this stream.
        let status = resolve_status(existing.as_ref().map(|e| e.status.as_str()), update.status);
        // Monotonic window: never narrow what a prior run already recorded.
        let earliest = merge_min(
            existing.as_ref().and_then(|e| e.earliest),
            update.earliest_minute,
        );
        let newest = merge_max(
            existing.as_ref().and_then(|e| e.newest),
            update.newest_minute,
        );

        // Preserve started_at across updates; server `now()` seeds a fresh row.
        let started_at_sql = match existing.as_ref() {
            Some(e) => format!("toDateTime({})", e.started_at),
            None => "now()".to_string(),
        };
        // completed_at: stamp `now()` only on the transition INTO completed;
        // preserve the original stamp if the stream was already completed; NULL
        // while running (so a mid-run write never wipes a real completion time).
        let completed_at_sql = match status {
            ProgressStatus::Completed => match existing.as_ref().and_then(|e| e.completed) {
                Some(ts) => format!("toDateTime({ts})"),
                None => "now()".to_string(),
            },
            _ => "NULL".to_string(),
        };
        let earliest_sql = earliest.map_or("NULL".to_string(), |e| format!("toDateTime({e})"));
        let newest_sql = newest.map_or("NULL".to_string(), |n| format!("toDateTime({n})"));

        // updated_at is the ReplacingMergeTree version (second resolution). Two
        // updates for the same task_name within one wall-clock second — e.g. the
        // last per-partition `running` write and the terminal `completed` write —
        // would otherwise share a version, and FINAL would keep a nondeterministic
        // winner (losing the completion). Because writes for a task are strictly
        // sequential and each reads the prior row via FINAL, forcing this row's
        // version strictly past the stored one guarantees the latest write always
        // wins the collapse.
        let updated_at_sql = match existing.as_ref() {
            Some(e) => format!("greatest(now(), toDateTime({}))", e.updated as u64 + 1),
            None => "now()".to_string(),
        };

        // All interpolated values are numeric or from fixed-string constants
        // (task_name is gated above to the two stream names, status to the enum),
        // so there is no injection surface. Datetimes are set server-side (now() /
        // toDateTime(unix)) to avoid encoding a Rust DateTime.
        let sql = format!(
            "INSERT INTO prices.backfill_progress \
             (task_name, start_ledger, target_ledger, current_ledger, status, \
              last_push_at, earliest_data_available, newest_data_available, \
              started_at, completed_at, updated_at) \
             VALUES ('{task}', {start}, {target}, {current}, '{status}', \
              now(), {earliest_sql}, {newest_sql}, {started_at_sql}, {completed_at_sql}, {updated_at_sql})",
            task = update.task_name,
            start = update.start_ledger,
            target = update.target_ledger,
            current = current,
            status = status.as_ch(),
        );
        self.writer.client().query(&sql).execute().await
    }

    // --- backfill-only resume bookkeeping (prices.backfill_sdex_ledgers) ---

    pub async fn load_completed(
        &self,
        start: u32,
        end: u32,
    ) -> Result<HashSet<u32>, BackfillError> {
        let rows = self
            .writer
            .client()
            .query(
                "SELECT sequence FROM prices.backfill_sdex_ledgers \
                 WHERE sequence BETWEEN ? AND ?",
            )
            .bind(start)
            .bind(end)
            .fetch_all::<u32>()
            .await?;

        let set: HashSet<u32> = rows.into_iter().collect();
        info!(
            start,
            end,
            completed = set.len(),
            "loaded completed ledgers from backfill_sdex_ledgers"
        );
        Ok(set)
    }

    pub async fn write_completed_ledgers(&self, sequences: &[u32]) -> Result<(), BackfillError> {
        if sequences.is_empty() {
            return Ok(());
        }
        // Idempotent resume bookkeeping (RMT keyed by sequence) → retry the whole
        // batch INSERT as transient, same bounded backoff as the candle writes.
        self.retry_write(|| async {
            let mut insert = self
                .writer
                .client()
                .insert("prices.backfill_sdex_ledgers")?;
            for &seq in sequences {
                insert.write(&LedgerRow { sequence: seq }).await?;
            }
            insert.end().await?;
            Ok::<(), clickhouse::error::Error>(())
        })
        .await
    }
}

#[derive(Debug, Serialize, Row)]
struct LedgerRow {
    sequence: u32,
}

/// Mergeable state read back from an existing `backfill_progress` row. Field
/// order MUST match the `SELECT` column order in `write_progress_once` (the
/// clickhouse `Row` derive binds positionally). `completed` / `earliest` /
/// `newest` are `toUnixTimestamp(Nullable(DateTime))` → `Option`.
#[derive(Debug, Row, Deserialize)]
struct ExistingProgress {
    current_ledger: u64,
    status: String,
    started_at: u32,
    completed: Option<u32>,
    earliest: Option<u32>,
    newest: Option<u32>,
    updated: u32,
}

/// Resolve the `current_ledger` to write, merged monotonically against the
/// stored value so a partial / resumed / out-of-order run never moves it the
/// wrong way. A forward stream keeps `max(new, stored)`; a backward stream keeps
/// `min(new, stored)`, treating the seeded `0` (never a real reflected ledger —
/// genesis is 1) as unset. `Keep` preserves the stored value (0 on a fresh row).
fn resolve_current(update: Current, existing: Option<u64>) -> u64 {
    match update {
        Current::SetForward(v) => existing.map_or(v, |e| e.max(v)),
        Current::SetBackward(v) => match existing {
            Some(e) if e != 0 => e.min(v),
            _ => v,
        },
        Current::Keep => existing.unwrap_or(0),
    }
}

/// Never downgrade a stored `completed` back to `running`: a separate run may
/// have already finished this stream (e.g. the sdex-only pass completes the
/// archive before a combined pass touches it). Any other transition takes the
/// update's status.
fn resolve_status(existing: Option<&str>, update: ProgressStatus) -> ProgressStatus {
    match (existing, update) {
        (Some("completed"), ProgressStatus::Running) => ProgressStatus::Completed,
        _ => update,
    }
}

/// Lowest of two optional minute watermarks (monotonic earliest). Shared with
/// `run.rs`'s run-level window merge.
pub(crate) fn merge_min(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, y) => x.or(y),
    }
}

/// Highest of two optional minute watermarks (monotonic newest). Shared with
/// `run.rs`'s run-level window merge.
pub(crate) fn merge_max(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, y) => x.or(y),
    }
}

#[cfg(test)]
mod tests {
    use super::{merge_max, merge_min, resolve_current, resolve_status};
    use crate::progress::{Current, ProgressStatus};

    #[test]
    fn forward_current_never_regresses() {
        // Resume that indexed less than a prior run keeps the higher watermark.
        assert_eq!(resolve_current(Current::SetForward(50), Some(100)), 100);
        assert_eq!(resolve_current(Current::SetForward(100), Some(50)), 100);
        assert_eq!(resolve_current(Current::SetForward(42), None), 42);
    }

    #[test]
    fn backward_current_never_regresses_and_ignores_placeholder_zero() {
        // A combined pass (floor=activation) must not un-do a completed archive
        // already carried down to genesis (current=1).
        assert_eq!(
            resolve_current(Current::SetBackward(50_463_000), Some(1)),
            1
        );
        // Lower floor wins going backward.
        assert_eq!(resolve_current(Current::SetBackward(10), Some(100)), 10);
        // Seeded 0 placeholder is treated as unset, not as "reached genesis".
        assert_eq!(resolve_current(Current::SetBackward(500), Some(0)), 500);
        assert_eq!(resolve_current(Current::SetBackward(500), None), 500);
    }

    #[test]
    fn keep_preserves_stored_current() {
        assert_eq!(resolve_current(Current::Keep, Some(123)), 123);
        assert_eq!(resolve_current(Current::Keep, None), 0);
    }

    #[test]
    fn status_never_downgrades_completed() {
        // A mid-run running write over an already-completed stream stays done.
        assert_eq!(
            resolve_status(Some("completed"), ProgressStatus::Running),
            ProgressStatus::Completed
        );
        // Running → completed and fresh rows take the update as-is.
        assert_eq!(
            resolve_status(Some("running"), ProgressStatus::Completed),
            ProgressStatus::Completed
        );
        assert_eq!(
            resolve_status(None, ProgressStatus::Running),
            ProgressStatus::Running
        );
    }

    #[test]
    fn merge_min_takes_the_older_and_tolerates_gaps() {
        assert_eq!(merge_min(Some(100), Some(50)), Some(50));
        assert_eq!(merge_min(None, Some(50)), Some(50));
        assert_eq!(merge_min(Some(100), None), Some(100));
        assert_eq!(merge_min(None, None), None);
    }

    #[test]
    fn merge_max_takes_the_newer_and_tolerates_gaps() {
        assert_eq!(merge_max(Some(100), Some(50)), Some(100));
        assert_eq!(merge_max(None, Some(50)), Some(50));
        assert_eq!(merge_max(Some(100), None), Some(100));
        assert_eq!(merge_max(None, None), None);
    }
}
