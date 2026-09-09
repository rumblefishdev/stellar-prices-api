//! Asset Discovery worker (task 0054) — keeps `prices.assets` populated.
//!
//! Two responsibilities, in invocation order:
//! 1. **Seed** — ensure the well-known major assets exist (Tranche-1 bar:
//!    `prices.assets` carries the top assets without waiting for hours of
//!    organic discovery). See [`seed_identities`] + [`ensure_seed`].
//! 2. **Discover** — scan a window of recent ledgers from S3, extract every
//!    asset that appears in SDEX trades + Soroban AMM swaps (reusing the tested
//!    `extract_trades` / `process_ledger` pipeline), register the new ones, and
//!    advance the `prices.discovery_state` high-water-mark. Trade-activity is
//!    the discovery surface — an asset that never trades has no price row to
//!    populate, so it needs no registry entry. See [`discover_window`].
//!
//! **Pool-registry maintenance (task 0069).** The same window scan runs
//! `process_ledger`, which grows an AMM [`Registries`] from in-window factory
//! events (Soroswap / Aquarius / Phoenix). This worker loads the persisted
//! `prices.pool_registry`, keeps growing it across the window, and persists it
//! back — so newly-created pools become resolvable *durably*, surviving the live
//! ledger-processor's cold-start reload (task 0078). This is the periodic
//! maintenance the live processor deliberately does not do on its hot path (it
//! only reads the registry). Registry-as-output, sharing the exact
//! `to_pool_rows` shape the SDEX backfill persists (task 0053 decision #4).
//!
//! Both reuse `prices_ingest_core`'s [`AssetRegistry`] + [`OhlcvWriter`] so the
//! rows are byte-identical to the live ledger processor's (same surrogate ids,
//! same column mapping). The supply fetch (`prices.asset_supply`) is a
//! *different* worker (task 0039); this crate only writes the identity columns
//! of `prices.assets` — never `home_domain`, whose enrichment carries the
//! task-0067 whole-row-clobber hazard.

pub mod symbols;

use prices_ingest_core::{
    AssetIdentity, AssetRegistry, IngestError, OhlcvWriter, Registries, decode_object,
    extract_trades, process_ledger,
};
use prices_ledger_processor::galexie_key::ledger_s3_key;
use prices_ledger_processor::object_fetcher::{FetchError, ObjectFetcher};
use serde::{Deserialize, Serialize};
use stellar_xdr::LedgerCloseMeta;

/// The Tranche-1 seed list, embedded at build time. Edited as data, not code.
pub const SEED_JSON: &str = include_str!("../seed/major_assets.json");

/// `prices.discovery_state.worker` key for this worker's high-water-mark.
pub const WORKER: &str = "asset-discovery";

