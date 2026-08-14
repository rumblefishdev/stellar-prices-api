//! Schema-drift detection for the refreshable rollup MVs (task 0142).
//!
//! # Why this exists
//!
//! Every MV in `schema/rollups.sql` is declared `CREATE MATERIALIZED VIEW IF NOT
//! EXISTS`. `IF NOT EXISTS` does not redefine an object that already exists, so
//! on a provisioned target (ch-prod-01, which holds all six) **editing an MV body
//! and re-applying the file changes nothing and the apply reports success.** Task
//! 0134 removed exactly this footgun from `views.sql` by converting those to
//! `CREATE OR REPLACE VIEW`; that escape is not available here, because a
//! refreshable `TO`-table MV has no `OR REPLACE` form. Changing one requires
//! `DROP` + re-`CREATE`, and that is a data-exposure window (see
//! `docs/runbooks/0142-rollup-mv-reapply.md`), so it stays an operator action
//! rather than something the apply path does implicitly.
//!
//! What this module provides is the cheap half: making the divergence
//! **visible**. It is strictly read-only — it issues `SELECT`s against
//! `system.tables` and the `formatQuerySingleLine` function, and creates,
//! alters and drops nothing.
//!
//! # Why the comparison is not a string compare
//!
//! ClickHouse does not store the submitted text. `system.tables.create_table_query`
//! is re-serialised from the parsed AST, which differs from the file in four
//! deterministic ways (all verified on 26.3.10.60):
//!
//! 1. `IF NOT EXISTS` is dropped.
//! 2. The target table's full column list is injected after `TO <table>`.
//! 3. `DEFINER = <user> SQL SECURITY DEFINER` is injected.
//! 4. Syntax is normalised — `INTERVAL 15 MINUTE` becomes `toIntervalMinute(15)`,
//!    whitespace collapses, identifiers may gain backticks.
//!
//! A naive text compare would therefore report drift permanently, which is worse
//! than no check at all — a permanently-red signal gets ignored, and the real
//! drift arrives unnoticed inside it.
//!
//! The fix is to let ClickHouse's own printer normalise **both** sides:
//! `formatQuerySingleLine` renders a submitted statement through the same AST
//! serialiser that produced `create_table_query`, so differences (1) and (4)
//! disappear. Differences (2) and (3) are handled by comparing a
//! [`MvFingerprint`] rather than whole text: the injected column list and DEFINER
//! clause both sit strictly between the `TO <target>` token and ` AS SELECT`, so
//! parsing out `(name, refresh, target, body)` skips them by construction. That
//! is correct rather than merely convenient — the column list is a property of
//! the target table, not of the MV definition we own, and re-deriving it is not
//! something an edit to this file can change.

use clickhouse::Client;

use crate::{ROLLUPS_SQL, SchemaError, split_statements};

/// The load-bearing parts of a `CREATE MATERIALIZED VIEW` statement, parsed from
/// ClickHouse's own single-line rendering of it.
///
/// Both sides of a comparison are parsed by [`parse_fingerprint`] from output of
/// `formatQuerySingleLine`, so the parser only ever sees the server's canonical
/// form — never the hand-written file text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MvFingerprint {
    /// Qualified object name, e.g. `prices.mv_ohlcv_1m_to_15m`.
    pub name: String,
    /// Everything between `REFRESH` and `TO`, e.g. `EVERY 1 MINUTE APPEND`.
    pub refresh: String,
    /// Qualified target table, e.g. `prices.price_ohlcv_15m`.
    pub target: String,
    /// The whole `SELECT …` body.
    pub body: String,
}

impl MvFingerprint {
    /// Whether the MV refreshes in `APPEND` mode.
    ///
    /// A refreshable MV **without** `APPEND` atomically replaces its entire
    /// target table on every refresh. Paired with the bounded
    /// `WHERE timestamp >= now() - <window>` these MVs all carry, replace mode
    /// overwrites the coarse table with only the recent window each tick,
    /// deleting pre-rolled history — the production data loss task 0090 found
    /// and task 0095 fixed. A live MV that has lost `APPEND` is therefore not
    /// ordinary drift; it is actively destroying history on every tick.
    pub fn is_append(&self) -> bool {
        self.refresh.split_whitespace().any(|w| w == "APPEND")
    }
}

