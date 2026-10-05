//! BE reads behind the SAC resolver (task 0242): which contracts BE flags as a
//! Stellar Asset Contract, and one proof event per flagged contract our
//! registry cannot resolve yet. A flagged contract no proof resolves is
//! skipped, not minted as a `Contract` identity (D2).
//!
//! No `SETTINGS` in any statement here: a read that crosses the client's GET
//! length is sent as POST with `readonly=1`, which rejects per-query settings
//! (code 164; see events-backfill `source.rs`).

use std::collections::HashSet;

use crate::canonical::AssetRegistry;
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

/// One SAC-signature event per `is_sac` contract whose address is no
/// `sac_address` in `<prices>.assets`: the contract and its event's last topic
/// (the SEP-11 asset). Without the "not resolvable" filter the scan times out
/// on production (0242 RESEARCH M16b).
pub(crate) fn sac_proof_preload_sql(be_db: &str, prices_db: &str) -> Result<String, IngestError> {
    let be = identifier(be_db)?;
    let prices = identifier(prices_db)?;
    Ok(format!(
        "SELECT sc.contract_id AS contract_id, \
                JSONExtractString(x.topics_xdr, JSONLength(x.topics_xdr), 'value') AS sep11 \
         FROM (SELECT contract_id, topics_xdr FROM {be}.soroban_events \
               WHERE contract_id IN (SELECT id FROM {be}.soroban_contracts \
                                     WHERE is_sac AND contract_id NOT IN \
                                       (SELECT sac_address FROM {prices}.assets FINAL WHERE sac_address != '')) \
                 AND signature IN ('transfer', 'mint', 'burn', 'clawback') \
               LIMIT 1 BY contract_id) x \
         INNER JOIN (SELECT id, any(contract_id) AS contract_id FROM {be}.soroban_contracts \
                     WHERE is_sac GROUP BY id) sc ON sc.id = x.contract_id"
    ))
}

/// A row of [`sac_proof_preload_sql`], fields in SELECT order.
#[derive(Debug, Clone, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct SacProofRow {
    pub contract_id: String,
    pub sep11: String,
}

/// Load the proofs events-backfill feeds to `learn_sac` before its first chunk
/// (about 1,100 rows in 2.6 s on production).
pub async fn load_sac_proofs(
    client: &clickhouse::Client,
    be_db: &str,
    prices_db: &str,
) -> Result<Vec<SacProofRow>, IngestError> {
    Ok(client
        .query(&sac_proof_preload_sql(be_db, prices_db)?)
        .fetch_all::<SacProofRow>()
        .await?)
}

/// Feed every proof through `learn_sac`: (verified, rejected).
pub fn apply_sac_proofs(assets: &mut AssetRegistry, rows: &[SacProofRow]) -> (u64, u64) {
    let verified = rows
        .iter()
        .filter(|r| assets.learn_sac(&r.contract_id, &r.sep11))
        .count() as u64;
    (verified, rows.len() as u64 - verified)
}

/// What [`preload_sac_resolver`] armed a registry with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SacPreload {
    /// BE's `is_sac` contracts, now the registry's candidates.
    pub candidates: usize,
    /// Proof rows read.
    pub proofs: usize,
    /// Proofs `learn_sac` accepted, and the ones it did not.
    pub verified: u64,
    pub rejected: u64,
}

/// events-backfill's run-start step (task 0242 D1): BE's `is_sac` set as the
/// registry's candidates, then one proof per SAC `assets` cannot resolve,
/// through `learn_sac`. That path streams pool events only, so this is how it
/// resolves a SAC whose classic is not in `assets` yet.
pub async fn preload_sac_resolver(
    client: &clickhouse::Client,
    be_db: &str,
    prices_db: &str,
    assets: &mut AssetRegistry,
) -> Result<SacPreload, IngestError> {
    let candidates = load_sac_contracts(client, be_db).await?;
    let n = candidates.len();
    assets.set_sac_candidates(candidates);
    let proofs = load_sac_proofs(client, be_db, prices_db).await?;
    let (verified, rejected) = apply_sac_proofs(assets, &proofs);
    Ok(SacPreload {
        candidates: n,
        proofs: proofs.len(),
        verified,
        rejected,
    })
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
        for (be, prices) in [
            ("default; DROP", "prices"),
            ("default", "pri ces"),
            ("", ""),
        ] {
            assert!(
                matches!(
                    sac_proof_preload_sql(be, prices),
                    Err(IngestError::Precondition(_))
                ),
                "{be:?} {prices:?}"
            );
        }
    }

    #[test]
    fn sac_proof_preload_sql_reads_one_proof_per_unresolvable_sac_without_settings() {
        let sql = sac_proof_preload_sql("default", "prices").unwrap();
        for part in [
            "LIMIT 1 BY contract_id",
            "signature IN ('transfer', 'mint', 'burn', 'clawback')",
            "NOT IN (SELECT sac_address FROM prices.assets FINAL WHERE sac_address != '')",
            "default.soroban_events",
            "default.soroban_contracts",
            "JSONLength(",
        ] {
            assert!(sql.contains(part), "{part} in {sql}");
        }
        assert!(!sql.contains("SETTINGS"), "{sql}");
    }

    const XCR_ISSUER: &str = "GBLJBHWVORDFI4J7CLBDRPECMYT3XO5S6GERXGC74VXOJMZPLI6ZU3S7";
    const XCR_SAC: &str = "CDJQXBQO5ICVQUPHZHW7SHOM56K2UNNPPAIXUUSA3XACEI6Q4JQLXNVI";

    #[test]
    fn apply_sac_proofs_learns_only_what_verifies() {
        let xcr = crate::AssetIdentity::Credit {
            code: "XCR".to_string(),
            issuer: XCR_ISSUER.to_string(),
        };
        let mut assets = AssetRegistry::from_existing(vec![]);
        assert_eq!(assets.sac_address_of(&xcr).as_deref(), Some(XCR_SAC));
        let impostor = stellar_strkey::Contract([7; 32]).to_string();
        let rows = [
            SacProofRow {
                contract_id: XCR_SAC.to_string(),
                sep11: format!("XCR:{XCR_ISSUER}"),
            },
            SacProofRow {
                contract_id: impostor.clone(),
                sep11: format!("XCR:{XCR_ISSUER}"),
            },
        ];
        assert_eq!(apply_sac_proofs(&mut assets, &rows), (1, 1));
        assert_eq!(assets.resolve_sac(XCR_SAC), Some(xcr));
        assert_eq!(assets.resolve_sac(&impostor), None);
    }
}
