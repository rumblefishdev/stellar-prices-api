use extractors_core::{ExtractError, SorobanEventRow, SwapExtractor, TradeRow, Venue, VenueRegistry};
use phoenix_extractor::{
    PhoenixPoolRegistry, PhoenixXykExtractor, PHOENIX_STABLE_EVENT_COUNT,
    PHOENIX_XYK_EVENT_COUNT, POOL_TYPE_XYK,
};

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("extract error: {0}")]
    Extract(#[from] ExtractError),
    #[error("unknown Phoenix pool_type {pool_type} with {event_count} events for pool {contract_id}")]
    UnknownPhoenixShape {
        contract_id: String,
        pool_type: u32,
        event_count: usize,
    },
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
        .ok_or_else(|| DispatchError::UnknownPhoenixShape {
            contract_id: contract_id.clone(),
            pool_type: u32::MAX,
            event_count: rows.len(),
        })?;

    match (pool.pool_type, rows.len()) {
        (POOL_TYPE_XYK, n) if n >= PHOENIX_XYK_EVENT_COUNT => {
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
pub fn dispatch(
    rows: &[SorobanEventRow],
    venue_registry: &VenueRegistry,
    phoenix_registry: &PhoenixPoolRegistry,
) -> Result<Vec<TradeRow>, DispatchError> {
    if rows.is_empty() {
        return Ok(vec![]);
    }

    let contract_id = &rows[0].contract_id;
    let venue = venue_registry.get(contract_id);

    match venue {
        Some(Venue::Phoenix) => dispatch_phoenix(rows, phoenix_registry),
        Some(Venue::Soroswap) => {
            todo!("Soroswap extractor not yet implemented")
        }
        Some(Venue::Aquarius) => {
            todo!("Aquarius extractor not yet implemented")
        }
        None => Ok(vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extractors_core::TaggedValue;

    const XLM_USDC_POOL: &str = "CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX";
    const PHO_USDC_POOL: &str = "CD5XNKK3B6BEF2N7ULNHHGAMOKZ7P6456BFNIHRF4WNTEDKBRWAE7IAA";
    const TRADER: &str = "GDCRZPZYBZ24RHRO3WBPJGFDL7NDFKUQBS3ZDB6YGBJB3TGKMFYBQ3LD";
    const XLM_SAC: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
    const USDC_SAC: &str = "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75";
    const TX_HASH: &str = "559498bdf567340c0780b80f2bfa07bcc58713fc328e659ef72461849a326aa8";

    fn make_xyk_events(pool: &str) -> Vec<SorobanEventRow> {
        let fields: &[(&str, TaggedValue)] = &[
            ("sender", TaggedValue::Address(TRADER.into())),
            ("sell_token", TaggedValue::Address(XLM_SAC.into())),
            ("offer_amount", TaggedValue::I128(11659417676)),
            ("actual received amount", TaggedValue::I128(11659417676)),
            ("buy_token", TaggedValue::Address(USDC_SAC.into())),
            ("return_amount", TaggedValue::I128(1857322909)),
            ("spread_amount", TaggedValue::I128(503808)),
            ("referral_fee_amount", TaggedValue::I128(0)),
        ];

        fields
            .iter()
            .enumerate()
            .map(|(i, (name, data))| SorobanEventRow {
                contract_id: pool.to_string(),
                transaction_id: TX_HASH.to_string(),
                ledger_sequence: 62460522,
                event_index: 5 + i as u32,
                topics: vec![
                    TaggedValue::String("swap".into()),
                    TaggedValue::String((*name).into()),
                ],
                data: data.clone(),
            })
            .collect()
    }

    fn phoenix_registry_both_wasm_variants() -> PhoenixPoolRegistry {
        PhoenixPoolRegistry::from_fixture(&[
            (XLM_USDC_POOL, 0), // common XYK WASM (167ab414…506c)
            (PHO_USDC_POOL, 0), // alt XYK WASM (13b158655e…f2ca)
        ])
    }

    fn venue_registry_phoenix(pools: &[&str]) -> VenueRegistry {
        pools
            .iter()
            .map(|p| (p.to_string(), Venue::Phoenix))
            .collect()
    }

    #[test]
    fn dispatch_routes_xlm_usdc_xyk_pool() {
        let rows = make_xyk_events(XLM_USDC_POOL);
        let phoenix_reg = phoenix_registry_both_wasm_variants();
        let venue_reg = venue_registry_phoenix(&[XLM_USDC_POOL, PHO_USDC_POOL]);

        let trades = dispatch(&rows, &venue_reg, &phoenix_reg).unwrap();
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].contract_id, XLM_USDC_POOL);
        assert_eq!(trades[0].venue, Venue::Phoenix);
    }

    #[test]
    fn dispatch_routes_pho_usdc_alt_wasm_pool_identically() {
        let rows = make_xyk_events(PHO_USDC_POOL);
        let phoenix_reg = phoenix_registry_both_wasm_variants();
        let venue_reg = venue_registry_phoenix(&[XLM_USDC_POOL, PHO_USDC_POOL]);

        let trades = dispatch(&rows, &venue_reg, &phoenix_reg).unwrap();
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].contract_id, PHO_USDC_POOL);
        assert_eq!(trades[0].amount_in, 11659417676);
        assert_eq!(trades[0].amount_out, 1857322909);
    }

    #[test]
    fn dispatch_phoenix_uses_pool_type_not_wasm_hash() {
        let mut reg = PhoenixPoolRegistry::new();

        let mut wasm_a = [0u8; 32];
        wasm_a[0] = 0x16;
        reg.register_with_wasm(XLM_USDC_POOL.to_string(), 0, wasm_a);

        let mut wasm_b = [0u8; 32];
        wasm_b[0] = 0x13;
        reg.register_with_wasm(PHO_USDC_POOL.to_string(), 0, wasm_b);

        // Both pools have different WASM hashes but same pool_type=0.
        // Dispatch must succeed for both — proving the classifier uses
        // pool_type, not WASM hash.
        let pool_a = reg.lookup(XLM_USDC_POOL).unwrap();
        let pool_b = reg.lookup(PHO_USDC_POOL).unwrap();
        assert_ne!(pool_a.wasm_hash, pool_b.wasm_hash);

        for pool in [XLM_USDC_POOL, PHO_USDC_POOL] {
            let rows = make_xyk_events(pool);
            let trades = dispatch_phoenix(&rows, &reg).unwrap();
            assert_eq!(trades.len(), 1, "pool {pool} should produce 1 trade");
        }
    }

    #[test]
    fn dispatch_skips_unknown_venue() {
        let rows = make_xyk_events("CUNKNOWN_POOL_NOT_IN_REGISTRY");
        let phoenix_reg = phoenix_registry_both_wasm_variants();
        let venue_reg = venue_registry_phoenix(&[XLM_USDC_POOL]);

        let trades = dispatch(&rows, &venue_reg, &phoenix_reg).unwrap();
        assert!(trades.is_empty());
    }

    #[test]
    fn dispatch_empty_rows_returns_empty() {
        let phoenix_reg = phoenix_registry_both_wasm_variants();
        let venue_reg = venue_registry_phoenix(&[XLM_USDC_POOL]);

        let trades = dispatch(&[], &venue_reg, &phoenix_reg).unwrap();
        assert!(trades.is_empty());
    }
}