/// Which part of the definition diverged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftField {
    /// The refresh clause — cadence, or the load-bearing `APPEND` mode.
    Refresh,
    /// The `TO` target table.
    Target,
    /// The `SELECT` body: projections, source, window, `GROUP BY`.
    Body,
}

impl DriftField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Refresh => "refresh clause",
            Self::Target => "target table",
            Self::Body => "select body",
        }
    }
}

/// One diverging field, with both renderings for the operator to diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    pub field: DriftField,
    /// What `schema/rollups.sql` declares.
    pub declared: String,
    /// What the target actually holds.
    pub live: String,
}

/// Outcome for a single MV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MvStatus {
    /// The live definition matches the file.
    InSync,
    /// The file declares it; the target does not have it. On a provisioned
    /// cluster this is the [task 0136] shape — an MV dropped for recovery and
    /// never re-created, so that tier silently stops rolling up.
    ///
    /// [task 0136]: docs/runbooks/0136-coarse-rollup-merge-recovery.md
    Missing,
    /// The live definition and the file disagree. On a target that already holds
    /// the MV this is what re-applying `rollups.sql` will NOT fix: the apply
    /// will report success and change nothing.
    Drifted(Vec<Difference>),
    /// The object exists under a declared name but its live definition is not a
    /// shape this module can fingerprint — e.g. re-created without `REFRESH`,
    /// which turns a refreshable MV into an insert-trigger MV with entirely
    /// different semantics.
    ///
    /// Reported per-MV rather than raised, so one unrecognisable object degrades
    /// one row instead of the whole report. Aborting here would hide every MV
    /// after it, including any that had lost `APPEND` — a strictly worse outcome
    /// than the condition being reported.
    Unparseable(String),
    /// An MV that is NOT declared in the file but writes into one of the target
    /// tables the declared MVs own.
    ///
    /// Nothing in `rollups.sql` will ever mention it, so a check that only walks
    /// the file cannot see it — yet it is inserting into a coarse
    /// `ReplacingMergeTree` alongside the MV that is supposed to own that table.
    /// Given how often these objects have been dropped and re-created by hand
    /// (tasks 0090, 0095, 0136), a leftover is a plausible state.
    Undeclared,
}

/// Per-MV drift report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MvReport {
    /// Unqualified object name, e.g. `mv_ohlcv_1m_to_15m`.
    pub name: String,
    pub status: MvStatus,
    /// The live fingerprint, when the object exists. Carried so callers can
    /// check invariants that hold regardless of drift — notably
    /// [`MvFingerprint::is_append`], which stays worth reporting even when the
    /// file and the target agree.
    pub live: Option<MvFingerprint>,
}

impl MvReport {
    /// Whether this MV needs operator attention.
    pub fn needs_attention(&self) -> bool {
        !matches!(self.status, MvStatus::InSync)
            || self.live.as_ref().is_some_and(|f| !f.is_append())
    }
}

/// Compare every MV declared in `schema/rollups.sql` against the live
/// definitions on `database`.
///
/// Read-only. See [`check_mv_drift`] for the injectable-SQL form.
pub async fn check_rollup_drift(
    client: &Client,
    database: &str,
) -> Result<Vec<MvReport>, SchemaError> {
    check_mv_drift(client, database, ROLLUPS_SQL).await
}

