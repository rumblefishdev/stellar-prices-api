//! ClickHouse query layer for `/v1/oracles`.

use clickhouse::Client;

use crate::oracles::dto::OracleEntry;

/// Latest reading per oracle for `asset_id`. `argMax(price_usd, timestamp)`
/// picks the most-recent price per `oracle_name` (the ReplacingMergeTree dedup
/// collapses identical (asset, oracle, timestamp) samples). Empty vec when the
/// asset has no oracle rows (table is retention-capped).
pub async fn oracles_for_asset(
    ch: &Client,
    asset_id: u32,
) -> Result<Vec<OracleEntry>, clickhouse::error::Error> {
    let sql = "SELECT \
                 oracle_name AS name, \
                 toString(argMax(price_usd, timestamp)) AS price_usd, \
                 formatDateTime(max(timestamp), '%Y-%m-%dT%H:%i:%SZ') AS updated_at \
               FROM oracle_prices \
               WHERE asset_id = ? \
               GROUP BY oracle_name \
               ORDER BY oracle_name";
    ch.query(sql)
        .bind(asset_id)
        .fetch_all::<OracleEntry>()
        .await
}
