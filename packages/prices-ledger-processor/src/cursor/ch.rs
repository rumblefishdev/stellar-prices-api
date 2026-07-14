//! Durable ClickHouse-backed cursor (task 0064).
//!
//! Replaces [`super::StubFileCursor`] in production. The ledger-sequence
//! checkpoint lives in a `prices.ingest_cursor` row (one per consumer `id`)
//! instead of an ephemeral `/tmp` file, so the doorbell-cursor reconcile loop
//! resumes across Lambda execution-environment recycles instead of rewinding to
//! the static `INITIAL_CURSOR` seed on every cold start (the freeze bug 0064
//! removes).
//!
//! The table is created by `schema/init.sql` (applied out-of-band, like every
//! other `prices.*` table); this cursor only reads and writes rows — it never
//! applies schema — so the table MUST exist before the processor starts.

use clickhouse::Client;

use super::{Cursor, CursorError};

/// Cursor persisted to `prices.ingest_cursor`, keyed by a logical consumer `id`.
pub struct ClickHouseCursor {
    client: Client,
    id: String,
}

impl ClickHouseCursor {
    /// `id` is the logical consumer key (e.g. `"ledger-processor"`); its row in
    /// `prices.ingest_cursor` is independent of any other consumer's. The
    /// `client` is any configured `clickhouse::Client` — the mTLS remote client
    /// in the Lambda, or a plaintext local client in tests.
    pub fn new(client: Client, id: impl Into<String>) -> Self {
        Self {
            client,
            id: id.into(),
        }
    }
}

impl Cursor for ClickHouseCursor {
    async fn read(&self) -> Result<u64, CursorError> {
        // FINAL collapses the ReplacingMergeTree(updated_at) rows to the latest
        // write even before a background merge. `None` = no cursor row yet (a
        // genuine first run) → surfaced as a Read error so `main`'s seed path
        // fires exactly once. A missing table also errors here (fatal at init,
        // matching the pool_registry "must exist" contract), which is correct —
        // the schema must be applied before the processor is deployed.
        let stored = self
            .client
            .query("SELECT ledger FROM prices.ingest_cursor FINAL WHERE id = ?")
            .bind(&self.id)
            .fetch_optional::<u64>()
            .await
            .map_err(|e| CursorError::Read(e.to_string()))?;
        stored.ok_or_else(|| CursorError::Read(format!("no cursor row for id '{}'", self.id)))
    }

    async fn write(&self, value: u64) -> Result<(), CursorError> {
        // `id` is a fixed constant (no injection surface) and `value` is numeric,
        // so string interpolation is safe here (same pattern as the backfill's
        // `backfill_progress` write). `updated_at` is server-side `now64(3)` —
        // monotonic across the strictly serial reconcile runs
        // (reservedConcurrency = 1), so the FINAL read always returns this write.
        let sql = format!(
            "INSERT INTO prices.ingest_cursor (id, ledger, updated_at) \
             VALUES ('{id}', {value}, now64(3))",
            id = self.id,
        );
        self.client
            .query(&sql)
            .execute()
            .await
            .map_err(|e| CursorError::Write(e.to_string()))
    }
}