/// Compare the MVs declared in `sql` against the live definitions on `database`.
///
/// `sql` is taken as the source of truth and its `prices.` qualifiers are
/// rewritten onto `database`, so a test can point the whole check at an isolated
/// scratch schema — and, more importantly, can feed in a *modified* copy of
/// `rollups.sql` to prove the check actually catches an edit rather than
/// reporting in-sync unconditionally.
pub async fn check_mv_drift(
    client: &Client,
    database: &str,
    sql: &str,
) -> Result<Vec<MvReport>, SchemaError> {
    let mut reports = Vec::new();
    let mut declared_names = Vec::new();
    let mut targets = Vec::new();

    for statement in split_statements(&rewrite_database(sql, database)) {
        let declared = fingerprint_via_server(client, &statement).await?;
        let short = short_name(&declared.name).to_string();

        // Recorded from the DECLARED side and before any early exit below: the
        // sweep for undeclared writers needs the target of an MV that is missing
        // from the target entirely, and needs the name of one whose live
        // definition could not be read — otherwise that object is reported twice,
        // once as unparseable and again as undeclared.
        declared_names.push(short.clone());
        targets.push(declared.target.clone());

        let Some(live_ddl) = fetch_live_ddl(client, database, &short).await? else {
            reports.push(MvReport {
                name: short,
                status: MvStatus::Missing,
                live: None,
            });
            continue;
        };

        // A live definition this module cannot read degrades THIS row only. The
        // declared side still raises (it is our own file, and the unit guards
        // pin its form), but an object someone re-created by hand must not be
        // able to blank out the rest of the report.
        let live = match fingerprint_via_server(client, &live_ddl).await {
            Ok(f) => f,
            Err(SchemaError::UnparsableDdl { rendering }) => {
                reports.push(MvReport {
                    name: short,
                    status: MvStatus::Unparseable(rendering),
                    live: None,
                });
                continue;
            }
            Err(e) => return Err(e),
        };

        let mut differences = Vec::new();
        for (field, d, l) in [
            (DriftField::Refresh, &declared.refresh, &live.refresh),
            (DriftField::Target, &declared.target, &live.target),
            (DriftField::Body, &declared.body, &live.body),
        ] {
            if d != l {
                differences.push(Difference {
                    field,
                    declared: d.clone(),
                    live: l.clone(),
                });
            }
        }

        reports.push(MvReport {
            name: short,
            status: if differences.is_empty() {
                MvStatus::InSync
            } else {
                MvStatus::Drifted(differences)
            },
            live: Some(live),
        });
    }

    reports.extend(undeclared_writers(client, database, &declared_names, &targets).await?);

    Ok(reports)
}

/// MVs on the target that write into one of `targets` but are not in
/// `declared_names`.
///
/// Walking only the file cannot find these, and the distinction matters for how
/// the summary reads: "six MVs in sync" is a statement about the six that were
/// looked for, not about what is writing into the coarse tables. A leftover MV
/// double-writing into a `ReplacingMergeTree` is a plausible state given how
/// often these have been dropped and re-created by hand.
async fn undeclared_writers(
    client: &Client,
    database: &str,
    declared_names: &[String],
    targets: &[String],
) -> Result<Vec<MvReport>, SchemaError> {
    let live: Vec<(String, String)> = client
        .query(
            "SELECT name, create_table_query FROM system.tables \
             WHERE database = ? AND engine = 'MaterializedView'",
        )
        .bind(database)
        .fetch_all::<(String, String)>()
        .await?;

    Ok(live
        .into_iter()
        .filter(|(name, _)| !declared_names.contains(name))
        .filter(|(_, ddl)| targets.iter().any(|t| writes_into(ddl, t)))
        .map(|(name, _)| MvReport {
            name,
            status: MvStatus::Undeclared,
            live: None,
        })
        .collect())
}

