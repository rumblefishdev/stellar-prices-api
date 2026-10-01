//! Task 0139 migration tool, part 1: preflight, map, create, fill, check.
//!
//! Re-keys the 11 asset-id tables from the old UInt32 counter ids to the ids
//! ClickHouse derives from the identity ([`crate::asset_id`]), by copy.
//!
//! - `map` classifies every old id from the RAW `assets` rows: `mapped` (one
//!   identity), `colliding` (two or more: its rows are blends and are not
//!   copied), `orphan` (in a fact table, not in `assets`), `sentinel` (0 → 0,
//!   the REDSTONE oracle rows). Two old ids of one identity map to one new id
//!   (D6). Only `mapped` and `sentinel` rows are copied.
//! - Every write is gated, logged to `rekey_0139_log`, and re-runnable.
//! - Without `execute` the tool prints the SQL it would run and writes nothing.

use clickhouse::Client;

use crate::asset_id::{id_expr, id_of};

/// Old id → new id, one row per (old id, identity).
pub const MAP_TABLE: &str = "asset_id_map_0139";
/// One row per step, partition and attempt.
pub const LOG_TABLE: &str = "rekey_0139_log";
/// Per month, the `price_ohlcv_1m` rows `fill` did not copy (task 0139's
/// re-ingest list).
pub const MONTHS_TABLE: &str = "rekey_0139_reingest_months";

pub const STATUS_MAPPED: &str = "mapped";
pub const STATUS_COLLIDING: &str = "colliding";
pub const STATUS_ORPHAN: &str = "orphan";
pub const STATUS_SENTINEL: &str = "sentinel";
/// The statuses whose rows are copied, as an SQL tuple.
pub const COPYABLE: &str = "('mapped', 'sentinel')";

/// The refusal `map` gives once `assets` holds derived ids: a re-run would
/// read new ids as old ones.
pub const MAP_FINAL: &str = "assets already carry derived ids; the map is final";

/// The id-keyed tables copied to `X__new` (all but `assets`, migrated in place).
pub const COPIED_TABLES: [&str; 11] = [
    "price_ohlcv_1m",
    "price_ohlcv_15m",
    "price_ohlcv_1h",
    "price_ohlcv_4h",
    "price_ohlcv_1d",
    "price_ohlcv_1w",
    "price_ohlcv_1M",
    "current_prices",
    "asset_supply",
    "asset_metadata",
    "oracle_prices",
];

#[derive(Debug, thiserror::Error)]
pub enum RekeyError {
    #[error("clickhouse: {0}")]
    Query(#[from] clickhouse::error::Error),
    #[error("refused: {0}")]
    Refused(String),
    #[error("gate failed: {0}")]
    Gate(String),
}

type Result<T> = std::result::Result<T, RekeyError>;

/// The id columns of a copied table.
pub fn id_columns(table: &str) -> &'static [&'static str] {
    if table.starts_with("price_ohlcv_") {
        &["asset_id", "quote_asset_id"]
    } else {
        &["asset_id"]
    }
}

/// DDL of the tool's own tables.
pub fn tool_tables_ddl(db: &str) -> [String; 3] {
    [
        format!(
            "CREATE TABLE IF NOT EXISTS {db}.{MAP_TABLE} (old_id UInt32, new_id UInt64, \
             asset_code String, issuer_address String, contract_address String, \
             status LowCardinality(String), classified_at DateTime DEFAULT now()) \
             ENGINE = MergeTree ORDER BY (old_id, asset_code, issuer_address, contract_address)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {db}.{LOG_TABLE} (at DateTime64(3) DEFAULT now64(3), \
             step LowCardinality(String), source String DEFAULT '', target String DEFAULT '', \
             partition String DEFAULT '', status LowCardinality(String), \
             query_id String DEFAULT '', written UInt64 DEFAULT 0, expected UInt64 DEFAULT 0, \
             expected_keys UInt64 DEFAULT 0, target_keys UInt64 DEFAULT 0, \
             fingerprint String DEFAULT '', detail String DEFAULT '', user String DEFAULT '') \
             ENGINE = MergeTree ORDER BY (step, target, partition, at)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {db}.{MONTHS_TABLE} (month UInt32, \
             colliding_rows UInt64, orphan_rows UInt64, computed_at DateTime DEFAULT now()) \
             ENGINE = MergeTree ORDER BY month"
        ),
    ]
}

