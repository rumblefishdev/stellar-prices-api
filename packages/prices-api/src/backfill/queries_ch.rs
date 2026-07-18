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
                    formatDateTime(earliest_data_available, '%Y-%m-%dT%H:%i:%SZ')) AS earliest_data_available \
               FROM backfill_progress FINAL \
               ORDER BY task_name";
    ch.query(sql).fetch_all::<ProgressRow>().await
}