/// Whether `ddl` declares `TO <qualified_target>`.
///
/// The trailing-character check is load-bearing: the cluster carries `_bak`
/// copies of the coarse tables (task 0177), and a plain `contains` would read an
/// MV writing into `price_ohlcv_1d_bak` as one writing into `price_ohlcv_1d`.
fn writes_into(ddl: &str, qualified_target: &str) -> bool {
    let needle = format!(" TO {qualified_target}");
    ddl.match_indices(&needle).any(|(at, _)| {
        ddl[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_')
    })
}

/// Render `statement` through ClickHouse's AST serialiser and parse the result.
///
/// Both the file statement and `create_table_query` go through this, which is
/// the whole trick: the server normalises syntax that differs only in spelling
/// (`INTERVAL 15 MINUTE` vs `toIntervalMinute(15)`) so the comparison is
/// semantic without this crate having to implement a SQL parser.
async fn fingerprint_via_server(
    client: &Client,
    statement: &str,
) -> Result<MvFingerprint, SchemaError> {
    let rendered: String = client
        .query("SELECT formatQuerySingleLine(?)")
        .bind(statement)
        .fetch_all::<String>()
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| SchemaError::UnparsableDdl {
            rendering: statement.chars().take(200).collect(),
        })?;

    parse_fingerprint(&rendered).ok_or(SchemaError::UnparsableDdl {
        rendering: rendered,
    })
}

/// `None` when the object does not exist on the target.
async fn fetch_live_ddl(
    client: &Client,
    database: &str,
    name: &str,
) -> Result<Option<String>, SchemaError> {
    Ok(client
        .query("SELECT create_table_query FROM system.tables WHERE database = ? AND name = ?")
        .bind(database)
        .bind(name)
        .fetch_all::<String>()
        .await?
        .into_iter()
        .next())
}

/// Point `prices.`-qualified SQL at another schema, so the check can run against
/// an isolated scratch database in tests. A no-op for the production name.
fn rewrite_database(sql: &str, database: &str) -> String {
    if database == crate::PROD_DATABASE {
        return sql.to_string();
    }
    sql.replace("prices.", &format!("{database}."))
}

/// `prices.mv_ohlcv_1m_to_15m` → `mv_ohlcv_1m_to_15m`.
fn short_name(qualified: &str) -> &str {
    qualified.rsplit('.').next().unwrap_or(qualified)
}

