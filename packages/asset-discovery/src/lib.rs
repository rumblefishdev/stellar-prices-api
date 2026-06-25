//! Asset Discovery worker (task 0054) — keeps `prices.assets` populated.
//!
//! Two responsibilities, in invocation order:
//! 1. **Seed** — ensure the well-known major assets exist (Tranche-1 bar:
//!    `prices.assets` carries the top assets without waiting for hours of
//!    organic discovery). See [`seed_identities`] + [`ensure_seed`].
//! 2. **Discover** — (Increment 2, not yet implemented) scan recent ledgers
//!    for new classic issuances / SEP-41 deployments and add them, advancing
//!    the `prices.discovery_state` high-water-mark.
//!
//! Both reuse `prices_ingest_core`'s [`AssetRegistry`] + [`OhlcvWriter`] so the
//! rows are byte-identical to the live ledger processor's (same surrogate ids,
//! same column mapping). The supply fetch (`prices.asset_supply`) is a
//! *different* worker (task 0039); this crate only writes the identity columns
//! of `prices.assets` — never `home_domain`, whose enrichment carries the
//! task-0067 whole-row-clobber hazard.

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
/// reassigns ids nor duplicates rows (ReplacingMergeTree collapses on the sort
/// key). Returns the total asset count in the registry after the write.
pub async fn ensure_seed(
    writer: &OhlcvWriter,
    identities: &[AssetIdentity],
) -> Result<usize, DiscoveryError> {
    let existing = writer.load_assets().await?;
    let mut registry = AssetRegistry::from_existing(existing);
    for identity in identities {
        registry.get_or_assign(identity);
    }
    writer.write_assets(&registry).await?;
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
}
