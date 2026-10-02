//! Task 0139 migration tool, the window steps: capture, alter-assets, swap,
//! recreate-mvs, rollback.
//!
//! - `capture`: every MV's and view's `create_table_query`, and the 12 id
//!   tables', into `rekey_0139_ddl`. The rollback's source of truth.
//! - `alter-assets`: `assets` in place (D4): snapshot `assets__pre0139` by
//!   hardlink, `MODIFY COLUMN asset_id UInt64 MATERIALIZED`, `MATERIALIZE
//!   COLUMN`, `ADD CONSTRAINT`, then the equality and uniqueness gates.
//! - `swap`: `EXCHANGE TABLES X AND X__new` for the 11 copied tables, then
//!   `X__new` → `X__pre0139`. Logs `last_live_1m_ts`, the start of the
//!   writer-stop gap. Resumable.
//! - `recreate-mvs`: the MVs (and views) again, so their declared id types are
//!   UInt64, then the type gate.
//! - `rollback`: the reverse, valid until writers resume.
//!
//! `alter-assets` and `swap` refuse unless every refreshable view is
//! Disabled, no INSERT into the database runs, no async insert is queued, and
//! the last `check` covered all 11 tables and was green.

use std::time::Duration;

use super::*;
use crate::asset_id::derived_id_check;
use crate::rollup_sql::TIERS;

/// Suffix of a swapped-out table.
pub const PRE: &str = "__pre0139";
/// `assets` before `alter-assets` (hardlinked parts).
pub const ASSETS_PRE: &str = "assets__pre0139";
/// The altered `assets`, kept aside by a rollback.
pub const ASSETS_NEW: &str = "assets__new";
/// The refusal for post-swap rows; also the CLI flag that overrides it.
pub const FORCE_LOSE: &str = "--force-lose-post-swap-rows";

/// Tables that may keep UInt32 ids after the swap: the swapped-out copies
/// and GA1's old-id-space backups. An SQL predicate on `table`.
pub const OLD_ID_SPACE: &str = "(endsWith(table, 'pre0139') \
     OR startsWith(table, 'rollout_0286_bak_') OR match(table, '^price_ohlcv_.+_bak$'))";

/// `system.columns` rows of `db`: id columns outside [`OLD_ID_SPACE`] that
/// are not UInt64.
pub fn uint32_ids_where(db: &str) -> String {
    format!(
        "database = '{db}' AND name IN ('asset_id', 'quote_asset_id') \
         AND type NOT IN ('UInt64', 'Nullable(UInt64)') AND NOT {OLD_ID_SPACE}"
    )
}

/// `(table, name, type)` of every id column [`uint32_ids_where`] finds.
pub fn uint32_ids_sql(db: &str) -> String {
    format!(
        "SELECT table, name, type FROM system.columns WHERE {} ORDER BY table, name",
        uint32_ids_where(db)
    )
}

/// Where `recreate-mvs` takes the MV and view definitions from (OP1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MvSource {
    /// The captured definitions, id types rewritten to UInt64.
    ProdText,
    /// `rollups.sql`, `current.sql` and `views.sql` of this build.
    Generator,
}

/// A captured column list's id types, UInt32 → UInt64 (also inside
/// `Nullable`, which `mv_current_prices` declares).
pub fn with_uint64_ids(ddl: &str) -> String {
    ["asset_id", "quote_asset_id"]
        .iter()
        .flat_map(|c| [(*c, ""), (*c, "Nullable(")])
        .fold(ddl.to_string(), |s, (c, wrap)| {
            s.replace(
                &format!("`{c}` {wrap}UInt32"),
                &format!("`{c}` {wrap}UInt64"),
            )
        })
}

