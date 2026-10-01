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

/// `fill` reads `written_rows` from `system.query_log`; without it no
/// partition can be verified.
pub const NO_QUERY_LOG: &str = "system.query_log does not exist: fill cannot verify written_rows";

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

    async fn count(&self, sql: &str) -> Result<u64> {
        Ok(self.client.query(sql).fetch_one().await?)
    }

    /// `system.query_log` exists once the server logs queries.
    async fn has_query_log(&self) -> Result<bool> {
        self.client.query("SYSTEM FLUSH LOGS").execute().await?;
        Ok(self
            .count(
                "SELECT count() FROM system.tables WHERE database = 'system' \
                 AND name = 'query_log'",
            )
            .await?
            > 0)
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

/// The table a schema statement creates or alters, if it is `prices.<name>`.
fn statement_table(stmt: &str) -> Option<&str> {
    let rest = stmt
        .strip_prefix("CREATE TABLE IF NOT EXISTS prices.")
        .or_else(|| stmt.strip_prefix("ALTER TABLE prices."))?;
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// `prices.<name>` → `<db>.<name>__new` for a copied table, `<db>.<name>`
/// otherwise.
pub fn rename_for_new(stmt: &str, db: &str) -> String {
    let mut out = String::with_capacity(stmt.len());
    let mut rest = stmt;
    while let Some(at) = rest.find("prices.") {
        out.push_str(&rest[..at]);
        let tail = &rest[at + "prices.".len()..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(tail.len());
        let name = &tail[..end];
        let suffix = if COPIED_TABLES.contains(&name) {
            "__new"
        } else {
            ""
        };
        out.push_str(&format!("{db}.{name}{suffix}"));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// The `INIT_SQL` statements that build the eleven `X__new` tables in `db`.
pub fn create_sql(db: &str) -> Vec<String> {
    crate::split_statements(crate::INIT_SQL)
        .into_iter()
        .filter(|s| statement_table(s).is_some_and(|t| COPIED_TABLES.contains(&t)))
        .map(|s| rename_for_new(&s, db))
        .collect()
}

/// Stored columns `(name, type)` of `X` and `X__new` must agree except for
/// the id columns' types.
pub fn parity(
    table: &str,
    live: &[(String, String)],
    new: &[(String, String)],
) -> std::result::Result<(), String> {
    let find = |cols: &[(String, String)], n: &str| {
        cols.iter().find(|(c, _)| c == n).map(|(_, t)| t.clone())
    };
    let mut bad = Vec::new();
    for (name, ty) in live {
        match find(new, name) {
            None => bad.push(format!("{table}__new lacks {name}")),
            Some(t) if t != *ty && !["asset_id", "quote_asset_id"].contains(&name.as_str()) => {
                bad.push(format!("{name} is {ty} in {table}, {t} in {table}__new"))
            }
            Some(_) => {}
        }
    }
    for (name, _) in new {
        if find(live, name).is_none() {
            bad.push(format!("{table} lacks {name}"));
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.join(", "))
    }
}

/// One fill: `source` (old ids) into `target` (new ids) through the map.
#[derive(Debug, Clone, Default)]
pub struct Fill {
    pub source: String,
    pub target: String,
    /// Only this partition (its value or its id).
    pub partition: Option<String>,
    /// Test hooks: an extra predicate on the INSERT only, and a column whose
    /// copied expression is replaced.
    #[doc(hidden)]
    pub fault: Option<String>,
    #[doc(hidden)]
    pub fault_expr: Option<(String, String)>,
}

impl Fill {
    /// The default fill of a copied table: `X` into `X__new`.
    pub fn table(t: &str) -> Self {
        Self {
            source: t.into(),
            target: format!("{t}__new"),
            ..Self::default()
        }
    }
}

/// Partitions copied (or, dry, to copy) and skipped as unchanged.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FillReport {
    pub copied: usize,
    pub skipped: usize,
}

type LastFill = (String, String, u64, u64, u64);

/// What a fill needs to know about its two tables.
#[derive(Debug, Clone)]
pub struct Layout {
    pub partition_key: String,
    /// The target's sorting key columns.
    pub key: Vec<String>,
    pub source_cols: Vec<String>,
    pub target_cols: Vec<String>,
    /// The id columns, `asset_id` first.
    pub ids: Vec<String>,
}

fn id_alias(col: &str) -> &'static str {
    if col == "quote_asset_id" { "q" } else { "m" }
}

fn mapped_col(col: &str, ids: &[String]) -> String {
    if ids.iter().any(|i| i == col) {
        format!("{}.new_id", id_alias(col))
    } else {
        format!("s.{col}")
    }
}

fn joins(db: &str, ids: &[String], kind: &str) -> String {
    ids.iter()
        .map(|c| {
            let a = id_alias(c);
            format!(
                " {kind} JOIN (SELECT old_id, new_id, 1 AS ok FROM {db}.{MAP_TABLE} \
                 WHERE status IN {COPYABLE}) AS {a} ON {a}.old_id = s.{c}"
            )
        })
        .collect()
}

/// The predicate selecting one partition.
pub fn partition_where(partition_key: &str, partition: &str) -> String {
    if partition_key.is_empty() {
        "1".into()
    } else {
        format!("{partition_key} = {partition}")
    }
}

/// One read of a source partition: raw rows, a fingerprint hash over every
/// stored column and the map's verdict on its ids, the rows `fill` copies,
/// and their distinct target keys.
pub fn probe_sql(db: &str, source: &str, l: &Layout, filter: &str) -> String {
    let hashed = l
        .source_cols
        .iter()
        .map(|c| format!("s.{c}"))
        .chain(l.ids.iter().flat_map(|c| {
            let a = id_alias(c);
            [format!("{a}.new_id"), format!("{a}.ok")]
        }))
        .collect::<Vec<_>>()
        .join(", ");
    let copied = l
        .ids
        .iter()
        .map(|c| format!("{}.ok = 1", id_alias(c)))
        .collect::<Vec<_>>()
        .join(" AND ");
    let key = l
        .key
        .iter()
        .map(|c| mapped_col(c, &l.ids))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT count(), sum(cityHash64({hashed})), countIf({copied}), \
         uniqExactIf(tuple({key}), {copied}) FROM {db}.{source} AS s{} WHERE {filter}",
        joins(db, &l.ids, "LEFT")
    )
}

/// Copy the mapped rows of one source partition.
pub fn insert_sql(db: &str, f: &Fill, l: &Layout, filter: &str) -> String {
    let fault = f
        .fault
        .as_deref()
        .map(|p| format!(" AND ({p})"))
        .unwrap_or_default();
    format!(
        "INSERT INTO {db}.{} ({}) SELECT {} FROM {db}.{} AS s{} WHERE {filter}{fault}",
        f.target,
        l.target_cols.join(", "),
        l.target_cols
            .iter()
            .map(|c| match &f.fault_expr {
                Some((col, expr)) if col == c => expr.clone(),
                _ => mapped_col(c, &l.ids),
            })
            .collect::<Vec<_>>()
            .join(", "),
        f.source,
        joins(db, &l.ids, "INNER"),
    )
}

/// Distinct sorting keys in a target partition: unchanged by merges that
/// collapse duplicate keys, which a `count()` is not.
pub fn target_keys_sql(db: &str, target: &str, l: &Layout, filter: &str) -> String {
    format!(
        "SELECT uniqExact(tuple({})) FROM {db}.{target} WHERE {filter}",
        l.key.join(", ")
    )
}

/// `written_rows` of a finished query from `system.query_log`, after
/// `SYSTEM FLUSH LOGS`; `None` if the row never appears.
pub async fn written_rows(
    client: &Client,
    query_id: &str,
    attempts: u32,
    delay: std::time::Duration,
) -> Result<Option<u64>> {
    for i in 0..attempts {
        if i > 0 {
            tokio::time::sleep(delay).await;
        }
        client.query("SYSTEM FLUSH LOGS").execute().await?;
        let n: Option<u64> = client
            .query(
                "SELECT written_rows FROM system.query_log WHERE event_date >= yesterday() \
                 AND query_id = ? AND type = 'QueryFinish' \
                 ORDER BY event_time_microseconds DESC LIMIT 1",
            )
            .bind(query_id)
            .fetch_optional()
            .await?;
        if n.is_some() {
            return Ok(n);
        }
    }
    Ok(None)
}

impl Rekey {
    /// `(name, type)` of the stored columns, in table order.
    async fn stored_columns(&self, table: &str) -> Result<Vec<(String, String)>> {
        Ok(self
            .client
            .query(
                "SELECT name, type FROM system.columns WHERE database = ? AND table = ? \
                 AND default_kind NOT IN ('EPHEMERAL', 'ALIAS', 'MATERIALIZED') \
                 ORDER BY position",
            )
            .bind(&self.db)
            .bind(table)
            .fetch_all()
            .await?)
    }

    /// Build `X__new` for the eleven copied tables from `INIT_SQL`, then gate
    /// stored-column parity against `X`.
    pub async fn create(&self) -> Result<()> {
        for stmt in create_sql(&self.db) {
            self.write(&stmt).await?;
        }
        if !self.execute {
            return Ok(());
        }
        let mut bad = Vec::new();
        for t in COPIED_TABLES {
            let live = self.stored_columns(t).await?;
            let new = self.stored_columns(&format!("{t}__new")).await?;
            if let Err(e) = parity(t, &live, &new) {
                bad.push(e);
            }
        }
        if !bad.is_empty() {
            return Err(RekeyError::Gate(format!("create: {}", bad.join("; "))));
        }
        self.ensure_tool_tables().await?;
        self.log(Log {
            step: "create",
            status: "ok",
            ..Log::default()
        })
        .await
    }

    /// Fill refuses a source already in the new id space, a target that is
    /// not, and a missing or failing map.
    async fn refuse_fill(&self, f: &Fill) -> Result<()> {
        self.refuse_ids(f).await?;
        if !self.exists(MAP_TABLE).await?
            || self
                .count(&format!("SELECT count() FROM {}", self.t(MAP_TABLE)))
                .await?
                == 0
        {
            return Err(RekeyError::Refused("no map (run map)".into()));
        }
        if !self.has_query_log().await? {
            return Err(RekeyError::Refused(NO_QUERY_LOG.into()));
        }
        self.map_gates(MAP_TABLE).await
    }

    /// The source must hold UInt32 ids and the target UInt64 ids.
    async fn refuse_ids(&self, f: &Fill) -> Result<()> {
        let src = self.id_types(&f.source).await?;
        if src.is_empty() {
            return Err(RekeyError::Refused(format!(
                "{} has no asset id column",
                f.source
            )));
        }
        if src.iter().any(|(_, t)| t != "UInt32") {
            return Err(RekeyError::Refused(format!(
                "{} is already in the new id space (UInt64 ids)",
                f.source
            )));
        }
        let tgt = self.id_types(&f.target).await?;
        if tgt.len() != src.len() || tgt.iter().any(|(_, t)| t != "UInt64") {
            return Err(RekeyError::Refused(format!(
                "{} does not hold UInt64 ids (run create)",
                f.target
            )));
        }
        Ok(())
    }

    async fn layout(&self, f: &Fill) -> Result<Layout> {
        let keys = |t: &str| {
            self.client
                .query(
                    "SELECT partition_key, sorting_key FROM system.tables \
                     WHERE database = ? AND name = ?",
                )
                .bind(&self.db)
                .bind(t.to_string())
                .fetch_one::<(String, String)>()
        };
        let (src_pk, _) = keys(&f.source).await?;
        let (pk, sk) = keys(&f.target).await?;
        if src_pk != pk {
            return Err(RekeyError::Refused(format!(
                "{} is partitioned by `{src_pk}`, {} by `{pk}`",
                f.source, f.target
            )));
        }
        let names = |cols: Vec<(String, String)>| cols.into_iter().map(|(n, _)| n).collect();
        let target_cols: Vec<String> = names(self.stored_columns(&f.target).await?);
        let ids = ["asset_id", "quote_asset_id"]
            .iter()
            .filter(|c| target_cols.iter().any(|t| t == *c))
            .map(|c| c.to_string())
            .collect();
        Ok(Layout {
            partition_key: pk,
            key: sk.split(", ").map(str::to_string).collect(),
            source_cols: names(self.stored_columns(&f.source).await?),
            target_cols,
            ids,
        })
    }

    /// `(status, fingerprint, expected_keys, written, expected)` of the last
    /// fill of a partition.
    async fn last_fill(&self, f: &Fill, pid: &str) -> Result<Option<LastFill>> {
        if !self.exists(LOG_TABLE).await? {
            return Ok(None);
        }
        Ok(self
            .client
            .query(&format!(
                "SELECT status, fingerprint, expected_keys, written, expected FROM {} \
                 WHERE step = 'fill' AND source = ? AND target = ? AND partition = ? \
                 ORDER BY at DESC LIMIT 1",
                self.t(LOG_TABLE)
            ))
            .bind(&f.source)
            .bind(&f.target)
            .bind(pid)
            .fetch_optional()
            .await?)
    }

    /// `(partition, partition_id)` of the active parts of `table`.
    async fn partitions(&self, table: &str) -> Result<Vec<(String, String)>> {
        Ok(self
            .client
            .query(
                "SELECT partition, partition_id FROM system.parts WHERE database = ? \
                 AND table = ? AND active GROUP BY partition, partition_id \
                 ORDER BY partition_id",
            )
            .bind(&self.db)
            .bind(table)
            .fetch_all()
            .await?)
    }

    /// `(fingerprint, expected rows, expected distinct keys)` of a source
    /// partition.
    async fn probe(&self, f: &Fill, l: &Layout, filter: &str) -> Result<(String, u64, u64)> {
        let (rows, hash, expected, keys): (u64, u64, u64, u64) = self
            .client
            .query(&probe_sql(&self.db, &f.source, l, filter))
            .fetch_one()
            .await?;
        Ok((format!("{rows}:{hash}"), expected, keys))
    }

    async fn target_keys(&self, f: &Fill, l: &Layout, filter: &str) -> Result<u64> {
        self.count(&target_keys_sql(&self.db, &f.target, l, filter))
            .await
    }

    /// Copy `f.source` into `f.target` partition by partition. A partition
    /// whose fingerprint and target keys still match its last verified fill
    /// is skipped; any other is dropped in the target and refilled. A copy is
    /// verified only if `written_rows` equals the expected rows and the
    /// target's distinct keys equal the source's mapped distinct keys.
    pub async fn fill(&self, f: &Fill) -> Result<FillReport> {
        self.refuse_fill(f).await?;
        self.ensure_tool_tables().await?;
        let l = self.layout(f).await?;
        let parts = self.partitions(&f.source).await?;
        let mut report = FillReport::default();
        let mut failed = Vec::new();
        for (partition, pid) in parts {
            if f.partition
                .as_ref()
                .is_some_and(|p| *p != partition && *p != pid)
            {
                continue;
            }
            let filter = partition_where(&l.partition_key, &partition);
            let (fingerprint, expected, expected_keys) = self.probe(f, &l, &filter).await?;
            if let Some((status, fp, keys, _, _)) = self.last_fill(f, &pid).await?
                && status == "verified"
                && fp == fingerprint
                && self.target_keys(f, &l, &filter).await? == keys
            {
                report.skipped += 1;
                continue;
            }
            report.copied += 1;
            let attempts: u64 = if self.exists(LOG_TABLE).await? {
                self.client
                    .query(&format!(
                        "SELECT count() FROM {} WHERE step = 'fill' AND target = ? \
                         AND partition = ?",
                        self.t(LOG_TABLE)
                    ))
                    .bind(&f.target)
                    .bind(&pid)
                    .fetch_one()
                    .await?
            } else {
                0
            };
            let query_id = format!("rekey0139-{}-{}-{pid}-{}", self.db, f.target, attempts + 1);
            self.write(&format!(
                "ALTER TABLE {} DROP PARTITION ID '{pid}'",
                self.t(&f.target)
            ))
            .await?;
            let insert = insert_sql(&self.db, f, &l, &filter);
            if !self.execute {
                println!("{insert}; -- query_id {query_id}");
                continue;
            }
            self.client
                .query(&insert)
                .with_option("query_id", &query_id)
                .execute()
                .await?;
            let written = written_rows(
                &self.client,
                &query_id,
                5,
                std::time::Duration::from_secs(2),
            )
            .await?;
            let keys = self.target_keys(f, &l, &filter).await?;
            let verified = written == Some(expected) && keys == expected_keys;
            if !verified {
                failed.push(format!(
                    "{}/{pid}: written {written:?} of {expected}, keys {keys} of {expected_keys}",
                    f.target
                ));
            }
            self.log(Log {
                step: "fill",
                source: &f.source,
                target: &f.target,
                partition: &pid,
                status: if verified { "verified" } else { "failed" },
                query_id: &query_id,
                written: written.unwrap_or(0),
                expected,
                expected_keys,
                target_keys: keys,
                fingerprint: &fingerprint,
                detail: if written.is_none() {
                    "no QueryFinish row in system.query_log"
                } else {
                    ""
                },
            })
            .await?;
        }
        if failed.is_empty() {
            Ok(report)
        } else {
            Err(RekeyError::Gate(format!("fill: {}", failed.join("; "))))
        }
    }
}

/// Each source row's fate: `copied`, or why not (`colliding` before
/// `orphan`; `unmapped` means the map is older than the row).
pub fn classify_sql(db: &str, source: &str, ids: &[String], extra: &str) -> String {
    let st = |c: &str| format!("{}.st", id_alias(c));
    let any = |v: &str| {
        ids.iter()
            .map(|c| format!("{} = '{v}'", st(c)))
            .collect::<Vec<_>>()
            .join(" OR ")
    };
    let joins: String = ids
        .iter()
        .map(|c| {
            let a = id_alias(c);
            format!(
                " LEFT JOIN (SELECT old_id, any(status) AS st FROM {db}.{MAP_TABLE} \
                 GROUP BY old_id) AS {a} ON {a}.old_id = s.{c}"
            )
        })
        .collect();
    format!(
        "SELECT {extra}multiIf({}, 'unmapped', {}, '{STATUS_COLLIDING}', {}, '{STATUS_ORPHAN}', \
         'copied') AS cls FROM {db}.{source} AS s{joins}",
        any(""),
        any(STATUS_COLLIDING),
        any(STATUS_ORPHAN),
    )
}

/// The free-space gate: 1.2× the bytes to be copied.
pub fn disk_ok(free: u64, bytes: u64) -> bool {
    u128::from(free) * 5 >= u128::from(bytes) * 6
}

/// The server the tool was built and rehearsed against.
pub fn supported_version(v: &str) -> bool {
    v.starts_with("26.3.")
}

impl Rekey {
    /// Per table: every source partition's last fill verified, unchanged
    /// since, and its target keys intact; Σ written = Σ expected; excluded
    /// rows per status. Writes the per-month exclusions of `price_ohlcv_1m`
    /// to `rekey_0139_reingest_months`. Returns one line per table.
    pub async fn check(&self, fills: &[Fill]) -> Result<Vec<String>> {
        self.ensure_tool_tables().await?;
        let mut lines = Vec::new();
        let mut problems = Vec::new();
        for f in fills {
            self.refuse_ids(f).await?;
            let l = self.layout(f).await?;
            let (mut written, mut expected) = (0u64, 0u64);
            for (partition, pid) in self.partitions(&f.source).await? {
                let filter = partition_where(&l.partition_key, &partition);
                let at = format!("{}/{pid}", f.target);
                let Some((status, fp, keys, w, e)) = self.last_fill(f, &pid).await? else {
                    problems.push(format!("{at}: not filled"));
                    continue;
                };
                (written, expected) = (written + w, expected + e);
                let (fingerprint, _, _) = self.probe(f, &l, &filter).await?;
                let target_keys = self.target_keys(f, &l, &filter).await?;
                if status != "verified" {
                    problems.push(format!("{at}: last fill {status}"));
                } else if fp != fingerprint {
                    problems.push(format!("{at}: source changed since fill"));
                } else if target_keys != keys {
                    problems.push(format!("{at}: target keys {target_keys} of {keys}"));
                }
            }
            if written != expected {
                problems.push(format!(
                    "{}: written {written} != expected {expected}",
                    f.target
                ));
            }
            let classes: Vec<(String, u64)> = self
                .client
                .query(&format!(
                    "SELECT cls, count() FROM ({}) GROUP BY cls ORDER BY cls",
                    classify_sql(&self.db, &f.source, &l.ids, "")
                ))
                .fetch_all()
                .await?;
            let n = |c: &str| classes.iter().find(|(k, _)| k == c).map_or(0, |(_, n)| *n);
            if n("unmapped") > 0 {
                problems.push(format!(
                    "{}: {} rows under ids the map lacks (re-run map)",
                    f.source,
                    n("unmapped")
                ));
            }
            lines.push(format!(
                "{} -> {}: written {written} of {expected}; excluded colliding {}, orphan {}",
                f.source,
                f.target,
                n(STATUS_COLLIDING),
                n(STATUS_ORPHAN)
            ));
            if f.source == "price_ohlcv_1m" {
                self.write(&format!("TRUNCATE TABLE {}", self.t(MONTHS_TABLE)))
                    .await?;
                self.write(&format!(
                    "INSERT INTO {} (month, colliding_rows, orphan_rows, computed_at) \
                     SELECT month, countIf(cls = '{STATUS_COLLIDING}'), \
                     countIf(cls = '{STATUS_ORPHAN}'), now() FROM ({}) \
                     WHERE cls IN ('{STATUS_COLLIDING}', '{STATUS_ORPHAN}') \
                     GROUP BY month",
                    self.t(MONTHS_TABLE),
                    classify_sql(
                        &self.db,
                        &f.source,
                        &l.ids,
                        "toYYYYMM(s.timestamp) AS month, "
                    )
                ))
                .await?;
            }
        }
        self.log(Log {
            step: "check",
            status: if problems.is_empty() { "ok" } else { "failed" },
            detail: &lines.join("; "),
            ..Log::default()
        })
        .await?;
        if problems.is_empty() {
            Ok(lines)
        } else {
            Err(RekeyError::Gate(format!("check: {}", problems.join("; "))))
        }
    }

    /// Read-only: Atomic database, a 26.3 server with `query_log`, the twelve
    /// id tables (their id types reported), free disk ≥ 1.2× the copied
    /// tables' bytes, and the refreshable MVs' states.
    pub async fn preflight(&self) -> Result<Vec<String>> {
        let mut lines = Vec::new();
        let mut bad = Vec::new();
        let engine: Option<String> = self
            .client
            .query("SELECT engine FROM system.databases WHERE name = ?")
            .bind(&self.db)
            .fetch_optional()
            .await?;
        lines.push(format!("database {}: {engine:?}", self.db));
        if engine.as_deref() != Some("Atomic") {
            bad.push(format!(
                "database {} is not Atomic (EXCHANGE needs it)",
                self.db
            ));
        }
        let version: String = self.client.query("SELECT version()").fetch_one().await?;
        lines.push(format!("server {version}"));
        if !supported_version(&version) {
            bad.push(format!("server {version} is not 26.3"));
        }
        if !self.has_query_log().await? {
            bad.push(NO_QUERY_LOG.into());
        }
        for t in std::iter::once("assets").chain(COPIED_TABLES) {
            let ids = self.id_types(t).await?;
            if ids.is_empty() {
                bad.push(format!("{t} missing"));
            }
            let ids: Vec<String> = ids.iter().map(|(c, ty)| format!("{c} {ty}")).collect();
            lines.push(format!("{t}: {}", ids.join(", ")));
        }
        let tables = COPIED_TABLES
            .iter()
            .map(|t| format!("'{t}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let bytes = self
            .count(&format!(
                "SELECT sum(bytes_on_disk) FROM system.parts WHERE database = '{}' \
                 AND table IN ({tables}) AND active",
                self.db
            ))
            .await?;
        let free = self
            .count(&format!(
                "SELECT min(free_space) FROM system.disks WHERE name IN (SELECT disk_name \
                 FROM system.parts WHERE database = '{}' AND active UNION DISTINCT \
                 SELECT 'default')",
                self.db
            ))
            .await?;
        lines.push(format!("disk: {free} bytes free, {bytes} bytes to copy"));
        if !disk_ok(free, bytes) {
            bad.push(format!("free {free} < 1.2 x {bytes}"));
        }
        let mvs: Vec<(String, String)> = self
            .client
            .query("SELECT view, toString(status) FROM system.view_refreshes WHERE database = ? ORDER BY view")
            .bind(&self.db)
            .fetch_all()
            .await?;
        for (view, status) in mvs {
            lines.push(format!("mv {view}: {status}"));
        }
        if bad.is_empty() {
            Ok(lines)
        } else {
            Err(RekeyError::Gate(format!(
                "preflight: {}\n{}",
                bad.join("; "),
                lines.join("\n")
            )))
        }
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

    #[test]
    fn create_renames_only_the_copied_tables_of_init_sql() {
        let sql = create_sql("db");
        let creates = sql.iter().filter(|s| s.starts_with("CREATE")).count();
        assert_eq!(creates, COPIED_TABLES.len());
        assert!(
            sql.contains(
                &"CREATE TABLE IF NOT EXISTS db.price_ohlcv_15m__new AS db.price_ohlcv_1m__new"
                    .to_string()
            )
        );
        assert!(sql.iter().all(|s| !s.contains("prices.")));
        assert!(
            !sql.iter()
                .any(|s| s.contains(" db.assets") || s.contains("usd_rate"))
        );
        assert_eq!(
            rename_for_new("x prices.price_ohlcv_1M, prices.assets;", "d"),
            "x d.price_ohlcv_1M__new, d.assets;"
        );
    }

    #[test]
    fn parity_ignores_id_types_and_names_a_missing_column() {
        let c = |v: &[(&str, &str)]| {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect::<Vec<_>>()
        };
        let live = c(&[("asset_id", "UInt32"), ("price", "Decimal(38, 14)")]);
        assert_eq!(
            parity(
                "t",
                &live,
                &c(&[("asset_id", "UInt64"), ("price", "Decimal(38, 14)")])
            ),
            Ok(())
        );
        assert_eq!(
            parity("t", &live, &c(&[("asset_id", "UInt64")])),
            Err("t__new lacks price".into())
        );
        assert_eq!(
            parity(
                "t",
                &live,
                &c(&[("asset_id", "UInt64"), ("price", "Float64")])
            ),
            Err("price is Decimal(38, 14) in t, Float64 in t__new".into())
        );
    }

    #[test]
    fn fill_maps_ids_through_copyable_rows_and_keys_on_the_target() {
        let l = Layout {
            partition_key: "toYYYYMM(timestamp)".into(),
            key: vec![
                "asset_id".into(),
                "quote_asset_id".into(),
                "timestamp".into(),
            ],
            source_cols: vec![
                "timestamp".into(),
                "asset_id".into(),
                "quote_asset_id".into(),
            ],
            target_cols: vec![
                "timestamp".into(),
                "asset_id".into(),
                "quote_asset_id".into(),
            ],
            ids: vec!["asset_id".into(), "quote_asset_id".into()],
        };
        let filter = partition_where(&l.partition_key, "202401");
        assert_eq!(filter, "toYYYYMM(timestamp) = 202401");
        assert_eq!(partition_where("", "tuple()"), "1");
        let ins = insert_sql("db", &Fill::table("t"), &l, &filter);
        assert!(
            ins.starts_with(
                "INSERT INTO db.t__new (timestamp, asset_id, quote_asset_id) \
             SELECT s.timestamp, m.new_id, q.new_id FROM db.t AS s INNER JOIN"
            ),
            "{ins}"
        );
        assert!(ins.contains(&format!(
            "status IN {COPYABLE}) AS q ON q.old_id = s.quote_asset_id"
        )));
        let probe = probe_sql("db", "t", &l, &filter);
        assert!(
            probe.contains(
                "uniqExactIf(tuple(m.new_id, q.new_id, s.timestamp), m.ok = 1 AND q.ok = 1)"
            ),
            "{probe}"
        );
        assert!(probe.contains("LEFT JOIN"));
        assert_eq!(
            target_keys_sql("db", "t__new", &l, &filter),
            "SELECT uniqExact(tuple(asset_id, quote_asset_id, timestamp)) FROM db.t__new \
             WHERE toYYYYMM(timestamp) = 202401"
        );
    }

    #[test]
    fn classify_names_colliding_before_orphan_and_flags_unmapped_ids() {
        let ids = vec!["asset_id".to_string(), "quote_asset_id".to_string()];
        let sql = classify_sql("db", "t", &ids, "");
        assert!(
            sql.contains(
                "multiIf(m.st = '' OR q.st = '', 'unmapped', m.st = 'colliding' OR q.st = \
             'colliding', 'colliding', m.st = 'orphan' OR q.st = 'orphan', 'orphan', 'copied')"
            ),
            "{sql}"
        );
        assert!(sql.contains("GROUP BY old_id) AS q ON q.old_id = s.quote_asset_id"));
    }

    #[test]
    fn the_disk_gate_wants_one_fifth_headroom_and_the_server_is_26_3() {
        assert!(disk_ok(120, 100));
        assert!(!disk_ok(119, 100));
        assert!(disk_ok(u64::MAX, u64::MAX / 2));
        assert!(supported_version("26.3.10.60"));
        assert!(!supported_version("26.4.1.1"));
        assert!(!supported_version("25.3.1"));
    }
}
