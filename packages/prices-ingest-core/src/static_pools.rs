//! Pools that have no factory event, registered by a committed list (task 0300).
//!
//! Every other pool enters `prices.pool_registry` because its factory announced
//! it: `learn_factory` on the live path, `events-backfill --discover-pools`
//! historically. A pool deployed by hand has no such event, so neither can ever
//! see it — its swaps would be counted as an unregistered pool forever. This
//! list is the registration for those pools.

use extractors_core::Venue;

use crate::soroban::Registries;

/// Factory-less pools, each routed to its venue. An entry is added only by a
/// reviewed commit.
///
/// - `CAS3FL6T…VEAM` — the Comet weighted pool BLND/USDC (Blend's backstop).
///   Deployed at ledger 51,499,545 (2024-05-02) by `GDJEHTBE…`, the BLND
///   issuer, with wasm `8abc28913035c07411ed5d134e6bfeab4723d97ddd4d1a22a0605d35c94d1a36`.
///   It has no factory and emits no init / new-pool event (task 0300).
///
/// - `CASYAKK3…UMIHE` — Blend v2.1's Comet BLND/USDC pool, listed in
///   `blend-capital/docs-v2` `mainnet-deployments.md`. Deployed 2026-09-30
///   with wasm `d735c3395f59510172cf5cf838823a1389b97cfec6bf24e580e9bd77d2b3e687`,
///   CometDEX release v1.1.0, which rejects self-swaps (task 0334).
///
/// - `CDQMIKIH…C7Y3` (BLNT/USDC) and `CA5HX3D4…3FLC` (BLNT/XLM) — Comet v1.1
///   pools deployed 2026-10-25 by `GDEA2WMO…` for the BLNT token launch, same
///   wasm `d735c339…`. First swaps at ledger 65,137,110 (task 0337).
///
/// - `CCNDNFA7…7QFZ` — a small Comet v1.1 BLND/USDC pool deployed 2026-09-23
///   by `GCRKRXU2…`, same wasm. First swap at ledger 65,099,227 (task 0337).
///
/// Its sister `CB3A6LUP…` (same wasm as `CAS3FL6T…`) is deliberately absent:
/// it has emitted no event, ever.
pub const STATIC_POOLS: &[(&str, Venue)] = &[
    (
        "CAS3FL6TLZKDGGSISDBWGGPXT3NRR4DYTZD7YOD3HMYO6LTJUVGRVEAM",
        Venue::Comet,
    ),
    (
        "CASYAKK3SHDN6S2E6IUOBVRXT3KKPVQCVQO4GNBDTVE6XZQQMW4UMIHE",
        Venue::Comet,
    ),
    (
        "CDQMIKIHKA5UY3FQJ2UDTLNX3GOPD3GPNZWSPTOBVOHEUMQJQXOPC7Y3",
        Venue::Comet,
    ),
    (
        "CA5HX3D47PQZWC2E7VWXCKH4534P2NOJEOIXAWU47MI4QV2FQUYY3FLC",
        Venue::Comet,
    ),
    (
        "CCNDNFA7LEVUJRYWTSWVAA2ESZAKFO6MYOPI6E5AZT3V2UBP2MNV7QFZ",
        Venue::Comet,
    ),
];

impl Registries {
    /// Route every [`STATIC_POOLS`] entry to its venue. The committed list
    /// wins for its own ids. Idempotent.
    ///
    /// Called explicitly at the four registry-build seams, each at the point
    /// its own semantics need:
    /// - the live `Reconciler` (`prices-ledger-processor`), BEFORE its
    ///   `persisted_pools` snapshot, so live routes the pools but never writes
    ///   their rows;
    /// - `events-backfill`'s reprice, AFTER the `EmptyPoolRegistry` guard;
    /// - `events-backfill --discover-pools`, AFTER its snapshot, so a row the
    ///   table lacks is written once (missing-only);
    /// - `sdex-backfill`, after its load.
    ///
    /// It is deliberately NOT inside `OhlcvWriter::load_pool_registry`: there
    /// it would disarm the `EmptyPoolRegistry` guard, make discover's snapshot
    /// count the rows as already written, make asset-discovery rewrite them
    /// with its whole registry, and break the ClickHouse ITs that count the
    /// rows a load returns.
    pub fn merge_static_pools(&mut self) {
        for (contract_id, venue) in STATIC_POOLS {
            self.venue.insert((*contract_id).to_string(), venue.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::registry_io::PoolRegistryRow;

    const COMET: &str = "CAS3FL6TLZKDGGSISDBWGGPXT3NRR4DYTZD7YOD3HMYO6LTJUVGRVEAM";
    const COMET_V1_1: &str = "CASYAKK3SHDN6S2E6IUOBVRXT3KKPVQCVQO4GNBDTVE6XZQQMW4UMIHE";
    const BLNT_USDC: &str = "CDQMIKIHKA5UY3FQJ2UDTLNX3GOPD3GPNZWSPTOBVOHEUMQJQXOPC7Y3";
    const BLNT_XLM: &str = "CA5HX3D47PQZWC2E7VWXCKH4534P2NOJEOIXAWU47MI4QV2FQUYY3FLC";
    const BLND_USDC_SMALL: &str = "CCNDNFA7LEVUJRYWTSWVAA2ESZAKFO6MYOPI6E5AZT3V2UBP2MNV7QFZ";
    const ALL: [&str; 5] = [COMET, COMET_V1_1, BLNT_USDC, BLNT_XLM, BLND_USDC_SMALL];

    #[test]
    fn the_list_holds_exactly_the_five_comet_pools() {
        let listed: Vec<(&str, Venue)> = STATIC_POOLS
            .iter()
            .map(|(id, v)| (*id, v.clone()))
            .collect();
        let expected: Vec<(&str, Venue)> = ALL.iter().map(|id| (*id, Venue::Comet)).collect();
        assert_eq!(listed, expected);
    }

    #[test]
    fn every_entry_is_a_distinct_contract_strkey() {
        let mut seen = HashSet::new();
        for (id, _) in STATIC_POOLS {
            stellar_strkey::Contract::from_string(id)
                .unwrap_or_else(|e| panic!("{id} is not a contract strkey: {e:?}"));
            assert!(seen.insert(*id), "{id} listed twice");
        }
    }

    #[test]
    fn merging_routes_the_pools_and_touches_nothing_else() {
        let mut reg = Registries::new();
        reg.merge_static_pools();
        assert_eq!(reg.venue.len(), ALL.len());
        for id in ALL {
            assert_eq!(reg.venue.get(id), Some(&Venue::Comet), "{id}");
        }
        assert_eq!(reg.pool_count(), 0, "no pair or phoenix entry");

        reg.merge_static_pools();
        assert_eq!(reg.venue.len(), ALL.len(), "idempotent");
    }

    #[test]
    fn each_merged_pool_flattens_to_one_venue_only_row() {
        let mut reg = Registries::new();
        reg.merge_static_pools();
        let mut rows = reg.to_pool_rows();
        rows.sort_by(|a, b| a.contract_id.cmp(&b.contract_id));
        let row = |id: &str| PoolRegistryRow {
            contract_id: id.to_string(),
            venue: "comet".to_string(),
            token0: String::new(),
            token1: String::new(),
            pool_type: 0,
            wasm_hash: String::new(),
        };
        let mut expected: Vec<PoolRegistryRow> = ALL.iter().map(|id| row(id)).collect();
        expected.sort_by(|a, b| a.contract_id.cmp(&b.contract_id));
        assert_eq!(rows, expected);
    }
}
