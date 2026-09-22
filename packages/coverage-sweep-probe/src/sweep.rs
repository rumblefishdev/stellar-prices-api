//! The sweep query and the Rust-side subtraction of the allow-list (task 0100).
//!
//! One statement over BE's `soroban_events` (the swap/trade-shaped events of
//! the window) joined to `soroban_contracts` (strkey + wasm) and filtered
//! against `prices.pool_registry`. The allow-list is subtracted afterwards in
//! Rust, never in SQL: it stays out of the query text and is testable on its
//! own.
//!
//! Pinned facts behind the query (measured on production 2026-09-21):
//! - it filters on `topics_xdr`, **never `signature`** — that column is NULL
//!   for most string-topic events (0285: 272,144 of 310,829), so it would miss
//!   Soroswap and Phoenix;
//! - it matches topic[0] **and** topic[1]: Soroswap (`SoroswapPair`/`swap`),
//!   Comet-style (`POOL`/`swap`) and routers carry the action in topic[1];
//! - only `sym`/`string` topics may match: an Address strkey can spell `SWAP`
//!   (`GAA574…SWAPN…`), which pulled SAC `transfer`/`mint` events into a first
//!   draft;
//! - ClickHouse JSON indexes are 1-based: index 1 is topic[0].
//! - only CONTRACT events (`event_type = 1`): BE's table can also hold system
//!   (0) and diagnostic (2) events, and a diagnostic `fn_return` carries the
//!   invoked function's name at topic[1] — a call to `swap` on a contract that
//!   emits no swap would otherwise match (review of PR #332). Measured
//!   2026-09-21: every matching row was type 1, so this changes no result.

use crate::allowlist::AllowList;

/// The trailing window, in ledgers (decision D2: weekly run, 14-day window).
///
/// Measured on production 2026-09-21: the 14 days to ledger 64,541,178 began at
/// ledger 64,320,000, i.e. 64,541,178 − 64,320,000 = 221,178 ledgers
/// (≈ 15,798 ledgers/day, ≈ 5.47 s/ledger). This is the *span*: the window
/// is inclusive at both ends (`BETWEEN lo AND hi`, exactly as measured), so it
/// holds 221,179 ledgers. A 14-day window over a weekly
/// cadence means every week is seen twice, so a run that fails once loses
/// nothing.
pub const SWEEP_WINDOW_LEDGERS: i64 = 221_178;

/// One contract the query returned: a swap/trade emitter in the window that
/// is not in `pool_registry`.
///
/// ⚠️ RowBinary is positional: the field order must equal the SELECT column
/// order of [`sweep_sql`].
#[derive(Debug, Clone, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct SweepRow {
    /// The `C…` strkey, or `""` when BE's `soroban_contracts` has no row for
    /// the contract (the LEFT JOIN leaves the default).
    pub strkey: String,
    /// The wasm hash, lowercase hex; `None` for a SAC or an unresolved contract.
    pub wasm: Option<String>,
    /// BE's surrogate `soroban_events.contract_id`, so an unresolved contract
    /// stays identifiable.
    pub contract_surrogate: i64,
    pub events: u64,
    pub txs: u64,
    pub first_ledger: i64,
    pub last_ledger: i64,
    /// Most frequent `topic[0] / topic[1]` (topic[1] cut to 20 chars).
    pub top_action: String,
    /// Most frequent `topic types -> data type`.
    pub top_shape: String,
}

impl SweepRow {
    /// The strkey, or `unresolved:<surrogate>` when BE has not resolved it.
    pub fn display_id(&self) -> String {
        if self.strkey.is_empty() {
            format!("unresolved:{}", self.contract_surrogate)
        } else {
            self.strkey.clone()
        }
    }
}

/// Why a sweep failed. Every variant fails the invocation: the unclassified
/// alarm is NOT_BREACHING, so an error mapped to "nothing found" would read
/// healthy forever.
#[derive(Debug, thiserror::Error)]
pub enum SweepError {
    #[error("not a bare SQL identifier: {0:?}")]
    InvalidIdentifier(String),
    #[error(
        "soroban_events is empty (max(ledger_sequence) = 0): the probe is misconfigured, not the chain healthy"
    )]
    EmptyEventsTable,
    #[error("clickhouse query failed: {0}")]
    Query(#[from] clickhouse::error::Error),
}

