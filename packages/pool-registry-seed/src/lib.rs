//! Seed `prices.pool_registry` from the Soroswap `/pools` API (task 0079).
//!
//! The live ledger-processor (0078) can only price AMM swaps for pools it finds
//! in `prices.pool_registry`. Rather than replay ~12.8M ledgers since Soroban
//! activation to discover them (the 0053 backfill), this fetches the *current*
//! pool set straight from the Soroswap API — one call per AMM venue — and maps
//! it into the registry via the shared `OhlcvWriter::write_pool_registry`
//! (task 0069). One-off, idempotent (ReplacingMergeTree on `contract_id`).
//!
//! The API's `GET /pools?network=…&protocol=…` (bearer-auth) returns, per pool:
//! `protocol, address, tokenA, tokenB, poolType` (+ reserves/fees we ignore).
//! Normalisation applied here: `protocol "aqua" → venue "aquarius"`, `sdex` and
//! any non-AMM protocol dropped, `poolType "xyk" → pool_type 0` (unknown types
//! logged + skipped rather than guessed — e.g. a future Phoenix stable pool).

use prices_ingest_core::{PoolRegistryRow, Registries};
use serde::Deserialize;
use tracing::warn;

/// The AMM venues we seed. `sdex` is indexed by the API too but is an order book,
/// not an AMM pool, so it is never queried.
pub const AMM_PROTOCOLS: [&str; 3] = ["soroswap", "phoenix", "aqua"];

/// One pool object from `GET /pools`. Only the fields we map are declared; the
/// API also returns reserves, fees, stake/LP addresses, etc. (ignored).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiPool {
    pub protocol: String,
    pub address: String,
    #[serde(default)]
    pub token_a: String,
    #[serde(default)]
    pub token_b: String,
    #[serde(default)]
    pub pool_type: String,
}

/// Errors from fetching or seeding.
#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Soroswap API returned {status} for protocol '{protocol}'")]
    Api { status: u16, protocol: String },
    #[error(transparent)]
    Ingest(#[from] prices_ingest_core::IngestError),
    #[error("mtls: {0}")]
    Mtls(String),
    #[error("config: {0}")]
    Config(String),
}

/// Map a Soroswap API `protocol` string to our canonical `pool_registry.venue`.
/// Returns `None` for `sdex` and anything we don't classify as an AMM venue —
/// those are dropped from the seed.
pub fn venue_for(protocol: &str) -> Option<&'static str> {
    match protocol {
        "soroswap" => Some("soroswap"),
        "phoenix" => Some("phoenix"),
        "aqua" => Some("aquarius"), // API venue string differs from our canonical
        _ => None,
    }
}

/// Map the API `poolType` to our numeric `pool_type`. `None` for anything other
/// than `xyk` so an unrecognised type (e.g. a future Phoenix stable pool the
/// extractor can't decode) is skipped rather than mis-seeded.
pub fn pool_type_code(pool_type: &str) -> Option<u32> {
    match pool_type {
        "xyk" => Some(0),
        _ => None,
    }
}

/// Outcome of mapping API pools to registry rows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MapStats {
    pub kept: usize,
    pub dropped_venue: usize,
    pub dropped_pool_type: usize,
}

/// Map API pools to durable `pool_registry` rows, applying the venue/pool_type
/// normalisation and dropping rows we can't classify. Pure — no I/O.
///
/// `token0`/`token1` are carried for every kept row; `load_pool_rows` keeps them
/// for Soroswap (the pair extractor needs them) and ignores them for Aquarius
/// (which stores venue only — its extractor reads tokens from the swap event).
pub fn to_registry_rows(pools: &[ApiPool]) -> (Vec<PoolRegistryRow>, MapStats) {
    let mut rows = Vec::new();
    let mut stats = MapStats::default();
    for p in pools {
        let Some(venue) = venue_for(&p.protocol) else {
            stats.dropped_venue += 1;
            continue;
        };
        let Some(pool_type) = pool_type_code(&p.pool_type) else {
            warn!(
                contract = %p.address,
                venue,
                pool_type = %p.pool_type,
                "unknown poolType — skipping pool (not seeding a type the extractor can't decode)"
            );
            stats.dropped_pool_type += 1;
            continue;
        };
        rows.push(PoolRegistryRow {
            contract_id: p.address.clone(),
            venue: venue.to_string(),
            token0: p.token_a.clone(),
            token1: p.token_b.clone(),
            pool_type,
            wasm_hash: String::new(),
        });
        stats.kept += 1;
    }
    (rows, stats)
}

/// Build a `Registries` from mapped rows (reuses the same `load_pool_rows` path
/// the backfill and live processor use, so the artifact is byte-identical).
pub fn build_registry(rows: &[PoolRegistryRow]) -> Registries {
    let mut reg = Registries::new();
    reg.load_pool_rows(rows);
    reg
}

