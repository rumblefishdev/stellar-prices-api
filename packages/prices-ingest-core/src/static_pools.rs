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
/// Its sister `CB3A6LUP…` (same wasm) is deliberately absent: it has emitted
/// no event, ever.
pub const STATIC_POOLS: &[(&str, Venue)] = &[(
    "CAS3FL6TLZKDGGSISDBWGGPXT3NRR4DYTZD7YOD3HMYO6LTJUVGRVEAM",
    Venue::Comet,
)];

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

    #[test]
    fn the_list_holds_exactly_the_comet_pool() {
        assert_eq!(STATIC_POOLS, &[(COMET, Venue::Comet)]);
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
        assert_eq!(reg.venue.len(), 1);
        assert_eq!(reg.venue.get(COMET), Some(&Venue::Comet));
        assert_eq!(reg.pool_count(), 0, "no pair or phoenix entry");

        reg.merge_static_pools();
        assert_eq!(reg.venue.len(), 1, "idempotent");
    }

    #[test]
    fn a_merged_pool_flattens_to_one_venue_only_row() {
        let mut reg = Registries::new();
        reg.merge_static_pools();
        assert_eq!(
            reg.to_pool_rows(),
            vec![PoolRegistryRow {
                contract_id: COMET.to_string(),
                venue: "comet".to_string(),
                token0: String::new(),
                token1: String::new(),
                pool_type: 0,
                wasm_hash: String::new(),
            }]
        );
    }
}