/// `^[A-Za-z_][A-Za-z0-9_]*$` — a bare SQL identifier, which needs no quoting
/// and cannot end the token it is spliced into. Copied from
/// `prices-clickhouse/src/rollup_sql.rs` (private there).
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn identifier(s: &str) -> Result<&str, SweepError> {
    if is_identifier(s) {
        Ok(s)
    } else {
        Err(SweepError::InvalidIdentifier(s.to_string()))
    }
}

/// The sweep statement. `be_db` holds `soroban_events`/`soroban_contracts`
/// (production: `default`), `prices_db` holds `pool_registry` (production:
/// `prices`). The ledger bounds are server-side typed parameters `{lo:Int64}`
/// and `{hi:Int64}`, inclusive; they are never spliced into the text.
///
/// A contract BE has not resolved yet (no `soroban_contracts` row) must still
/// be reported, as `unresolved:<surrogate>`. That relies on the LEFT JOIN miss
/// reading `''`, which is `join_use_nulls = 0`; under `= 1` the miss is NULL,
/// `NULL NOT IN (…)` is NULL and the row would vanish silently. Hence both the
/// `ifNull` and the pinned setting — the statement does not depend on the
/// session's default (review WR-03).
pub fn sweep_sql(be_db: &str, prices_db: &str) -> Result<String, SweepError> {
    let be = identifier(be_db)?;
    let pr = identifier(prices_db)?;
    Ok(format!(
        "WITH \
    ev AS ( \
        SELECT contract_id, ledger_sequence, transaction_id, \
               JSONExtractString(topics_xdr, 1, 'value') AS t0, \
               JSONExtractString(topics_xdr, 2, 'value') AS t1, \
               arrayStringConcat(arrayMap(x -> JSONExtractString(x, 'type'), \
                                          JSONExtractArrayRaw(topics_xdr)), '|') AS topic_types, \
               JSONExtractString(data_xdr, 'type') AS data_type \
        FROM {be}.soroban_events \
        WHERE ledger_sequence BETWEEN {{lo:Int64}} AND {{hi:Int64}} \
          AND event_type = 1 \
          AND ((JSONExtractString(topics_xdr, 1, 'type') IN ('sym', 'string') \
                AND match(lower(JSONExtractString(topics_xdr, 1, 'value')), 'swap|trade')) \
            OR (JSONExtractString(topics_xdr, 2, 'type') IN ('sym', 'string') \
                AND match(lower(JSONExtractString(topics_xdr, 2, 'value')), 'swap|trade'))) \
    ), \
    per_contract AS ( \
        SELECT contract_id, count() AS events, uniqExact(transaction_id) AS txs, \
               min(ledger_sequence) AS first_ledger, max(ledger_sequence) AS last_ledger, \
               topK(1)(concat(t0, ' / ', left(t1, 20)))[1] AS top_action, \
               topK(1)(concat(topic_types, ' -> ', data_type))[1] AS top_shape \
        FROM ev GROUP BY contract_id \
    ) \
SELECT ifNull(c.contract_id, '') AS strkey, lower(hex(c.wasm_hash)) AS wasm, \
       p.contract_id AS contract_surrogate, \
       p.events, p.txs, p.first_ledger, p.last_ledger, p.top_action, p.top_shape \
FROM per_contract p \
LEFT JOIN (SELECT id, contract_id, wasm_hash FROM {be}.soroban_contracts FINAL) c \
       ON c.id = p.contract_id \
WHERE ifNull(c.contract_id, '') NOT IN (SELECT contract_id FROM {pr}.pool_registry FINAL) \
ORDER BY p.events DESC, contract_surrogate \
SETTINGS join_use_nulls = 0"
    ))
}

