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
    pub push_age_seconds: Option<i64>,
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
                 if(isNull(last_push_at), NULL, \
                    toInt64(dateDiff('second', last_push_at, now()))) AS push_age_seconds \
               FROM backfill_progress FINAL \
               ORDER BY task_name";
    ch.query(sql).fetch_all::<ProgressRow>().await
}
