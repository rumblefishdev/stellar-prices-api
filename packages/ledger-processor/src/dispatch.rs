use aquarius_extractor::AquariusPoolExtractor;
use extractors_core::{
    ExtractError, SorobanEventRow, SwapExtractor, TradeRow, Venue, VenueRegistry,
};
use phoenix_extractor::{
    PHOENIX_STABLE_EVENT_COUNT, PHOENIX_XYK_MIN_EVENT_COUNT, POOL_TYPE_XYK, PhoenixPoolRegistry,
    PhoenixXykExtractor,
};
use soroswap_extractor::{SoroswapPairExtractor, SoroswapPoolRegistry};

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("extract error: {0}")]
    Extract(#[from] ExtractError),
    #[error("pool {contract_id} not found in Phoenix registry")]
    UnknownPool { contract_id: String },
    #[error(
        "unknown Phoenix pool_type {pool_type} with {event_count} events for pool {contract_id}"
    )]
    UnknownPhoenixShape {
        contract_id: String,
        pool_type: u32,
        event_count: usize,
    },
    #[error("venue {venue:?} extractor not yet implemented for pool {contract_id}")]
    VenueNotImplemented { venue: Venue, contract_id: String },
}

/// Route a batch of contiguous Soroban event rows for a single
/// (transaction_id, contract_id) group to the correct extractor.
///
/// Phoenix routing uses (pool_type, event_count) — never WASM hash.
pub fn dispatch_phoenix(
    rows: &[SorobanEventRow],
    registry: &PhoenixPoolRegistry,
) -> Result<Vec<TradeRow>, DispatchError> {
    if rows.is_empty() {
        return Ok(vec![]);
    }

    let contract_id = &rows[0].contract_id;
    let pool = registry
        .lookup(contract_id)
        .ok_or_else(|| DispatchError::UnknownPool {
            contract_id: contract_id.clone(),
        })?;

    match (pool.pool_type, rows.len()) {
        // Gate on the MINIMUM priceable group, not the fully-populated one.
        // Phoenix omits optional fields, so swap groups are variable length;
        // `>= PHOENIX_XYK_EVENT_COUNT` (8) discarded every 7-event swap — 5,175
        // of them (~2.1%) over the Soroban era, in live as well as backfill.
        // The extractor validates the four required fields and rejects
        // non-swap groups (liquidity events) on topic0 / missing fields.
        (POOL_TYPE_XYK, n) if n >= PHOENIX_XYK_MIN_EVENT_COUNT => {
            let result = PhoenixXykExtractor.extract(rows)?;
            Ok(result.trades)
        }
        (pool_type, n) if pool_type != POOL_TYPE_XYK && n >= PHOENIX_STABLE_EVENT_COUNT => {
            // Stable extractor not yet implemented — no stable pools exist on mainnet.
            Err(DispatchError::UnknownPhoenixShape {
                contract_id: contract_id.clone(),
                pool_type,
                event_count: n,
            })
        }
        (pool_type, event_count) => Err(DispatchError::UnknownPhoenixShape {
            contract_id: contract_id.clone(),
            pool_type,
            event_count,
        }),
    }
}