/// The top of BE's event table: the window's upper bound.
pub fn max_ledger_sql(be_db: &str) -> Result<String, SweepError> {
    let be = identifier(be_db)?;
    Ok(format!(
        "SELECT max(ledger_sequence) FROM {be}.soroban_events"
    ))
}

/// The inclusive window `[max(max_ledger − SWEEP_WINDOW_LEDGERS, 0), max_ledger]`.
pub fn window(max_ledger: i64) -> (i64, i64) {
    ((max_ledger - SWEEP_WINDOW_LEDGERS).max(0), max_ledger)
}

/// The outcome of one sweep.
#[derive(Debug, Clone)]
pub struct SweepReport {
    pub max_ledger: i64,
    pub lo: i64,
    pub hi: i64,
    /// Rows the query returned (unregistered emitters, before the allow-list).
    pub rows_total: usize,
    /// Rows no allow-list entry covers: the residual that pages.
    pub unclassified: Vec<SweepRow>,
    /// Rows an entry covers, with the matching key (`contract:<id>` /
    /// `wasm:<hash>`).
    pub allowlisted: Vec<(SweepRow, String)>,
}

/// Split the query's rows into `(unclassified, allowlisted)`.
pub fn partition(
    rows: Vec<SweepRow>,
    allow: &AllowList,
) -> (Vec<SweepRow>, Vec<(SweepRow, String)>) {
    let mut unclassified = Vec::new();
    let mut allowlisted = Vec::new();
    for row in rows {
        match allow.match_row(&row.strkey, row.wasm.as_deref()) {
            Some(key) => allowlisted.push((row, key)),
            None => unclassified.push(row),
        }
    }
    (unclassified, allowlisted)
}