fn ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `from.<name>` → `to.<name>`, for every name in `names` (all, if `None`).
pub fn rewrite_db(ddl: &str, from: &str, to: &str, names: Option<&[String]>) -> String {
    let pat = format!("{from}.");
    let mut out = String::with_capacity(ddl.len());
    let mut rest = ddl;
    while let Some(at) = rest.find(&pat) {
        out.push_str(&rest[..at]);
        let bounded = out.chars().last().is_none_or(|c| !ident(c));
        let tail = &rest[at + pat.len()..];
        let end = tail.find(|c: char| !ident(c)).unwrap_or(tail.len());
        let name = &tail[..end];
        let hit = bounded && names.is_none_or(|n| n.iter().any(|x| x == name));
        out.push_str(&format!("{}.{name}", if hit { to } else { from }));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// A rehearsal's definitions run as the operator: `DEFINER = <user>` becomes
/// `DEFINER = CURRENT_USER`. A production MV's definer (`prices_admin`) cannot
/// read the scratch database, so its refresh would fail there.
pub fn own_definer(ddl: &str) -> String {
    let pat = "DEFINER = ";
    let mut out = String::with_capacity(ddl.len());
    let mut rest = ddl;
    while let Some(at) = rest.find(pat) {
        out.push_str(&rest[..at + pat.len()]);
        let tail = &rest[at + pat.len()..];
        let end = tail.find(|c: char| !ident(c)).unwrap_or(tail.len());
        out.push_str("CURRENT_USER");
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// `db.name` created by a CREATE [MATERIALIZED] VIEW statement.
pub fn created_object(stmt: &str) -> Option<&str> {
    let rest = [
        "CREATE MATERIALIZED VIEW IF NOT EXISTS ",
        "CREATE MATERIALIZED VIEW ",
        "CREATE OR REPLACE VIEW ",
        "CREATE VIEW ",
    ]
    .iter()
    .find_map(|p| stmt.trim_start().strip_prefix(p))?;
    rest.split_whitespace().next()
}

/// The statement creates its object in `db` and writes (`TO`) only into `db`.
pub fn creates_in(stmt: &str, db: &str) -> bool {
    let head = stmt.split(" AS ").next().unwrap_or(stmt);
    let in_db = |o: &str| o.strip_prefix(db).is_some_and(|r| r.starts_with('.'));
    created_object(stmt).is_some_and(in_db)
        && head
            .split_once(" TO ")
            .is_none_or(|(_, t)| t.split_whitespace().next().is_some_and(in_db))
}

/// Captured MVs, dependencies first: the fast tiers fine to coarse, the
/// reconcile tiers, `mv_current_prices`, then anything else by name.
fn mv_rank(name: &str) -> (usize, String) {
    let known: Vec<&str> = TIERS
        .iter()
        .map(|t| t.mv)
        .chain(TIERS.iter().map(|t| t.reconcile_mv))
        .chain(["mv_current_prices"])
        .collect();
    (
        known.iter().position(|k| *k == name).unwrap_or(known.len()),
        name.to_string(),
    )
}

/// Views in `views.sql` order (a view may read another).
fn view_rank(name: &str) -> (usize, String) {
    (
        crate::VIEWS_SQL
            .find(&format!("VIEW prices.{name} AS"))
            .unwrap_or(usize::MAX),
        name.to_string(),
    )
}

/// Why the window is not quiet: a refreshable view not Disabled, a running
/// INSERT, a queued async insert.
pub fn window_refusals(
    refreshes: &[(String, String)],
    inserts: u64,
    async_inserts: u64,
) -> Vec<String> {
    let mut why: Vec<String> = refreshes
        .iter()
        .filter(|(_, s)| s != "Disabled")
        .map(|(v, s)| format!("{v} is {s}, not Disabled (SYSTEM STOP VIEW)"))
        .collect();
    if inserts > 0 {
        why.push(format!(
            "{inserts} INSERT queries into the database are running"
        ));
    }
    if async_inserts > 0 {
        why.push(format!("{async_inserts} async inserts are queued"));
    }
    why
}

impl Rekey {
    async fn captured(&self, kind: &str) -> Result<Vec<(String, String)>> {
        if !self.exists(DDL_TABLE).await? {
            return Ok(Vec::new());
        }
        Ok(self
            .client
            .query(&format!(
                "SELECT name, create_query FROM {} WHERE kind = ? ORDER BY name",
                self.t(DDL_TABLE)
            ))
            .bind(kind)
            .fetch_all()
            .await?)
    }

    /// `(at, status, range_from, range_to)` of the last row of `step` since
    /// the last rollback.
    pub(crate) async fn last_logged(
        &self,
        step: &str,
    ) -> Result<Option<(String, String, u32, u32)>> {
        if !self.exists(LOG_TABLE).await? {
            return Ok(None);
        }
        Ok(self
            .client
            .query(&format!(
                "SELECT toString(at), status, toUInt32(range_from), toUInt32(range_to) \
                 FROM {log} WHERE step = ? AND (step = 'rollback' OR at > \
                 (SELECT max(at) FROM {log} WHERE step = 'rollback')) \
                 ORDER BY at DESC LIMIT 1",
                log = self.t(LOG_TABLE)
            ))
            .bind(step)
            .fetch_optional()
            .await?)
    }

    pub(crate) async fn id_type(&self, table: &str) -> Result<Option<String>> {
        Ok(self
            .id_types(table)
            .await?
            .into_iter()
            .find(|(c, _)| c == "asset_id")
            .map(|(_, t)| t))
    }

    /// Store the DDL of every MV, view and id table. With `rewrite_from`
    /// (rehearsal in a scratch database), the MVs and views are read from
    /// that database and their references to the id tables and to each
    /// other are rewritten into this one. Refused after `swap`.
    pub async fn capture(&self, rewrite_from: Option<&str>) -> Result<Vec<String>> {
        if self.last_logged("swap").await?.is_some() {
            return Err(RekeyError::Refused(
                "swap has run: the captured DDL is the rollback's source and stays".into(),
            ));
        }
        let views_db = rewrite_from.unwrap_or(&self.db).to_string();
        if views_db != self.db && self.db == crate::PROD_DATABASE {
            return Err(RekeyError::Refused(
                "--rewrite-db is for a scratch database, not prices".into(),
            ));
        }
        self.ensure_tool_tables().await?;
        let views: Vec<(String, String, String)> = self
            .client
            .query(
                "SELECT if(engine = 'MaterializedView', 'mv', 'view'), name, \
                 create_table_query FROM system.tables WHERE database = ? \
                 AND engine IN ('MaterializedView', 'View') ORDER BY name",
            )
            .bind(&views_db)
            .fetch_all()
            .await?;
        let mut names: Vec<String> = views.iter().map(|(_, n, _)| n.clone()).collect();
        names.extend(
            std::iter::once("assets")
                .chain(COPIED_TABLES)
                .map(String::from),
        );
        self.write(&format!("TRUNCATE TABLE {}", self.t(DDL_TABLE)))
            .await?;
        let mut lines = Vec::new();
        for (kind, name, q) in &views {
            let mut q = rewrite_db(q, &views_db, &self.db, Some(&names));
            if views_db != self.db {
                q = own_definer(&q);
            }
            self.write(&format!(
                "INSERT INTO {} (kind, name, create_query) SELECT {}, {}, {}",
                self.t(DDL_TABLE),
                crate::asset_id::sql_str(kind),
                crate::asset_id::sql_str(name),
                crate::asset_id::sql_str(&q)
            ))
            .await?;
            lines.push(format!("{kind} {name}"));
        }
        let tables = std::iter::once("assets")
            .chain(COPIED_TABLES)
            .map(|t| format!("'{t}'"))
            .collect::<Vec<_>>()
            .join(", ");
        self.write(&format!(
            "INSERT INTO {} (kind, name, create_query) SELECT 'table', name, \
             create_table_query FROM system.tables WHERE database = '{}' AND name IN ({tables})",
            self.t(DDL_TABLE),
            self.db
        ))
        .await?;
        lines.push(format!("{} id tables", COPIED_TABLES.len() + 1));
        self.log(Log {
            step: "capture",
            status: "ok",
            detail: &lines.join(", "),
            ..Log::default()
        })
        .await?;
        Ok(lines)
    }

    /// Capture has run, every refreshable view is Disabled, no INSERT into
    /// the database runs, no async insert is queued.
    pub async fn window_gates(&self) -> Result<()> {
        if self.captured("table").await?.is_empty() {
            return Err(RekeyError::Refused("nothing captured (run capture)".into()));
        }
        let refreshes: Vec<(String, String)> = self
            .client
            .query(
                "SELECT view, toString(status) FROM system.view_refreshes \
                 WHERE database = ? ORDER BY view",
            )
            .bind(&self.db)
            .fetch_all()
            .await?;
        let inserts: u64 = self
            .client
            .query(
                "SELECT count() FROM system.processes WHERE query_kind = 'Insert' \
                 AND (current_database = ? OR positionCaseInsensitive(query, ?) > 0)",
            )
            .bind(&self.db)
            .bind(format!("{}.", self.db))
            .fetch_one()
            .await?;
        let async_inserts: u64 = self
            .client
            .query("SELECT count() FROM system.asynchronous_inserts WHERE database = ?")
            .bind(&self.db)
            .fetch_one()
            .await?;
        let why = window_refusals(&refreshes, inserts, async_inserts);
        if why.is_empty() {
            Ok(())
        } else {
            Err(RekeyError::Refused(format!("window: {}", why.join("; "))))
        }
    }

    /// The last `check` covered all 11 tables, was green, and no fill ran
    /// after it.
    pub async fn check_green(&self) -> Result<()> {
        let refuse = |m: &str| Err(RekeyError::Refused(format!("check is not green: {m}")));
        if !self.exists(LOG_TABLE).await? {
            return refuse("no log");
        }
        let last: Option<(String, String)> = self
            .client
            .query(&format!(
                "SELECT status, detail FROM {log} WHERE step = 'check' \
                 AND at >= (SELECT max(at) FROM {log} WHERE step = 'fill') \
                 ORDER BY at DESC LIMIT 1",
                log = self.t(LOG_TABLE)
            ))
            .fetch_optional()
            .await?;
        match last {
            None => refuse("no check after the last fill"),
            Some((status, _)) if status != "ok" => refuse(&format!("last check {status}")),
            Some((_, detail)) => match COPIED_TABLES
                .iter()
                .find(|t| !detail.contains(&format!("{t} -> {t}__new:")))
            {
                Some(t) => refuse(&format!("last check did not cover {t}")),
                None => Ok(()),
            },
        }
    }

    async fn wait_mutations(&self, table: &str, timeout: Duration) -> Result<()> {
        if !self.execute {
            return Ok(());
        }
        let start = std::time::Instant::now();
        loop {
            let (open, why): (u64, String) = self
                .client
                .query(
                    "SELECT count(), any(latest_fail_reason) FROM system.mutations \
                     WHERE database = ? AND table = ? AND NOT is_done",
                )
                .bind(&self.db)
                .bind(table)
                .fetch_one()
                .await?;
            if open == 0 {
                return Ok(());
            }
            if start.elapsed() >= timeout {
                return Err(RekeyError::Gate(format!(
                    "{open} mutations on {table} not done after {}s ({why}); \
                     check merges are running: SYSTEM START MERGES {}",
                    timeout.as_secs(),
                    self.t(table)
                )));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    /// An `ALTER` that returns at once; [`Self::wait_mutations`] bounds the
    /// wait (a type change waits forever while merges are stopped).
    async fn alter_async(&self, sql: &str) -> Result<()> {
        if self.execute {
            self.client
                .query(sql)
                .with_option("alter_sync", "0")
                .with_option("mutations_sync", "0")
                .execute()
                .await?;
        } else {
            println!("{sql};");
        }
        Ok(())
    }

    /// `assets` in place: snapshot, retype, materialize, constrain, gate.
    /// Resumes after a partial run.
    pub async fn alter_assets(&self, timeout: Duration) -> Result<Vec<String>> {
        self.window_gates().await?;
        self.check_green().await?;
        let assets = self.t("assets");
        let pre = self.t(ASSETS_PRE);
        let mut lines = Vec::new();
        if self.id_type("assets").await?.as_deref() == Some("UInt32") {
            self.write(&format!("DROP TABLE IF EXISTS {pre} SYNC"))
                .await?;
            self.write(&format!("CREATE TABLE {pre} AS {assets}"))
                .await?;
            self.write(&format!("SYSTEM STOP MERGES {pre}")).await?;
            for (_, pid) in self.partitions("assets").await? {
                self.write(&format!(
                    "ALTER TABLE {pre} ATTACH PARTITION ID '{pid}' FROM {assets}"
                ))
                .await?;
            }
            if self.execute {
                let (a, p) = (
                    self.count(&format!("SELECT count() FROM {assets}")).await?,
                    self.count(&format!("SELECT count() FROM {pre}")).await?,
                );
                if a != p {
                    return Err(RekeyError::Gate(format!(
                        "alter-assets: snapshot holds {p} of {a} rows"
                    )));
                }
                lines.push(format!("snapshot {ASSETS_PRE}: {p} rows"));
            }
            self.write(&format!("SYSTEM START MERGES {assets}")).await?;
            self.alter_async(&format!(
                "ALTER TABLE {assets} MODIFY COLUMN asset_id UInt64 MATERIALIZED {}",
                id_expr("asset_code", "issuer_address", "contract_address")
            ))
            .await?;
            self.wait_mutations("assets", timeout).await?;
        } else {
            lines.push("asset_id is already UInt64: resuming".into());
            self.write(&format!("SYSTEM START MERGES {assets}")).await?;
        }
        self.alter_async(&format!("ALTER TABLE {assets} MATERIALIZE COLUMN asset_id"))
            .await?;
        self.wait_mutations("assets", timeout).await?;
        self.write(&format!(
            "ALTER TABLE {assets} ADD CONSTRAINT IF NOT EXISTS asset_id_derived CHECK {}",
            derived_id_check(&["asset_id"])
        ))
        .await?;
        if !self.execute {
            return Ok(lines);
        }
        let (rows, wrong): (u64, u64) = self
            .client
            .query(&format!(
                "SELECT count(), countIf(asset_id != {}) FROM {assets}",
                id_expr("asset_code", "issuer_address", "contract_address")
            ))
            .fetch_one()
            .await?;
        let (ids, distinct): (u64, u64) = self
            .client
            .query(&format!(
                "SELECT count(), uniqExact(asset_id) FROM {assets} FINAL"
            ))
            .fetch_one()
            .await?;
        if wrong > 0 || ids != distinct {
            return Err(RekeyError::Gate(format!(
                "alter-assets: {wrong} of {rows} rows differ from the expression; \
                 FINAL {ids} rows, {distinct} distinct ids"
            )));
        }
        lines.push(format!(
            "assets: {rows} rows derived, {ids} identities, {distinct} ids"
        ));
        self.log(Log {
            step: "alter-assets",
            status: "ok",
            detail: &lines.join("; "),
            ..Log::default()
        })
        .await?;
        Ok(lines)
    }

    /// Exchange the 11 copied tables with their `X__new`, then name the old
    /// ones `X__pre0139`. With `check_only`, only the window gates run.
    pub async fn swap(&self, check_only: bool) -> Result<Vec<String>> {
        let last = self.last_logged("swap").await?;
        if !check_only && last.as_ref().is_some_and(|(_, s, _, _)| s == "ok") {
            return Err(RekeyError::Refused("swap already done".into()));
        }
        self.window_gates().await?;
        if check_only {
            let check = match self.check_green().await {
                Ok(()) => "green".to_string(),
                Err(e) => e.to_string(),
            };
            return Ok(vec![
                "window: every view Disabled, no insert running or queued".into(),
                format!("check: {check}"),
            ]);
        }
        let (from, to) = match last {
            Some((_, _, from, to)) => (from, to),
            None => {
                self.check_green().await?;
                if self.id_type("assets").await?.as_deref() != Some("UInt64")
                    || self.altered().await? == 0
                {
                    return Err(RekeyError::Refused("run alter-assets first".into()));
                }
                let (from, to): (u32, u32) = self
                    .client
                    .query(&format!(
                        "SELECT toUInt32(max(timestamp)), toUInt32(now()) FROM {}",
                        self.t("price_ohlcv_1m")
                    ))
                    .fetch_one()
                    .await?;
                self.log(Log {
                    step: "swap",
                    status: "started",
                    range_from: from,
                    range_to: to,
                    detail: "range = (last_live_1m_ts, swap_at)",
                    ..Log::default()
                })
                .await?;
                (from, to)
            }
        };
        let mut lines = Vec::new();
        for t in COPIED_TABLES {
            let new = format!("{t}__new");
            let pre = format!("{t}{PRE}");
            let live = self.id_type(t).await?;
            let staged = self.id_type(&new).await?;
            let action = match (live.as_deref(), staged.as_deref()) {
                (Some("UInt32"), Some("UInt64")) => "exchanged",
                (Some("UInt64"), Some("UInt32")) => "renamed",
                (Some("UInt64"), None) if self.exists(&pre).await? => {
                    lines.push(format!("{t}: already swapped"));
                    continue;
                }
                (l, s) => {
                    return Err(RekeyError::Refused(format!(
                        "{t} is {l:?} and {new} is {s:?}: cannot swap"
                    )));
                }
            };
            if action == "exchanged" {
                self.write(&format!(
                    "EXCHANGE TABLES {} AND {}",
                    self.t(t),
                    self.t(&new)
                ))
                .await?;
            }
            self.write(&format!(
                "RENAME TABLE {} TO {}",
                self.t(&new),
                self.t(&pre)
            ))
            .await?;
            // Every swap row carries the range: a resumed swap reads it back
            // from the newest one, whichever row that is.
            self.log(Log {
                step: "swap",
                target: t,
                status: action,
                range_from: from,
                range_to: to,
                ..Log::default()
            })
            .await?;
            lines.push(format!("{t}: {action}"));
        }
        self.log(Log {
            step: "swap",
            status: "ok",
            range_from: from,
            range_to: to,
            detail: "range = (last_live_1m_ts, swap_at)",
            ..Log::default()
        })
        .await?;
        lines.push(format!("last_live_1m_ts {from}, swap_at {to} (unix s)"));
        Ok(lines)
    }

    /// No id column of the database is UInt32 outside [`OLD_ID_SPACE`].
    pub async fn type_gate(&self) -> Result<()> {
        let bad: Vec<(String, String, String)> = self
            .client
            .query(&uint32_ids_sql(&self.db))
            .fetch_all()
            .await?;
        if bad.is_empty() {
            return Ok(());
        }
        Err(RekeyError::Gate(format!(
            "type gate: {}",
            bad.iter()
                .map(|(t, c, ty)| format!("{t}.{c} is {ty}"))
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }

    /// The MV and view statements of `source`, in creation order.
    async fn definitions(&self, source: MvSource) -> Result<(Vec<String>, Vec<String>)> {
        let all = |sql: &str| {
            crate::split_statements(sql)
                .into_iter()
                .filter(|s| s.starts_with("CREATE"))
                .map(|s| rewrite_db(&s, crate::PROD_DATABASE, &self.db, None))
                .collect::<Vec<_>>()
        };
        Ok(match source {
            MvSource::Generator => (
                [crate::ROLLUPS_SQL, crate::CURRENT_SQL]
                    .iter()
                    .flat_map(|s| all(s))
                    .collect(),
                all(crate::VIEWS_SQL),
            ),
            MvSource::ProdText => {
                let mut mvs = self.captured("mv").await?;
                mvs.sort_by_key(|(n, _)| mv_rank(n));
                let mut views = self.captured("view").await?;
                views.sort_by_key(|(n, _)| view_rank(n));
                (
                    mvs.iter().map(|(_, q)| with_uint64_ids(q)).collect(),
                    views
                        .iter()
                        .map(|(_, q)| {
                            with_uint64_ids(&q.replacen("CREATE VIEW", "CREATE OR REPLACE VIEW", 1))
                        })
                        .collect(),
                )
            }
        })
    }

    /// Drop every captured MV and every MV about to be created, then create
    /// `mvs` and `views`. Each must stay inside this database.
    async fn replace_mvs(&self, mvs: &[String], views: &[String]) -> Result<()> {
        if let Some(bad) = mvs.iter().chain(views).find(|s| !creates_in(s, &self.db)) {
            return Err(RekeyError::Refused(format!(
                "a definition creates or writes outside {} (capture --rewrite-db): {}",
                self.db,
                bad.chars().take(120).collect::<String>()
            )));
        }
        let mut drop: Vec<String> = self
            .client
            .query(
                "SELECT name FROM system.tables WHERE database = ? \
                 AND engine = 'MaterializedView'",
            )
            .bind(&self.db)
            .fetch_all()
            .await?;
        drop.extend(self.captured("mv").await?.into_iter().map(|(n, _)| n));
        drop.extend(
            mvs.iter()
                .filter_map(|s| created_object(s))
                .filter_map(|o| o.split_once('.').map(|(_, n)| n.to_string())),
        );
        drop.sort();
        drop.dedup();
        for mv in &drop {
            self.write(&format!("DROP VIEW IF EXISTS {} SYNC", self.t(mv)))
                .await?;
        }
        for s in mvs.iter().chain(views) {
            self.write(s).await?;
        }
        Ok(())
    }

    /// Re-create the MVs and views so their declared ids are UInt64, then
    /// the type gate. Refused before `swap`.
    pub async fn recreate_mvs(&self, source: MvSource) -> Result<Vec<String>> {
        if !self
            .last_logged("swap")
            .await?
            .is_some_and(|(_, s, _, _)| s == "ok")
        {
            return Err(RekeyError::Refused("swap has not run".into()));
        }
        let (mvs, views) = self.definitions(source).await?;
        self.replace_mvs(&mvs, &views).await?;
        if self.execute {
            self.type_gate().await?;
        }
        let detail = format!("{source:?}: {} MVs, {} views", mvs.len(), views.len());
        self.log(Log {
            step: "recreate-mvs",
            status: "ok",
            detail: &detail,
            ..Log::default()
        })
        .await?;
        Ok(vec![
            detail,
            "type gate: no UInt32 id outside the old id space".into(),
        ])
    }

    /// Rows a rollback would lose: 1m candles after the last live one, oracle
    /// rows after the swap, identities `assets` gained after `alter-assets`.
    async fn post_swap_rows(&self, last_live: u32, swap_at: u32) -> Result<Vec<String>> {
        let mut found = Vec::new();
        let probes = [
            (
                "price_ohlcv_1m",
                format!("timestamp > toDateTime({last_live})"),
            ),
            (
                "oracle_prices",
                format!("timestamp > toDateTime({swap_at})"),
            ),
        ];
        for (t, cond) in probes {
            if self.id_type(t).await?.as_deref() == Some("UInt64") {
                let n = self
                    .count(&format!("SELECT count() FROM {} WHERE {cond}", self.t(t)))
                    .await?;
                if n > 0 {
                    found.push(format!("{t}: {n} rows"));
                }
            }
        }
        if self.exists(ASSETS_PRE).await? {
            let n = self
                .count(&format!(
                    "SELECT count() FROM {} WHERE (asset_code, issuer_address, \
                     contract_address) NOT IN (SELECT asset_code, issuer_address, \
                     contract_address FROM {})",
                    self.t("assets"),
                    self.t(ASSETS_PRE)
                ))
                .await?;
            if n > 0 {
                found.push(format!("assets: {n} new identities"));
            }
        }
        Ok(found)
    }

    /// Wait until every refreshable view is Disabled: a CREATE starts a
    /// refresh that `SYSTEM STOP VIEW` does not cancel at once.
    async fn settle_views(&self, timeout: Duration) -> Result<()> {
        if !self.execute {
            return Ok(());
        }
        let start = std::time::Instant::now();
        loop {
            let running = self
                .count(&format!(
                    "SELECT count() FROM system.view_refreshes WHERE database = '{}' \
                     AND toString(status) != 'Disabled'",
                    self.db
                ))
                .await?;
            if running == 0 {
                return Ok(());
            }
            if start.elapsed() > timeout {
                return Err(RekeyError::Gate(format!(
                    "{running} views still not Disabled after {}s",
                    timeout.as_secs()
                )));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Undo swap and alter-assets, re-create the captured MVs (stopped) and
    /// views. Refused if rows were written after the swap, unless `force`.
    pub async fn rollback(&self, force: bool) -> Result<Vec<String>> {
        let Some((_, _, last_live, swap_at)) = self.last_logged("swap").await? else {
            return Err(RekeyError::Refused(
                "swap has not run: nothing to roll back".into(),
            ));
        };
        let lost = self.post_swap_rows(last_live, swap_at).await?;
        if !lost.is_empty() && !force {
            return Err(RekeyError::Refused(format!(
                "rows were written after the swap ({}); rolling back loses them: {FORCE_LOSE}",
                lost.join(", ")
            )));
        }
        let mut lines = Vec::new();
        for t in COPIED_TABLES {
            let pre = format!("{t}{PRE}");
            if self.id_type(t).await?.as_deref() == Some("UInt64") && self.exists(&pre).await? {
                self.write(&format!(
                    "EXCHANGE TABLES {} AND {}",
                    self.t(t),
                    self.t(&pre)
                ))
                .await?;
                self.write(&format!(
                    "RENAME TABLE {} TO {}",
                    self.t(&pre),
                    self.t(&format!("{t}__new"))
                ))
                .await?;
                lines.push(format!("{t}: restored"));
            }
        }
        if self.id_type("assets").await?.as_deref() == Some("UInt64")
            && self.exists(ASSETS_PRE).await?
        {
            let (assets, pre, new) = (self.t("assets"), self.t(ASSETS_PRE), self.t(ASSETS_NEW));
            self.write(&format!("DROP TABLE IF EXISTS {new} SYNC"))
                .await?;
            self.write(&format!("EXCHANGE TABLES {assets} AND {pre}"))
                .await?;
            self.write(&format!("RENAME TABLE {pre} TO {new}")).await?;
            self.write(&format!("SYSTEM START MERGES {assets}")).await?;
            lines.push("assets: restored".into());
        }
        let mut mvs = self.captured("mv").await?;
        mvs.sort_by_key(|(n, _)| mv_rank(n));
        let mut views = self.captured("view").await?;
        views.sort_by_key(|(n, _)| view_rank(n));
        let mv_sql: Vec<String> = mvs.iter().map(|(_, q)| q.clone()).collect();
        let view_sql: Vec<String> = views
            .iter()
            .map(|(_, q)| q.replacen("CREATE VIEW", "CREATE OR REPLACE VIEW", 1))
            .collect();
        self.replace_mvs(&mv_sql, &view_sql).await?;
        for (mv, _) in &mvs {
            self.write(&format!("SYSTEM STOP VIEW {}", self.t(mv)))
                .await?;
        }
        self.settle_views(Duration::from_secs(300)).await?;
        lines.push(format!(
            "{} MVs re-created from the capture and STOPPED; {} views",
            mvs.len(),
            views.len()
        ));
        let detail = lines.join("; ");
        self.log(Log {
            step: "rollback",
            status: "ok",
            detail: &detail,
            ..Log::default()
        })
        .await?;
        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_id_types_become_uint64_only_in_the_column_list() {
        let mv = "CREATE MATERIALIZED VIEW p.mv REFRESH EVERY 1 MINUTE APPEND TO p.t \
                  (`timestamp` DateTime, `asset_id` UInt32, `quote_asset_id` UInt32) \
                  AS SELECT asset_id, toUInt32(1) AS x FROM p.s";
        let out = with_uint64_ids(mv);
        assert!(out.contains("(`timestamp` DateTime, `asset_id` UInt64, `quote_asset_id` UInt64)"));
        assert_eq!(
            with_uint64_ids("TO p.c (`asset_id` Nullable(UInt32), `n` UInt32)"),
            "TO p.c (`asset_id` Nullable(UInt64), `n` UInt32)"
        );
        assert!(out.contains("toUInt32(1) AS x"));
    }

    #[test]
    fn a_rehearsal_definition_runs_as_the_operator() {
        assert_eq!(
            own_definer(
                "CREATE MATERIALIZED VIEW s.mv REFRESH EVERY 1 HOUR APPEND TO s.t \
                 DEFINER = prices_admin SQL SECURITY DEFINER AS SELECT 1"
            ),
            "CREATE MATERIALIZED VIEW s.mv REFRESH EVERY 1 HOUR APPEND TO s.t \
             DEFINER = CURRENT_USER SQL SECURITY DEFINER AS SELECT 1"
        );
        assert_eq!(
            own_definer("CREATE VIEW s.v AS SELECT 1"),
            "CREATE VIEW s.v AS SELECT 1"
        );
    }

    #[test]
    fn rewrite_touches_only_named_objects_on_a_boundary() {
        let names = vec!["assets".to_string(), "mv".to_string()];
        assert_eq!(
            rewrite_db(
                "CREATE VIEW prices.mv AS SELECT 1 FROM prices.assets, prices.usd_rate, xprices.assets",
                "prices",
                "scratch",
                Some(&names)
            ),
            "CREATE VIEW scratch.mv AS SELECT 1 FROM scratch.assets, prices.usd_rate, xprices.assets"
        );
        assert_eq!(
            rewrite_db("FROM prices.a JOIN prices.b", "prices", "s", None),
            "FROM s.a JOIN s.b"
        );
    }

    #[test]
    fn a_definition_must_create_and_write_inside_the_database() {
        let mv = |obj: &str, to: &str| {
            format!(
                "CREATE MATERIALIZED VIEW {obj} REFRESH EVERY 1 MINUTE APPEND TO {to} (`a` UInt64) AS SELECT 1 FROM prices.x"
            )
        };
        assert!(creates_in(&mv("s.mv", "s.t"), "s"));
        assert!(!creates_in(&mv("prices.mv", "s.t"), "s"));
        assert!(!creates_in(&mv("s.mv", "prices.t"), "s"));
        assert!(!creates_in(&mv("sx.mv", "s.t"), "s"));
        assert!(creates_in(
            "CREATE OR REPLACE VIEW s.v AS SELECT 1 FROM prices.y",
            "s"
        ));
        assert!(!creates_in("ALTER TABLE s.v DELETE WHERE 1", "s"));
        assert_eq!(
            created_object("CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv\nREFRESH"),
            Some("prices.mv")
        );
    }

    #[test]
    fn the_window_refuses_a_live_view_an_insert_or_a_queued_insert() {
        let s = |v: &[(&str, &str)]| {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect::<Vec<_>>()
        };
        let quiet = s(&[("a", "Disabled"), ("b", "Disabled")]);
        assert!(window_refusals(&quiet, 0, 0).is_empty());
        for status in ["Scheduled", "Running", "WaitingForDependencies"] {
            let r = window_refusals(&s(&[("a", "Disabled"), ("b", status)]), 0, 0);
            assert_eq!(
                r,
                vec![format!("b is {status}, not Disabled (SYSTEM STOP VIEW)")]
            );
        }
        assert_eq!(
            window_refusals(&quiet, 2, 0),
            vec!["2 INSERT queries into the database are running"]
        );
        assert_eq!(
            window_refusals(&quiet, 0, 1),
            vec!["1 async inserts are queued"]
        );
        assert_eq!(window_refusals(&quiet, 1, 1).len(), 2);
    }

    #[test]
    fn the_type_gate_skips_only_the_old_id_space() {
        let sql = uint32_ids_sql("prices");
        assert!(sql.contains("type NOT IN ('UInt64', 'Nullable(UInt64)')"));
        for kept in [
            "endsWith(table, 'pre0139')",
            "rollout_0286_bak_",
            "^price_ohlcv_.+_bak$",
        ] {
            assert!(OLD_ID_SPACE.contains(kept), "{kept}");
        }
        assert!(
            !OLD_ID_SPACE.contains("reingest_0286_bak"),
            "GA1 renames those first"
        );
    }

    #[test]
    fn mvs_are_created_dependencies_first() {
        let mut names = vec![
            "mv_current_prices",
            "zz",
            "mv_ohlcv_1h_to_4h",
            "mv_ohlcv_1m_to_15m",
        ];
        names.sort_by_key(|n| mv_rank(n));
        assert_eq!(
            names,
            vec![
                "mv_ohlcv_1m_to_15m",
                "mv_ohlcv_1h_to_4h",
                "mv_current_prices",
                "zz"
            ]
        );
        let mut views = vec!["current_price_usd", "usd_reference"];
        views.sort_by_key(|n| view_rank(n));
        assert_eq!(views, vec!["usd_reference", "current_price_usd"]);
    }
}
