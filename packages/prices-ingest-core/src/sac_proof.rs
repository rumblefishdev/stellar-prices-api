//! BE reads behind the SAC resolver (task 0242): which contracts BE flags as a
//! Stellar Asset Contract. A flagged contract no proof resolves is skipped, not
//! minted as a `Contract` identity (D2).
//!
//! No `SETTINGS` in any statement here: a read that crosses the client's GET
//! length is sent as POST with `readonly=1`, which rejects per-query settings
//! (code 164; see events-backfill `source.rs`).

use std::collections::HashSet;

use crate::error::IngestError;

/// BE's database, holding `soroban_contracts` and `soroban_events`.
pub const BE_DATABASE: &str = "default";
/// Ours, holding `assets`.
pub const PRICES_DATABASE: &str = prices_clickhouse::PROD_DATABASE;

/// `^[A-Za-z_][A-Za-z0-9_]*$`: a bare identifier, safe to splice unquoted.
fn identifier(s: &str) -> Result<&str, IngestError> {
    let mut chars = s.chars();
    let ok = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(s)
    } else {
        Err(IngestError::Precondition(format!(
            "not a database identifier: {s:?}"
        )))
    }
}

/// Every contract BE flags `is_sac`, once each.
///
/// No `FINAL`, on purpose. `soroban_contracts` is a
/// `ReplacingMergeTree(wasm_uploaded_at_ledger)`, so a contract BE re-inserts
/// with `is_sac = false` stays in this set until BE's parts merge. That errs
/// toward D2: its trades are skipped and counted, never minted. `FINAL` would
/// merge BE's whole table on every live cold start, a read that fails Init
/// (PC3), to shorten a case that is rare because `is_sac` is fixed at deploy.
pub(crate) fn sac_contracts_sql(be_db: &str) -> Result<String, IngestError> {
    let be = identifier(be_db)?;
    Ok(format!(
        "SELECT DISTINCT contract_id FROM {be}.soroban_contracts WHERE is_sac"
    ))
}

/// Load the `is_sac` contract set (about 4,000 addresses on production).
pub async fn load_sac_contracts(
    client: &clickhouse::Client,
    be_db: &str,
) -> Result<HashSet<String>, IngestError> {
    let rows = client
        .query(&sac_contracts_sql(be_db)?)
        .fetch_all::<String>()
        .await?;
    Ok(rows.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sac_contracts_sql_reads_bes_is_sac_set_without_settings() {
        let sql = sac_contracts_sql("default").unwrap();
        assert!(sql.contains("FROM default.soroban_contracts"), "{sql}");
        assert!(sql.contains("WHERE is_sac"), "{sql}");
        assert!(!sql.contains("SETTINGS"), "{sql}");
        assert!(!sql.contains("FINAL"), "{sql}");
    }

    #[test]
    fn a_non_identifier_database_is_refused() {
        for name in ["", "1db", "default; DROP", "de.fault", "db`"] {
            assert!(
                matches!(sac_contracts_sql(name), Err(IngestError::Precondition(_))),
                "{name:?}"
            );
        }
        assert!(sac_contracts_sql("it_0242_be").is_ok());
    }
}
