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
        // FINAL collapses the ReplacingMergeTree(ledger) rows to the highest
        // ledger for this id (the latest checkpoint) even before a background
        // merge. Two distinct outcomes, kept distinct on purpose:
        //   - query error (transient CH failure, or a MISSING table) → `Read`,
        //     which the caller must NOT treat as "seed me" — seeding on a
        //     transient read would clobber a healthy cursor with the floor.
        //   - query ok but 0 rows (`None`) → `Empty`, the genuine first-run
        //     signal the caller seeds on.
        let stored = self
            .client
            .query("SELECT ledger FROM prices.ingest_cursor FINAL WHERE id = ?")
            .bind(&self.id)
            .fetch_optional::<u64>()
            .await
            .map_err(|e| CursorError::Read(e.to_string()))?;
        stored.ok_or(CursorError::Empty)
    }

    async fn write(&self, value: u64) -> Result<(), CursorError> {
        // `id` is a fixed constant (no injection surface) and `value` is numeric,
        // so string interpolation is safe here (same pattern as the backfill's
        // `backfill_progress` write). The RMT version is `ledger`, so FINAL keeps
        // the HIGHEST ledger written for this id — the cursor is monotonic-forward
        // and a stray lower write (e.g. a spurious re-seed) can never rewind it.
        // `updated_at` (now64(3)) is informational only: last-written wall time.
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
