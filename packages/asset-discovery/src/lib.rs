//! Asset Discovery worker (task 0054) — keeps `prices.assets` populated.
//!
//! Two responsibilities, in invocation order:
//! 1. **Symbols** — resolve `symbol()` for Soroban contracts that have no
//!    `prices.asset_symbol` row yet (task 0210). See [`symbols`].
//! 2. **Seed** — ensure the well-known major assets exist (Tranche-1 bar:
//!    `prices.assets` carries the top assets without waiting for hours of
//!    organic discovery). See [`seed_identities`] + [`ensure_seed`].
//!
//! **There is no ledger scan.** The original design had a third stage that
//! re-read recent ledgers from S3 to register traded assets and to maintain
//! `prices.pool_registry` (task 0069). It was never switched on in production,
//! and task 0256 removed it: the live ledger processor registers every new
//! asset as it ingests (a delta per run via `write_new_assets`), and persists
//! each AMM pool as it learns it from a factory event (task 0291). A second,
//! hourly reader of the same ledgers could only find what live already had.
//!
//! The seed reuses `prices_ingest_core`'s [`AssetRegistry`] + [`OhlcvWriter`] so
//! the rows are byte-identical to the live ledger processor's (same surrogate
//! ids, same column mapping). The supply fetch (`prices.asset_supply`) is a
//! *different* worker (task 0039); this crate only writes the identity columns
//! of `prices.assets` — never `home_domain`, whose enrichment carries the
//! task-0067 whole-row-clobber hazard.

pub mod symbols;

use prices_ingest_core::{AssetIdentity, AssetRegistry, IngestError, OhlcvWriter};
use serde::Deserialize;

/// The Tranche-1 seed list, embedded at build time. Edited as data, not code.
pub const SEED_JSON: &str = include_str!("../seed/major_assets.json");

/// Errors from the discovery worker.
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("seed parse: {0}")]
    Seed(#[from] serde_json::Error),
    #[error(transparent)]
    Ingest(#[from] IngestError),
}

#[derive(Debug, Deserialize)]
struct SeedFile {
    assets: Vec<SeedAsset>,
}

/// One seed entry. `kind` tags the variant: `native` | `credit` | `contract`.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum SeedAsset {
    Native,
    Credit { code: String, issuer: String },
    Contract { address: String },
}

impl From<SeedAsset> for AssetIdentity {
    fn from(s: SeedAsset) -> Self {
        match s {
            SeedAsset::Native => AssetIdentity::Native,
            SeedAsset::Credit { code, issuer } => AssetIdentity::Credit { code, issuer },
            SeedAsset::Contract { address } => AssetIdentity::Contract(address),
        }
    }
}

/// Parse the embedded seed file into asset identities.
pub fn seed_identities() -> Result<Vec<AssetIdentity>, DiscoveryError> {
    let file: SeedFile = serde_json::from_str(SEED_JSON)?;
    Ok(file.assets.into_iter().map(AssetIdentity::from).collect())
}

/// Ensure the given identities exist in `prices.assets` (idempotent).
///
/// Loads the existing registry first so surrogate ids are reused — a re-run, or
/// a run after the live ledger processor has already interned an asset, neither
/// reassigns ids nor duplicates rows. Returns the total asset count in the
/// registry, whether or not this run had anything to write.
///
/// Writes **only what this run interned**, via [`OhlcvWriter::write_new_assets`].
/// It previously re-emitted the whole registry through `write_assets`, and
/// because this worker is scheduled hourly rather than one-shot, that piled a
/// fresh ~209k-row part into `prices.assets` every hour. `ReplacingMergeTree`
/// collapses duplicates only **on merge**, so between merges a reader without
/// `FINAL` sees 1×–4× the registry — which is what drove
/// `prices-production-oracle` into `Runtime.OutOfMemory` at its 256 MB ceiling
/// (task 0256). Task 0132 removed the same amplification from the live ledger
/// processor.
pub async fn ensure_seed(
    writer: &OhlcvWriter,
    identities: &[AssetIdentity],
) -> Result<usize, DiscoveryError> {
    let existing = writer.load_assets().await?;
    let mut registry = AssetRegistry::from_existing(existing);
    // Everything already durable in `prices.assets` sits below this id.
    let durable = registry.watermark();
    for identity in identities {
        registry.get_or_assign(identity);
    }
    // Steady state: the seed is already present, nothing lands at or above the
    // watermark, and this writes NOTHING — no INSERT, no new part.
    writer.write_new_assets(&registry, durable).await?;
    Ok(registry.assets().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_parses_and_is_non_empty() {
        let ids = seed_identities().expect("seed JSON must parse");
        assert!(!ids.is_empty(), "seed must contain at least one asset");
        // XLM native must be present.
        assert!(
            ids.iter().any(|i| matches!(i, AssetIdentity::Native)),
            "seed must include the native XLM asset"
        );
        // USDC must use the canonical issuer (no fabricated addresses).
        assert!(ids.iter().any(|i| matches!(
            i,
            AssetIdentity::Credit { code, issuer }
                if code == "USDC" && issuer == prices_clickhouse::USDC_ISSUER
        )));
    }

    #[test]
    fn seed_has_no_duplicate_identities() {
        let ids = seed_identities().unwrap();
        let mut seen = std::collections::HashSet::new();
        for id in &ids {
            assert!(seen.insert(id.clone()), "duplicate seed identity: {id:?}");
        }
    }

    /// Every credit-asset issuer in the seed must be a well-formed Stellar
    /// ed25519 public key. This fails the build on any malformed / mistyped
    /// address (checksum guard) — the safety net for hand-curated seed data.
    #[test]
    fn seed_issuers_are_valid_strkeys() {
        let ids = seed_identities().unwrap();
        let credits = ids
            .iter()
            .filter(|i| matches!(i, AssetIdentity::Credit { .. }))
            .count();
        assert!(credits >= 19, "expected >=19 credit assets, got {credits}");
        for id in &ids {
            if let AssetIdentity::Credit { code, issuer } = id {
                stellar_strkey::ed25519::PublicKey::from_string(issuer).unwrap_or_else(|e| {
                    panic!("seed asset {code} has an invalid issuer strkey {issuer}: {e}")
                });
            }
        }
    }

    /// The seed must clear the Tranche-1 "≥20 major assets" bar.
    #[test]
    fn seed_meets_tranche1_bar() {
        let ids = seed_identities().unwrap();
        assert!(
            ids.len() >= 20,
            "Tranche-1 requires >=20 seeded major assets, got {}",
            ids.len()
        );
    }
}