/// Run one sweep: read the top ledger, query the window, subtract the
/// allow-list. Every ClickHouse error propagates; nothing is mapped to an
/// empty result.
pub async fn run_sweep(
    client: &clickhouse::Client,
    be_db: &str,
    prices_db: &str,
    allow: &AllowList,
) -> Result<SweepReport, SweepError> {
    let sql = sweep_sql(be_db, prices_db)?;
    let max_ledger: i64 = client
        .query(&max_ledger_sql(be_db)?)
        .fetch_one::<i64>()
        .await?;
    if max_ledger <= 0 {
        return Err(SweepError::EmptyEventsTable);
    }
    let (lo, hi) = window(max_ledger);
    let rows = client
        .query(&sql)
        .param("lo", lo)
        .param("hi", hi)
        .fetch_all::<SweepRow>()
        .await?;
    let rows_total = rows.len();
    let (unclassified, allowlisted) = partition(rows, allow);
    Ok(SweepReport {
        max_ledger,
        lo,
        hi,
        rows_total,
        unclassified,
        allowlisted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(strkey: &str, surrogate: i64) -> SweepRow {
        SweepRow {
            strkey: strkey.to_string(),
            wasm: None,
            contract_surrogate: surrogate,
            events: 1,
            txs: 1,
            first_ledger: 1,
            last_ledger: 1,
            top_action: String::new(),
            top_shape: String::new(),
        }
    }

    fn prod_sql() -> String {
        sweep_sql("default", "prices").unwrap()
    }

    #[test]
    fn matches_topic_0_and_topic_1_by_1_based_json_index_of_sym_or_string_type() {
        let sql = prod_sql();
        assert!(sql.contains("JSONExtractString(topics_xdr, 1, 'type') IN ('sym', 'string')"));
        assert!(sql.contains("JSONExtractString(topics_xdr, 2, 'type') IN ('sym', 'string')"));
        assert!(
            sql.contains("match(lower(JSONExtractString(topics_xdr, 1, 'value')), 'swap|trade')")
        );
        assert!(
            sql.contains("match(lower(JSONExtractString(topics_xdr, 2, 'value')), 'swap|trade')")
        );
        // No index 0: ClickHouse JSON indexes are 1-based.
        assert!(!sql.contains("topics_xdr, 0,"));
    }

    #[test]
    fn reads_contract_events_only() {
        // A diagnostic `fn_return` names the invoked function at topic[1];
        // without this a call to `swap` would count as a swap event.
        assert!(prod_sql().contains("AND event_type = 1"));
    }

    #[test]
    fn never_filters_on_signature() {
        assert!(!prod_sql().to_lowercase().contains("signature"));
    }

    #[test]
    fn bounds_are_typed_server_side_parameters() {
        let sql = prod_sql();
        assert!(sql.contains("ledger_sequence BETWEEN {lo:Int64} AND {hi:Int64}"));
        // `.bind()` would rewrite every `?`; the SQL must carry none.
        assert!(!sql.contains('?'));
        // No ledger number is spliced in.
        assert!(!sql.contains("64320000") && !sql.contains("221178"));
    }

    #[test]
    fn lookup_tables_are_read_with_final() {
        let sql = prod_sql();
        assert!(sql.contains("FROM default.soroban_contracts FINAL"));
        assert!(sql.contains("FROM prices.pool_registry FINAL"));
        assert!(sql.contains("FROM default.soroban_events"));
    }

    #[test]
    fn database_qualifiers_follow_the_arguments() {
        let sql = sweep_sql("be_x", "pr_y").unwrap();
        assert!(
            !sql.contains("default.") && !sql.contains("prices."),
            "{sql}"
        );
        assert!(sql.contains("be_x.soroban_events"));
        assert!(sql.contains("be_x.soroban_contracts"));
        assert!(sql.contains("pr_y.pool_registry"));
        assert_eq!(
            max_ledger_sql("be_x").unwrap(),
            "SELECT max(ledger_sequence) FROM be_x.soroban_events"
        );
    }

    #[test]
    fn non_identifier_database_names_are_refused() {
        for (be, pr) in [
            ("default; SELECT 1", "prices"),
            ("prices", ""),
            ("1db", "prices"),
            ("default", "prices.x"),
            ("de`fault", "prices"),
        ] {
            assert!(
                matches!(sweep_sql(be, pr), Err(SweepError::InvalidIdentifier(_))),
                "{be:?}/{pr:?}"
            );
        }
        assert!(matches!(
            max_ledger_sql("default; DROP"),
            Err(SweepError::InvalidIdentifier(_))
        ));
    }

    #[test]
    fn select_order_matches_the_row_struct() {
        // RowBinary is positional: the SELECT list must follow SweepRow.
        let sql = prod_sql();
        let select = &sql[sql.rfind("SELECT ifNull(c.contract_id").unwrap()
            ..sql.find("FROM per_contract").unwrap()];
        let cols = [
            "AS strkey",
            "AS wasm",
            "AS contract_surrogate",
            "p.events",
            "p.txs",
            "p.first_ledger",
            "p.last_ledger",
            "p.top_action",
            "p.top_shape",
        ];
        let pos: Vec<usize> = cols.iter().map(|c| select.find(c).expect(c)).collect();
        assert!(pos.windows(2).all(|w| w[0] < w[1]), "{select}");
    }

    #[test]
    fn unresolved_contracts_do_not_depend_on_join_use_nulls() {
        let sql = prod_sql();
        assert!(sql.contains("ifNull(c.contract_id, '') AS strkey"));
        assert!(sql.contains("WHERE ifNull(c.contract_id, '') NOT IN"));
        assert!(sql.trim_end().ends_with("SETTINGS join_use_nulls = 0"));
    }

    #[test]
    fn window_constant_is_the_measured_14_days() {
        assert_eq!(SWEEP_WINDOW_LEDGERS, 64_541_178 - 64_320_000);
    }

    #[test]
    fn window_is_inclusive_and_saturates_at_zero() {
        assert_eq!(window(64_541_178), (64_320_000, 64_541_178));
        assert_eq!(window(100), (0, 100));
        assert_eq!(window(SWEEP_WINDOW_LEDGERS), (0, SWEEP_WINDOW_LEDGERS));
    }

    #[test]
    fn display_id_names_unresolved_contracts_by_surrogate() {
        assert_eq!(row("CABC", 7).display_id(), "CABC");
        assert_eq!(row("", 42).display_id(), "unresolved:42");
    }
}