/// Fetch one venue's pools from `GET {base_url}/pools?network=…&protocol=…` with
/// the bearer API key. `assetList` is intentionally omitted so the full,
/// unfiltered pool set is returned.
pub async fn fetch_pools(
    http: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    network: &str,
    protocol: &str,
) -> Result<Vec<ApiPool>, SeedError> {
    let resp = http
        .get(format!("{base_url}/pools"))
        .query(&[("network", network), ("protocol", protocol)])
        .bearer_auth(api_key)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(SeedError::Api {
            status: status.as_u16(),
            protocol: protocol.to_string(),
        });
    }
    Ok(resp.json::<Vec<ApiPool>>().await?)
}

/// Fetch every AMM venue and map into one deduped registry-row set. Returns the
/// rows plus aggregate map stats. Network I/O; venues fetched sequentially (a
/// handful of small calls).
pub async fn fetch_all_venues(
    http: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    network: &str,
) -> Result<(Vec<PoolRegistryRow>, MapStats), SeedError> {
    let mut all_rows = Vec::new();
    let mut agg = MapStats::default();
    for protocol in AMM_PROTOCOLS {
        let pools = fetch_pools(http, base_url, api_key, network, protocol).await?;
        let (rows, stats) = to_registry_rows(&pools);
        tracing::info!(
            protocol,
            fetched = pools.len(),
            kept = stats.kept,
            dropped_pool_type = stats.dropped_pool_type,
            "fetched venue"
        );
        all_rows.extend(rows);
        agg.kept += stats.kept;
        agg.dropped_venue += stats.dropped_venue;
        agg.dropped_pool_type += stats.dropped_pool_type;
    }
    Ok((all_rows, agg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(protocol: &str, address: &str, pool_type: &str) -> ApiPool {
        ApiPool {
            protocol: protocol.into(),
            address: address.into(),
            token_a: "CTOKENA".into(),
            token_b: "CTOKENB".into(),
            pool_type: pool_type.into(),
        }
    }

    #[test]
    fn venue_mapping_normalises_aqua_and_drops_non_amm() {
        assert_eq!(venue_for("soroswap"), Some("soroswap"));
        assert_eq!(venue_for("phoenix"), Some("phoenix"));
        assert_eq!(venue_for("aqua"), Some("aquarius")); // the critical remap
        assert_eq!(venue_for("sdex"), None); // order book, not a pool
        assert_eq!(venue_for("uniswap"), None);
    }

    #[test]
    fn pool_type_only_maps_xyk() {
        assert_eq!(pool_type_code("xyk"), Some(0));
        assert_eq!(pool_type_code("stable"), None);
        assert_eq!(pool_type_code(""), None);
    }

    #[test]
    fn mapping_keeps_amm_drops_sdex_and_unknown_type() {
        let pools = vec![
            pool("soroswap", "CSORO", "xyk"),
            pool("phoenix", "CPHO", "xyk"),
            pool("aqua", "CAQUA", "xyk"),
            pool("sdex", "CSDEX", "xyk"), // dropped: not an AMM venue
            pool("phoenix", "CSTABLE", "stable"), // dropped: unknown pool type
        ];
        let (rows, stats) = to_registry_rows(&pools);
        assert_eq!(stats.kept, 3);
        assert_eq!(stats.dropped_venue, 1);
        assert_eq!(stats.dropped_pool_type, 1);

        // aqua row carries the canonical venue.
        let aqua = rows.iter().find(|r| r.contract_id == "CAQUA").unwrap();
        assert_eq!(aqua.venue, "aquarius");
        assert!(rows.iter().all(|r| r.contract_id != "CSDEX"));
        assert!(rows.iter().all(|r| r.contract_id != "CSTABLE"));
    }

    #[test]
    fn built_registry_round_trips_all_venues() {
        let pools = vec![
            pool("soroswap", "CSORO", "xyk"),
            pool("phoenix", "CPHO", "xyk"),
            pool("aqua", "CAQUA", "xyk"),
        ];
        let (rows, _) = to_registry_rows(&pools);
        let reg = build_registry(&rows);
        // Soroswap + Phoenix count toward pool_count; all three round-trip to rows.
        let out = reg.to_pool_rows();
        assert_eq!(out.len(), 3);
        let soro = out.iter().find(|r| r.contract_id == "CSORO").unwrap();
        assert_eq!(
            (soro.token0.as_str(), soro.token1.as_str()),
            ("CTOKENA", "CTOKENB")
        );
        assert_eq!(
            reg.venue.get("CAQUA").map(|v| v.as_source()),
            Some("aquarius")
        );
    }

    #[test]
    fn deserializes_real_api_shape() {
        // A verbatim-shaped object from the live API (extra fields must be ignored).
        let json = r#"[{"protocol":"soroswap","address":"CA2G","tokenA":"CB3Y","tokenB":"CDNG",
            "reserveA":"15461025","reserveB":"9117735894","ledger":63273789,"poolType":"xyk","totalFeeBps":30}]"#;
        let pools: Vec<ApiPool> = serde_json::from_str(json).unwrap();
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].address, "CA2G");
        assert_eq!(pools[0].token_a, "CB3Y");
        assert_eq!(pools[0].pool_type, "xyk");
    }
}
