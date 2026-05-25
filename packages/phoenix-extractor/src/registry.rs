use std::collections::HashMap;

/// A Phoenix pool as registered from the factory.
///
/// Keyed by `contract_id` (C-strkey), NOT by WASM hash.
/// Two distinct XYK WASM builds exist in production (167ab414…506c and
/// 13b158655e…f2ca) — both report pool_type == 0 and emit identical
/// 8-event swap groupings. Keying off WASM hash would silently drop
/// the PHO/USDC pool.
#[derive(Debug, Clone)]
pub struct PhoenixPool {
    pub contract_id: String,
    pub pool_type: u32,
    pub wasm_hash: Option<[u8; 32]>,
}

#[derive(Debug, Default)]
pub struct PhoenixPoolRegistry {
    pools: HashMap<String, PhoenixPool>,
}

impl PhoenixPoolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, contract_id: String, pool_type: u32) {
        self.pools.insert(
            contract_id.clone(),
            PhoenixPool {
                contract_id,
                pool_type,
                wasm_hash: None,
            },
        );
    }

    pub fn register_with_wasm(
        &mut self,
        contract_id: String,
        pool_type: u32,
        wasm_hash: [u8; 32],
    ) {
        self.pools.insert(
            contract_id.clone(),
            PhoenixPool {
                contract_id,
                pool_type,
                wasm_hash: Some(wasm_hash),
            },
        );
    }

    pub fn lookup(&self, contract_id: &str) -> Option<&PhoenixPool> {
        self.pools.get(contract_id)
    }

    pub fn contains(&self, contract_id: &str) -> bool {
        self.pools.contains_key(contract_id)
    }

    pub fn pool_count(&self) -> usize {
        self.pools.len()
    }

    /// Build a registry from a fixture list of (contract_id, pool_type) pairs.
    pub fn from_fixture(entries: &[(&str, u32)]) -> Self {
        let mut reg = Self::new();
        for &(contract_id, pool_type) in entries {
            reg.register(contract_id.to_string(), pool_type);
        }
        reg
    }

    /// Build a registry from a fixture list that includes WASM hashes.
    pub fn from_fixture_with_wasm(entries: &[(&str, u32, [u8; 32])]) -> Self {
        let mut reg = Self::new();
        for (contract_id, pool_type, wasm_hash) in entries {
            reg.register_with_wasm(contract_id.to_string(), *pool_type, *wasm_hash);
        }
        reg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XLM_USDC_POOL: &str = "CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX";
    const PHO_USDC_POOL: &str = "CD5XNKK3B6BEF2N7ULNHHGAMOKZ7P6456BFNIHRF4WNTEDKBRWAE7IAA";

    fn common_xyk_wasm_hash() -> [u8; 32] {
        let mut h = [0u8; 32];
        h[0] = 0x16;
        h[1] = 0x7a;
        h[2] = 0xb4;
        h[3] = 0x14;
        h
    }

    fn alt_xyk_wasm_hash() -> [u8; 32] {
        let mut h = [0u8; 32];
        h[0] = 0x13;
        h[1] = 0xb1;
        h[2] = 0x58;
        h[3] = 0x65;
        h
    }

    #[test]
    fn registry_from_fixture_returns_correct_pool_type() {
        let reg = PhoenixPoolRegistry::from_fixture(&[
            (XLM_USDC_POOL, 0),
            (PHO_USDC_POOL, 0),
        ]);

        let xlm = reg.lookup(XLM_USDC_POOL).expect("XLM/USDC pool");
        assert_eq!(xlm.pool_type, 0);
        assert_eq!(xlm.wasm_hash, None);

        let pho = reg.lookup(PHO_USDC_POOL).expect("PHO/USDC pool");
        assert_eq!(pho.pool_type, 0);
    }

    #[test]
    fn registry_with_different_wasm_hashes_both_resolve_as_xyk() {
        let reg = PhoenixPoolRegistry::from_fixture_with_wasm(&[
            (XLM_USDC_POOL, 0, common_xyk_wasm_hash()),
            (PHO_USDC_POOL, 0, alt_xyk_wasm_hash()),
        ]);

        let xlm = reg.lookup(XLM_USDC_POOL).unwrap();
        let pho = reg.lookup(PHO_USDC_POOL).unwrap();

        // Different WASM hashes…
        assert_ne!(xlm.wasm_hash, pho.wasm_hash);
        // …but both are XYK (pool_type 0)
        assert_eq!(xlm.pool_type, 0);
        assert_eq!(pho.pool_type, 0);
    }

    #[test]
    fn lookup_unknown_pool_returns_none() {
        let reg = PhoenixPoolRegistry::from_fixture(&[(XLM_USDC_POOL, 0)]);
        assert!(reg.lookup("CNOTAPOOL").is_none());
    }

    #[test]
    fn pool_count_reflects_registered_entries() {
        let reg = PhoenixPoolRegistry::from_fixture(&[
            (XLM_USDC_POOL, 0),
            (PHO_USDC_POOL, 0),
        ]);
        assert_eq!(reg.pool_count(), 2);
    }
}