/// The map's rows, as a SELECT over raw `assets` (no FINAL) and the id
/// columns of the copied tables.
pub fn map_select_sql(db: &str) -> String {
    let fact_ids = COPIED_TABLES
        .iter()
        .flat_map(|t| {
            id_columns(t)
                .iter()
                .map(move |c| format!("SELECT {c} AS id FROM {db}.{t}"))
        })
        .collect::<Vec<_>>()
        .join(" UNION DISTINCT ");
    format!(
        "SELECT old_id, {new_id} AS new_id, asset_code, issuer_address, contract_address, \
         if(n > 1, '{STATUS_COLLIDING}', '{STATUS_MAPPED}') AS status \
         FROM (SELECT asset_id AS old_id, asset_code, issuer_address, contract_address, \
         count() OVER (PARTITION BY asset_id) AS n \
         FROM (SELECT DISTINCT asset_id, asset_code, issuer_address, contract_address \
         FROM {db}.assets)) \
         UNION ALL \
         SELECT toUInt32(id), toUInt64(0), '', '', '', '{STATUS_ORPHAN}' \
         FROM ({fact_ids}) WHERE id != 0 AND id NOT IN (SELECT asset_id FROM {db}.assets) \
         UNION ALL \
         SELECT toUInt32(0), toUInt64(0), '', '', '', '{STATUS_SENTINEL}'",
        new_id = id_expr("asset_code", "issuer_address", "contract_address"),
    )
}

/// The map gates: each SELECT returns its number of violations.
pub fn map_gates_sql(db: &str, table: &str) -> [(&'static str, String); 3] {
    let t = format!("{db}.{table}");
    [
        (
            "a mapped new_id is 0 or the blank identity's id",
            format!(
                "SELECT toInt64(count()) FROM {t} WHERE status = '{STATUS_MAPPED}' \
                 AND (new_id = 0 OR new_id = {})",
                id_of("", "", "")
            ),
        ),
        (
            "two identities share a new_id",
            format!(
                "SELECT toInt64(uniqExact(asset_code, issuer_address, contract_address)) \
                 - toInt64(uniqExact(new_id)) FROM {t} \
                 WHERE status IN ('{STATUS_MAPPED}', '{STATUS_COLLIDING}')"
            ),
        ),
        (
            "an old_id has more than one copyable row",
            format!(
                "SELECT toInt64(count()) FROM (SELECT old_id FROM {t} \
                 WHERE status IN {COPYABLE} GROUP BY old_id HAVING count() > 1)"
            ),
        ),
    ]
}

/// One `rekey_0139_log` row.
#[derive(Debug, Default)]
pub(crate) struct Log<'a> {
    pub step: &'a str,
    pub source: &'a str,
    pub target: &'a str,
    pub partition: &'a str,
    pub status: &'a str,
    pub query_id: &'a str,
    pub written: u64,
    pub expected: u64,
    pub expected_keys: u64,
    pub target_keys: u64,
    pub fingerprint: &'a str,
    pub detail: &'a str,
}

/// `INSERT … SELECT`, not `VALUES`: under `async_insert=1` (prod's profile) a
/// `DEFAULT currentUser()` is evaluated at flush and stores ''.
fn log_sql(db: &str, l: &Log<'_>) -> String {
    use crate::asset_id::sql_str as s;
    format!(
        "INSERT INTO {db}.{LOG_TABLE} (at, step, source, target, partition, status, query_id, \
         written, expected, expected_keys, target_keys, fingerprint, detail, user) \
         SELECT now64(3), {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, currentUser()",
        s(l.step),
        s(l.source),
        s(l.target),
        s(l.partition),
        s(l.status),
        s(l.query_id),
        l.written,
        l.expected,
        l.expected_keys,
        l.target_keys,
        s(l.fingerprint),
        s(l.detail),
    )
}

/// The tool against one database. Reads always run; writes run only with
/// `execute`, and are otherwise printed.
pub struct Rekey {
    client: Client,
    db: String,
    execute: bool,
}

impl Rekey {
    pub fn new(client: Client, db: impl Into<String>, execute: bool) -> Self {
        Self {
            client,
            db: db.into(),
            execute,
        }
    }

    fn t(&self, name: &str) -> String {
        format!("{}.{name}", self.db)
    }

    async fn write(&self, sql: &str) -> Result<()> {
        if self.execute {
            self.client.query(sql).execute().await?;
        } else {
            println!("{sql};");
        }
        Ok(())
    }

    async fn log(&self, l: Log<'_>) -> Result<()> {
        self.write(&log_sql(&self.db, &l)).await
    }

    async fn ensure_tool_tables(&self) -> Result<()> {
        for ddl in tool_tables_ddl(&self.db) {
            self.write(&ddl).await?;
        }
        Ok(())
    }

    async fn exists(&self, table: &str) -> Result<bool> {
        let n: u64 = self
            .client
            .query("SELECT count() FROM system.tables WHERE database = ? AND name = ?")
            .bind(&self.db)
            .bind(table)
            .fetch_one()
            .await?;
        Ok(n > 0)
    }

    /// `(name, type)` of `table`'s `asset_id` / `quote_asset_id` columns.
    async fn id_types(&self, table: &str) -> Result<Vec<(String, String)>> {
        Ok(self
            .client
            .query(
                "SELECT name, type FROM system.columns WHERE database = ? AND table = ? \
                 AND name IN ('asset_id', 'quote_asset_id') ORDER BY name",
            )
            .bind(&self.db)
            .bind(table)
            .fetch_all()
            .await?)
    }