/// Parse ClickHouse's single-line rendering of a `CREATE MATERIALIZED VIEW`.
///
/// Deliberately *not* a general SQL parser: it only ever sees output of
/// `formatQuerySingleLine`, whose shape is
///
/// ```text
/// CREATE MATERIALIZED VIEW [IF NOT EXISTS] <name> REFRESH <refresh> TO <target>[ <cols>][ DEFINER…] AS SELECT …
/// ```
///
/// Everything between the `<target>` token and ` AS SELECT` is server-injected
/// (the target's column list, the DEFINER clause) and is skipped by taking the
/// target as a single token and then jumping to the body — see the module docs.
///
/// Returns `None` on anything that is not a materialized-view DDL, so a
/// malformed rendering surfaces as an error rather than as a false in-sync.
fn parse_fingerprint(rendered: &str) -> Option<MvFingerprint> {
    let rest = rendered.trim().strip_prefix("CREATE MATERIALIZED VIEW ")?;
    let rest = rest.strip_prefix("IF NOT EXISTS ").unwrap_or(rest);

    let (name, rest) = rest.split_once(' ')?;
    let rest = rest.strip_prefix("REFRESH ")?;

    // First ` TO ` after the refresh clause: a table name cannot contain it, and
    // no refresh clause ClickHouse renders does either.
    let (refresh, rest) = rest.split_once(" TO ")?;

    // The target is one token; the column list and DEFINER clause that may
    // follow it are server-derived and intentionally not compared.
    let target = rest.split_whitespace().next()?;

    // `AS SELECT` cannot appear inside the head, so the first occurrence is the
    // body boundary.
    let body_at = rest.find(" AS SELECT")? + " AS ".len();

    Some(MvFingerprint {
        name: name.to_string(),
        refresh: refresh.trim().to_string(),
        target: target.to_string(),
        body: rest[body_at..].trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ClickHouse's rendering of the live `mv_ohlcv_1m_to_15m`, captured verbatim
    /// from `system.tables` on 26.3.10.60. Carries all three server injections
    /// the parser must skip: no `IF NOT EXISTS`, an injected column list, and a
    /// `DEFINER` clause.
    const LIVE: &str = "CREATE MATERIALIZED VIEW prices.mv_ohlcv_1m_to_15m REFRESH EVERY 1 MINUTE APPEND TO prices.price_ohlcv_15m (`timestamp` DateTime, `asset_id` UInt32, `close_usd` Decimal(38, 14), `vwap` Nullable(Decimal(38, 14)), `version` UInt64) DEFINER = default SQL SECURITY DEFINER AS SELECT toStartOfInterval(t.timestamp, toIntervalMinute(15)) AS timestamp, argMax(close_usd, t.timestamp) AS close_usd, sum(version) AS version FROM prices.price_ohlcv_1m AS t FINAL WHERE t.timestamp >= toStartOfInterval(now() - toIntervalHour(2), toIntervalMinute(15)) GROUP BY timestamp, asset_id";

    /// The same statement as the file declares it, after `formatQuerySingleLine`:
    /// `IF NOT EXISTS` survives, and neither injection is present.
    const DECLARED: &str = "CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1m_to_15m REFRESH EVERY 1 MINUTE APPEND TO prices.price_ohlcv_15m AS SELECT toStartOfInterval(t.timestamp, toIntervalMinute(15)) AS timestamp, argMax(close_usd, t.timestamp) AS close_usd, sum(version) AS version FROM prices.price_ohlcv_1m AS t FINAL WHERE t.timestamp >= toStartOfInterval(now() - toIntervalHour(2), toIntervalMinute(15)) GROUP BY timestamp, asset_id";

    /// The point of the whole module: the two renderings of one unedited MV must
    /// fingerprint identically, despite `IF NOT EXISTS`, the injected column list
    /// and the DEFINER clause. Without this the check is permanently red and
    /// therefore useless.
    #[test]
    fn the_server_injections_do_not_register_as_drift() {
        let live = parse_fingerprint(LIVE).expect("live rendering parses");
        let declared = parse_fingerprint(DECLARED).expect("declared rendering parses");

        assert_eq!(declared.name, live.name);
        assert_eq!(declared.refresh, live.refresh);
        assert_eq!(declared.target, live.target);
        assert_eq!(declared.body, live.body, "bodies must compare equal");
        assert_eq!(declared, live);
    }

    #[test]
    fn a_fingerprint_names_the_object_the_target_and_the_refresh_clause() {
        let f = parse_fingerprint(LIVE).expect("parses");
        assert_eq!(f.name, "prices.mv_ohlcv_1m_to_15m");
        assert_eq!(f.refresh, "EVERY 1 MINUTE APPEND");
        assert_eq!(f.target, "prices.price_ohlcv_15m");
        assert!(f.body.starts_with("SELECT toStartOfInterval("));
        assert!(
            f.body.ends_with("GROUP BY timestamp, asset_id"),
            "body must run to the end of the statement, got tail: {}",
            &f.body[f.body.len().saturating_sub(60)..]
        );
        // The injections must not have leaked into any field.
        assert!(!f.body.contains("DEFINER"), "DEFINER leaked into the body");
        assert!(!f.body.contains("Decimal(38, 14)"), "column list leaked");
    }

    /// Non-vacuity for the comparison itself: an edit to the body — the exact
    /// change `IF NOT EXISTS` swallows on a provisioned target — must register.
    /// This is the guarded `close_usd` projection task 0146 needs to land.
    #[test]
    fn an_edited_body_does_not_fingerprint_as_in_sync() {
        let edited = DECLARED.replace(
            "argMax(close_usd, t.timestamp)",
            "argMaxIf(close_usd, t.timestamp, close_usd > 0)",
        );
        assert_ne!(edited, DECLARED, "the test's own edit must apply");

        let live = parse_fingerprint(LIVE).expect("parses");
        let declared = parse_fingerprint(&edited).expect("parses");

        assert_ne!(declared.body, live.body);
        assert_ne!(declared, live);
    }

    /// A changed cadence must register too — `REFRESH` is where an accidental
    /// re-CREATE would most plausibly diverge from the file.
    #[test]
    fn an_edited_refresh_clause_does_not_fingerprint_as_in_sync() {
        let edited = DECLARED.replace("REFRESH EVERY 1 MINUTE", "REFRESH EVERY 5 MINUTE");
        let declared = parse_fingerprint(&edited).expect("parses");
        let live = parse_fingerprint(LIVE).expect("parses");

        assert_eq!(declared.body, live.body, "only the refresh clause changed");
        assert_ne!(declared.refresh, live.refresh);
        assert_ne!(declared, live);
    }

    /// Losing `APPEND` is not ordinary drift — a replace-mode refreshable MV
    /// overwrites its whole target with the bounded window on every tick, which
    /// is the task 0090 production data loss. It must be visible as its own
    /// condition, not merely as "the refresh clause differs".
    #[test]
    fn a_replace_mode_mv_is_not_append() {
        let appending = parse_fingerprint(LIVE).expect("parses");
        assert!(appending.is_append());

        let replacing =
            parse_fingerprint(&LIVE.replace("EVERY 1 MINUTE APPEND TO", "EVERY 1 MINUTE TO"))
                .expect("parses");
        assert!(
            !replacing.is_append(),
            "an MV with no APPEND keyword must not report as append mode"
        );
        assert_eq!(
            replacing.refresh, "EVERY 1 MINUTE",
            "the APPEND keyword is part of the refresh clause, so its loss is drift too"
        );
    }

    /// `needs_attention` must fire on a live replace-mode MV even when the file
    /// agrees with it — an in-sync file is no defence if what both hold is the
    /// data-destroying form.
    #[test]
    fn an_in_sync_but_replace_mode_mv_still_needs_attention() {
        let replacing =
            parse_fingerprint(&LIVE.replace("EVERY 1 MINUTE APPEND TO", "EVERY 1 MINUTE TO"))
                .expect("parses");
        let report = MvReport {
            name: "mv_ohlcv_1m_to_15m".to_string(),
            status: MvStatus::InSync,
            live: Some(replacing),
        };
        assert!(report.needs_attention());

        let healthy = MvReport {
            name: "mv_ohlcv_1m_to_15m".to_string(),
            status: MvStatus::InSync,
            live: parse_fingerprint(LIVE),
        };
        assert!(!healthy.needs_attention());
    }

    #[test]
    fn a_missing_mv_needs_attention() {
        let report = MvReport {
            name: "mv_ohlcv_1m_to_15m".to_string(),
            status: MvStatus::Missing,
            live: None,
        };
        assert!(report.needs_attention());
    }

    #[test]
    fn non_materialized_view_ddl_does_not_parse() {
        assert!(parse_fingerprint("CREATE OR REPLACE VIEW prices.foo AS SELECT 1").is_none());
        assert!(parse_fingerprint("CREATE TABLE prices.foo (x Int64) ENGINE = Memory").is_none());
        // A materialized view with no TO target is not one of ours, and silently
        // treating it as parseable would compare the wrong things.
        assert!(
            parse_fingerprint(
                "CREATE MATERIALIZED VIEW prices.foo REFRESH EVERY 1 DAY APPEND ENGINE = Memory AS SELECT 1"
            )
            .is_none()
        );
    }

    /// The scratch-database rewrite the integration test depends on. A rewrite
    /// that missed the target table or the object name would compare an MV in one
    /// schema against a declaration in another and report noise.
    #[test]
    fn rewrite_database_moves_every_qualifier_and_leaves_prod_alone() {
        let sql = "CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_x REFRESH EVERY 1 DAY APPEND \
                   TO prices.price_ohlcv_1d AS SELECT 1 FROM prices.price_ohlcv_4h AS t FINAL";
        let moved = rewrite_database(sql, "scratch_42");
        assert!(
            !moved.contains("prices."),
            "left a prices. qualifier: {moved}"
        );
        assert_eq!(moved.matches("scratch_42.").count(), 3);

        assert_eq!(rewrite_database(sql, crate::PROD_DATABASE), sql);
    }

    /// The trailing-character guard, which is not decoration: the cluster
    /// carries `_bak` copies of the coarse tables (task 0177), so a plain
    /// `contains` would read a backup-writing MV as a second writer into the
    /// live table and report a false `Undeclared`.
    #[test]
    fn writes_into_does_not_match_a_bak_table_of_the_same_stem() {
        let live =
            "CREATE MATERIALIZED VIEW prices.mv_x TO prices.price_ohlcv_1d (`a` Int64) AS SELECT 1";
        let bak = "CREATE MATERIALIZED VIEW prices.mv_x TO prices.price_ohlcv_1d_bak (`a` Int64) AS SELECT 1";

        assert!(writes_into(live, "prices.price_ohlcv_1d"));
        assert!(
            !writes_into(bak, "prices.price_ohlcv_1d"),
            "price_ohlcv_1d_bak must not match price_ohlcv_1d"
        );
        assert!(writes_into(bak, "prices.price_ohlcv_1d_bak"));
    }

    #[test]
    fn writes_into_ignores_an_unrelated_target() {
        let ddl = "CREATE MATERIALIZED VIEW prices.mv_current_prices REFRESH EVERY 1 MINUTE TO prices.current_prices (`a` Int64) AS SELECT 1";
        assert!(!writes_into(ddl, "prices.price_ohlcv_15m"));
        assert!(writes_into(ddl, "prices.current_prices"));
    }

    /// Both new statuses must count as needing attention. `Unparseable` in
    /// particular: it is the one status where the tool is admitting it does not
    /// know, and silence there would be indistinguishable from health.
    #[test]
    fn an_unparseable_or_undeclared_mv_needs_attention() {
        for status in [
            MvStatus::Unparseable("CREATE MATERIALIZED VIEW …".to_string()),
            MvStatus::Undeclared,
        ] {
            let report = MvReport {
                name: "mv_ohlcv_1m_to_15m".to_string(),
                status,
                live: None,
            };
            assert!(report.needs_attention(), "{:?}", report.status);
        }
    }

    /// A refreshable MV re-created WITHOUT `REFRESH` is an insert-trigger MV —
    /// different semantics entirely — and must not parse as one of ours.
    #[test]
    fn a_materialized_view_with_no_refresh_clause_does_not_parse() {
        let insert_trigger = "CREATE MATERIALIZED VIEW prices.mv_ohlcv_1m_to_15m TO prices.price_ohlcv_15m (`timestamp` DateTime) AS SELECT 1";
        assert!(parse_fingerprint(insert_trigger).is_none());
    }

    #[test]
    fn short_name_strips_the_schema() {
        assert_eq!(
            short_name("prices.mv_ohlcv_1m_to_15m"),
            "mv_ohlcv_1m_to_15m"
        );
        assert_eq!(short_name("mv_ohlcv_1m_to_15m"), "mv_ohlcv_1m_to_15m");
    }

    /// Every statement in the shipped file must be parseable as an MV once the
    /// server has rendered it — asserted here on the raw file text as a cheap
    /// pre-check, so a statement added in a form this module cannot fingerprint
    /// fails the unit suite rather than silently dropping out of the drift report.
    #[test]
    fn every_shipped_rollup_statement_declares_a_materialized_view_with_a_target() {
        let stmts = split_statements(ROLLUPS_SQL);
        assert_eq!(stmts.len(), 6, "guard is vacuous if the file is empty");
        for stmt in &stmts {
            let head: String = stmt.chars().take(80).collect();
            assert!(
                stmt.starts_with("CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_"),
                "unexpected statement form: {head}"
            );
            assert!(
                stmt.contains("\nTO prices.price_ohlcv_"),
                "no target: {head}"
            );
            assert!(stmt.contains(" AS\nSELECT"), "no select body: {head}");
        }
    }
}