/// Errors from the discovery worker.
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("seed parse: {0}")]
    Seed(#[from] serde_json::Error),
    #[error(transparent)]
    Ingest(#[from] IngestError),
    #[error("ledger fetch: {0}")]
    Fetch(#[from] FetchError),
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

// ---------------------------------------------------------------------------
// Organic discovery (Increment 2)
// ---------------------------------------------------------------------------

/// Outcome of a [`discover_window`] run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryStats {
    /// First ledger sequence the run attempted.
    pub from_ledger: u64,
    /// Last ledger sequence successfully scanned (== `from_ledger - 1` if none).
    pub to_ledger: u64,
    /// Number of contiguous ledgers scanned before the first gap / bound.
    pub ledgers_scanned: u64,
    /// Total assets in the registry after the run (existing + newly discovered).
    pub assets_total: usize,
    /// Rows persisted to `prices.pool_registry` after the run — one per
    /// registered pool across all venues (Soroswap / Aquarius / Phoenix), i.e.
    /// `to_pool_rows().len()`, *not* `pool_count()` (which omits Aquarius). See
    /// the pool-registry maintenance note (0069).
    pub pools_total: usize,
}

/// Register every asset appearing in `metas` (SDEX trades + Soroban AMM tokens)
/// into `registry`, and grow `pools` from any in-window AMM factory events.
/// Reuses the tested ingest pipeline. Pure — no I/O.
///
/// `pools` is caller-owned so it accumulates across a whole window scan (and is
/// pre-seeded from the persisted `prices.pool_registry`): every factory event
/// `process_ledger` sees registers its pool, which the caller then persists
/// (task 0069). Passing it in — rather than a throwaway per call — is what turns
/// the previously-discarded pool discovery into durable registry maintenance.
pub fn register_ledger_assets(
    registry: &mut AssetRegistry,
    pools: &mut Registries,
    metas: &[LedgerCloseMeta],
) {
    for lcm in metas {
        for trade in extract_trades(lcm) {
            // RawTrade assets are already AssetIdentity (via AssetIdentity::from_xdr).
            registry.get_or_assign(&trade.asset_sold);
            registry.get_or_assign(&trade.asset_bought);
        }
        // process_ledger get_or_assigns the AMM token identities into `registry`
        // and grows `pools` from factory events (Soroswap/Aquarius/Phoenix).
        let _ = process_ledger(lcm, pools, registry);
    }
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct DiscoveryStateRow {
    worker: String,
    last_ledger: u64,
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct CursorRow {
    last_ledger: u64,
}

/// Read this worker's high-water-mark from `prices.discovery_state` (`None` if
/// the worker has never run).
pub async fn load_cursor(writer: &OhlcvWriter) -> Result<Option<u64>, DiscoveryError> {
    let rows = writer
        .client()
        .query("SELECT last_ledger FROM prices.discovery_state FINAL WHERE worker = ?")
        .bind(WORKER)
        .fetch_all::<CursorRow>()
        .await
        .map_err(IngestError::from)?;
    Ok(rows.into_iter().next().map(|r| r.last_ledger))
}

/// Advance this worker's high-water-mark (ReplacingMergeTree → latest wins).
pub async fn save_cursor(writer: &OhlcvWriter, last_ledger: u64) -> Result<(), DiscoveryError> {
    let mut insert = writer
        .client()
        .insert("prices.discovery_state")
        .map_err(IngestError::from)?;
    insert
        .write(&DiscoveryStateRow {
            worker: WORKER.to_string(),
            last_ledger,
        })
        .await
        .map_err(IngestError::from)?;
    insert.end().await.map_err(IngestError::from)?;
    Ok(())
}

/// Scan up to `max_ledgers` contiguous ledgers starting at `start_ledger`,
/// register every asset found, persist them, and advance the cursor.
///
/// Stops early at the first missing ledger (a gap → caught up to the tip), like
/// the live processor's reconcile. Generic over the fetcher so tests drive it
/// with a `LocalDiskFetcher` over bundled fixtures and the Lambda drives it with
/// the S3 fetcher. Writes nothing (and leaves the cursor untouched) if no ledger
/// was scanned.
pub async fn discover_window<F: ObjectFetcher>(
    writer: &OhlcvWriter,
    fetcher: &F,
    start_ledger: u64,
    max_ledgers: u64,
) -> Result<DiscoveryStats, DiscoveryError> {
    let existing = writer.load_assets().await?;
    let mut registry = AssetRegistry::from_existing(existing);
    // Pre-seed the pool registry from the persisted table so the window scan
    // grows it (rather than re-deriving from activation) and a re-scan never
    // drops a pool it didn't happen to re-observe (task 0069). Snapshot the
    // persisted rows so we only re-write the table when the scan actually changed
    // it (see the write guard below).
    let mut pools = writer.load_pool_registry().await?;
    let loaded_rows = pools.to_pool_rows();

    let mut scanned = 0u64;
    let mut last = start_ledger.saturating_sub(1);
    for ledger in start_ledger..start_ledger.saturating_add(max_ledgers) {
        let key = ledger_s3_key(ledger as i64);
        let Some(bytes) = fetcher.fetch(&key).await? else {
            break; // gap → caught up
        };
        let metas = decode_object(&bytes)?;
        register_ledger_assets(&mut registry, &mut pools, &metas);
        last = ledger;
        scanned += 1;
    }

    // The durable row set after the scan — one row per registered pool across all
    // venues (Soroswap / Aquarius / Phoenix). This, not `pool_count()` (which omits
    // Aquarius), is what's actually persisted, so it is the true `pools_total`.
    let final_rows = pools.to_pool_rows();

    if scanned > 0 {
        writer.write_assets(&registry).await?;
        // Only re-write the registry when the scan changed it. The pre-seeded
        // registry is re-emitted verbatim on every zero-discovery run, and a full
        // RMT re-INSERT each hour would pile up parts and inflate the next FINAL
        // load. Compare the whole row set (not just the count) so a pool whose row
        // was enriched in-window is still persisted. Written before advancing the
        // cursor so a crash can't strand a discovered pool behind an already-
        // advanced high-water-mark; idempotent (RMT on contract_id).
        if final_rows != loaded_rows {
            writer.write_pool_registry(&pools).await?;
        }
        save_cursor(writer, last).await?;
    }

    Ok(DiscoveryStats {
        from_ledger: start_ledger,
        to_ledger: last,
        ledgers_scanned: scanned,
        assets_total: registry.assets().count(),
        pools_total: final_rows.len(),
    })
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

    /// Pool-registry maintenance (task 0069) must not drop a pool it loaded but
    /// did not re-observe this window. `register_ledger_assets` grows the caller's
    /// `pools` in place, so a pre-seeded pool survives a scan of ledgers with no
    /// factory events — the guarantee that a rolling-window re-scan is additive,
    /// never clobbering. Pure, no I/O.
    #[test]
    fn register_ledger_assets_preserves_preseeded_pools() {
        use prices_ingest_core::PoolRegistryRow;

        let mut assets = AssetRegistry::from_existing(Vec::new());
        // Seed a pool the way `discover_window` does — from a persisted row.
        let mut pools = Registries::new();
        pools.load_pool_rows(&[PoolRegistryRow {
            contract_id: "CPREEXISTING".into(),
            venue: "soroswap".into(),
            token0: "CTOKEN0".into(),
            token1: "CTOKEN1".into(),
            pool_type: 0,
            wasm_hash: String::new(),
        }]);
        assert_eq!(pools.pool_count(), 1);

        // No ledgers → no factory events → the seeded pool must remain.
        register_ledger_assets(&mut assets, &mut pools, &[]);

        assert_eq!(pools.pool_count(), 1, "pre-seeded pool must not be dropped");
        assert!(
            pools
                .to_pool_rows()
                .iter()
                .any(|r| r.contract_id == "CPREEXISTING"),
            "seeded pool must still round-trip to a durable row"
        );
    }
}