    async fn logged(&self, step: &str) -> Result<u64> {
        if !self.exists(LOG_TABLE).await? {
            return Ok(0);
        }
        Ok(self
            .client
            .query(&format!(
                "SELECT count() FROM {} WHERE step = ?",
                self.t(LOG_TABLE)
            ))
            .bind(step)
            .fetch_one()
            .await?)
    }

    /// `map` refuses once `assets` holds derived ids, or `alter-assets` ran.
    async fn refuse_if_map_final(&self) -> Result<()> {
        let uint64 = self
            .id_types("assets")
            .await?
            .iter()
            .any(|(_, ty)| ty == "UInt64");
        if uint64 || self.logged("alter-assets").await? > 0 {
            return Err(RekeyError::Refused(MAP_FINAL.into()));
        }
        Ok(())
    }

    /// Run the map gates on `table`; every failing gate is named.
    pub async fn map_gates(&self, table: &str) -> Result<()> {
        let mut failed = Vec::new();
        for (name, sql) in map_gates_sql(&self.db, table) {
            let n: i64 = self.client.query(&sql).fetch_one().await?;
            if n != 0 {
                failed.push(format!("{name} ({n})"));
            }
        }
        if failed.is_empty() {
            Ok(())
        } else {
            Err(RekeyError::Gate(format!("map: {}", failed.join("; "))))
        }
    }

    /// `status: ids / rows` per status of a map-shaped relation.
    async fn status_summary(&self, from: &str) -> Result<String> {
        let rows: Vec<(String, u64, u64)> = self
            .client
            .query(&format!(
                "SELECT status, uniqExact(old_id), count() FROM {from} \
                 GROUP BY status ORDER BY status"
            ))
            .fetch_all()
            .await?;
        Ok(rows
            .iter()
            .map(|(s, ids, n)| format!("{s}: {ids} ids / {n} rows"))
            .collect::<Vec<_>>()
            .join(", "))
    }

    /// Build the map in a side table, gate it, then swap it in. A re-run
    /// rebuilds it whole: new old ids appear, and an id that gained a second
    /// identity becomes `colliding`. Returns the per-status summary.
    pub async fn map(&self) -> Result<String> {
        self.refuse_if_map_final().await?;
        let map = self.t(MAP_TABLE);
        let build = self.t(&format!("{MAP_TABLE}__build"));
        let select = map_select_sql(&self.db);
        if !self.execute {
            self.ensure_tool_tables().await?;
            self.write(&format!("INSERT INTO {build} {select}")).await?;
            return self.status_summary(&format!("({select})")).await;
        }
        self.ensure_tool_tables().await?;
        self.write(&format!("DROP TABLE IF EXISTS {build} SYNC"))
            .await?;
        self.write(&format!("CREATE TABLE {build} AS {map}"))
            .await?;
        self.write(&format!(
            "INSERT INTO {build} (old_id, new_id, asset_code, issuer_address, \
             contract_address, status) {select}"
        ))
        .await?;
        // On failure the build table stays for inspection; the map is untouched.
        self.map_gates(&format!("{MAP_TABLE}__build")).await?;
        self.write(&format!("EXCHANGE TABLES {map} AND {build}"))
            .await?;
        self.write(&format!("DROP TABLE {build} SYNC")).await?;
        let summary = self.status_summary(&map).await?;
        self.log(Log {
            step: "map",
            status: "ok",
            detail: &summary,
            ..Log::default()
        })
        .await?;
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_map_is_built_from_raw_assets_with_the_schema_expression() {
        let sql = map_select_sql("db");
        assert!(sql.contains(&id_expr("asset_code", "issuer_address", "contract_address")));
        assert!(
            sql.contains("FROM db.assets))"),
            "raw assets, no FINAL: {sql}"
        );
        assert!(!sql.contains("FINAL"));
        for t in COPIED_TABLES {
            assert!(
                sql.contains(&format!("SELECT asset_id AS id FROM db.{t}")),
                "{t}"
            );
        }
        assert!(sql.contains("SELECT quote_asset_id AS id FROM db.price_ohlcv_1M"));
        assert!(!sql.contains("quote_asset_id AS id FROM db.oracle_prices"));
        assert!(sql.contains("'sentinel'"));
    }

    #[test]
    fn the_map_gates_check_the_blank_id_hash_uniqueness_and_one_copy_per_old_id() {
        let gates = map_gates_sql("db", "m");
        assert!(gates[0].1.contains(&id_of("", "", "")));
        assert!(gates[1].1.contains("'mapped', 'colliding'"));
        assert!(gates[2].1.contains(&format!("status IN {COPYABLE}")));
    }

    #[test]
    fn a_log_row_escapes_its_strings() {
        let sql = log_sql(
            "db",
            &Log {
                step: "map",
                detail: "it's",
                ..Log::default()
            },
        );
        assert!(sql.contains(r"'it\'s'"), "{sql}");
    }
}
