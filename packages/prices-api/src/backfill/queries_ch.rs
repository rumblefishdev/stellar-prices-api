//! ClickHouse query layer for `/v1/backfill/status`.

use clickhouse::Client;

/// Raw progress row (one per stream), datetimes pre-formatted / nullable.
#[derive(Debug, clickhouse::Row, serde::Deserialize)]
pub struct ProgressRow {
    pub task_name: String,
    pub start_ledger: u64,
    pub target_ledger: u64,
    pub current_ledger: u64,
    pub status: String,
    pub last_push_at: Option<String>,
    pub completed_at: Option<String>,
    pub earliest_data_available: Option<String>,
    /// Seconds since `last_push_at`, evaluated **server-side** in ClickHouse.
    ///
    /// `NULL` when the stream has never pushed. Computed in SQL rather than
    /// from the formatted string so it is immune to clock skew between the
    /// Lambda and ClickHouse — the same reasoning `backfill-freshness-probe`
    /// applies to its `PushAgeSeconds` metric, and the two must not disagree
    /// about whether a stream is stalled.
    ///
    /// ⚠️ The column is **table-qualified** in the SQL. `last_push_at` is also
    /// the alias of the formatted `String` projection above it, and an
    /// unqualified reference resolves to that alias, not the `DateTime` column
    /// — `dateDiff` then fails with `Code: 43 ILLEGAL_TYPE_OF_ARGUMENT` at
    /// runtime. The unit tests cannot catch it; only a query against a real
    /// ClickHouse can.
    pub push_age_seconds: Option<i64>,
    /// Newest ledger the live processor has durably committed
    /// (`prices.ingest_cursor`), carried on every row by a scalar subquery.
    ///
    /// `0` when the cursor table is empty — a fresh deployment before the first
    /// batch. This is the real chain tip; the SDEX `target_ledger` is only the
    /// tip as it stood when the backfill last pushed, which on 2026-09-08 was
    /// 534,222 ledgers behind (task 0176).
    ///
    /// 🔴 **`assumeNotNull` is load-bearing.** A ClickHouse *scalar subquery* is
    /// `Nullable` regardless of what it selects — `ifNull(max(ledger), 0)` still
    /// types as `Nullable(UInt64)`. Deserialised into a plain `u64`, RowBinary
    /// reads the null-flag byte as data, the row drifts one byte, and the NEXT
    /// row's `task_name` fails with `string is not valid utf8`. That took
    /// `/backfill/status` down on 2026-09-08 for the minutes between deploy and
    /// this fix.
    ///
    /// ⚠️ A `FORMAT TSVWithNames` query over HTTP does **not** reproduce it —
    /// text formats carry no null-flag byte, so the SQL looks perfectly healthy
    /// when checked by hand. Only the RowBinary path the client uses shows it.
    /// `backfill-freshness-probe` documents the same trap on its own age column.
    pub live_tip_ledger: u64,
}

/// Fetch all backfill-progress rows (latest per stream via `FINAL`).
pub async fn all_progress(ch: &Client) -> Result<Vec<ProgressRow>, clickhouse::error::Error> {
    let sql = "SELECT \
                 task_name, \
                 start_ledger, \
                 target_ledger, \
                 current_ledger, \
                 toString(status) AS status, \
                 if(isNull(last_push_at), NULL, \
                    formatDateTime(last_push_at, '%Y-%m-%dT%H:%i:%SZ')) AS last_push_at, \
                 if(isNull(completed_at), NULL, \
                    formatDateTime(completed_at, '%Y-%m-%dT%H:%i:%SZ')) AS completed_at, \
                 if(isNull(earliest_data_available), NULL, \
                    formatDateTime(earliest_data_available, '%Y-%m-%dT%H:%i:%SZ')) AS earliest_data_available, \
                 if(isNull(backfill_progress.last_push_at), NULL, \
                    toInt64(dateDiff('second', \
                      backfill_progress.last_push_at, now()))) AS push_age_seconds, \
                 assumeNotNull( \
                   (SELECT ifNull(max(ledger), 0) FROM ingest_cursor FINAL)) \
                   AS live_tip_ledger \
               FROM backfill_progress FINAL \
               ORDER BY task_name";
    ch.query(sql).fetch_all::<ProgressRow>().await
}