/// Top-level dispatcher: routes events by venue, then by pool shape for Phoenix.
///
/// Soroswap requires the pool→tokens registry to resolve token identities; an
/// unresolved pool (created before the indexed window) yields no trades rather
/// than an error. Aquarius and Phoenix carry tokens inline.
pub fn dispatch(
    rows: &[SorobanEventRow],
    venue_registry: &VenueRegistry,
    phoenix_registry: &PhoenixPoolRegistry,
    soroswap_registry: &SoroswapPoolRegistry,
) -> Result<Vec<TradeRow>, DispatchError> {
    if rows.is_empty() {
        return Ok(vec![]);
    }

    let contract_id = &rows[0].contract_id;
    let venue = venue_registry.get(contract_id);

    match venue {
        Some(Venue::Phoenix) => dispatch_phoenix(rows, phoenix_registry),
        Some(Venue::Soroswap) => match soroswap_registry.lookup(contract_id) {
            Some(pair) => Ok(SoroswapPairExtractor::new(pair).extract(rows)?.trades),
            None => Ok(vec![]),
        },
        Some(Venue::Aquarius) => Ok(AquariusPoolExtractor.extract(rows)?.trades),
        None => Ok(vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use phoenix_extractor::test_fixtures::*;

    fn phoenix_registry_both_wasm_variants() -> PhoenixPoolRegistry {
        PhoenixPoolRegistry::from_fixture(&[(XLM_USDC_POOL, 0), (PHO_USDC_POOL, 0)])
    }

    fn venue_registry_phoenix(pools: &[&str]) -> VenueRegistry {
        pools
            .iter()
            .map(|p| (p.to_string(), Venue::Phoenix))
            .collect()
    }

    #[test]
    fn dispatch_routes_xlm_usdc_xyk_pool() {
        let rows = make_phoenix_xyk_events(XLM_USDC_POOL, 5);
        let phoenix_reg = phoenix_registry_both_wasm_variants();
        let venue_reg = venue_registry_phoenix(&[XLM_USDC_POOL, PHO_USDC_POOL]);

        let trades = dispatch(
            &rows,
            &venue_reg,
            &phoenix_reg,
            &SoroswapPoolRegistry::new(),
        )
        .unwrap();
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].contract_id, XLM_USDC_POOL);
        assert_eq!(trades[0].venue, Venue::Phoenix);
    }

    #[test]
    fn dispatch_routes_pho_usdc_alt_wasm_pool_identically() {
        let rows = make_phoenix_xyk_events(PHO_USDC_POOL, 5);
        let phoenix_reg = phoenix_registry_both_wasm_variants();
        let venue_reg = venue_registry_phoenix(&[XLM_USDC_POOL, PHO_USDC_POOL]);

        let trades = dispatch(
            &rows,
            &venue_reg,
            &phoenix_reg,
            &SoroswapPoolRegistry::new(),
        )
        .unwrap();
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].contract_id, PHO_USDC_POOL);
        assert_eq!(trades[0].amount_in, 11659417676);
        assert_eq!(trades[0].amount_out, 1857322909);
    }

    #[test]
    fn dispatch_phoenix_uses_pool_type_not_wasm_hash() {
        let mut reg = PhoenixPoolRegistry::new();
        reg.register_with_wasm(XLM_USDC_POOL.to_string(), 0, common_xyk_wasm_hash());
        reg.register_with_wasm(PHO_USDC_POOL.to_string(), 0, alt_xyk_wasm_hash());

        let pool_a = reg.lookup(XLM_USDC_POOL).unwrap();
        let pool_b = reg.lookup(PHO_USDC_POOL).unwrap();
        assert_ne!(pool_a.wasm_hash, pool_b.wasm_hash);

        for pool in [XLM_USDC_POOL, PHO_USDC_POOL] {
            let rows = make_phoenix_xyk_events(pool, 5);
            let trades = dispatch_phoenix(&rows, &reg).unwrap();
            assert_eq!(trades.len(), 1, "pool {pool} should produce 1 trade");
        }
    }

    #[test]
    fn dispatch_stable_pool_returns_error_unimplemented() {
        let rows = make_phoenix_xyk_events(XLM_USDC_POOL, 0);
        let mut reg = PhoenixPoolRegistry::new();
        reg.register(XLM_USDC_POOL.to_string(), 1);

        let result = dispatch_phoenix(&rows, &reg);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unknown Phoenix pool_type 1"),
            "expected stable-pool error, got: {err}"
        );
    }

    #[test]
    fn dispatch_skips_unknown_venue() {
        let rows = make_phoenix_xyk_events("CUNKNOWN_POOL_NOT_IN_REGISTRY", 5);
        let phoenix_reg = phoenix_registry_both_wasm_variants();
        let venue_reg = venue_registry_phoenix(&[XLM_USDC_POOL]);

        let trades = dispatch(
            &rows,
            &venue_reg,
            &phoenix_reg,
            &SoroswapPoolRegistry::new(),
        )
        .unwrap();
        assert!(trades.is_empty());
    }

    #[test]
    fn dispatch_empty_rows_returns_empty() {
        let phoenix_reg = phoenix_registry_both_wasm_variants();
        let venue_reg = venue_registry_phoenix(&[XLM_USDC_POOL]);

        let trades = dispatch(&[], &venue_reg, &phoenix_reg, &SoroswapPoolRegistry::new()).unwrap();
        assert!(trades.is_empty());
    }
}
