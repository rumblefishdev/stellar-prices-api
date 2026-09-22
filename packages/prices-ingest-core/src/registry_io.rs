//! (De)serialisation of the discovered AMM pool [`Registries`] to/from the
//! durable `prices.pool_registry` artifact (task 0053, decision #4).
//!
//! The backfill grows `Registries` from in-window factory events; persisting it
//! lets a partial re-backfill or the live processor LOAD the classification
//! instead of re-deriving from Soroban activation (inverting task 0069:
//! registry-as-output, not required-input).
//!
//! `venue` is the master superset — every pool is registered there — so a row
//! per venue entry, enriched with the Soroswap pair tokens / Phoenix pool
//! details, round-trips the whole registry.

use std::collections::HashMap;

use extractors_core::Venue;
use serde::{Deserialize, Serialize};

use crate::soroban::Registries;

/// One persisted `prices.pool_registry` row. `updated_at` is omitted — the table
/// defaults it server-side (`now()`), same as the other RMT writers here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, clickhouse::Row)]
pub struct PoolRegistryRow {
    pub contract_id: String,
    pub venue: String,
    pub token0: String,
    pub token1: String,
    pub pool_type: u32,
    pub wasm_hash: String,
}

impl Registries {
    /// Flatten the registries into durable rows, one per discovered pool, sorted
    /// by `contract_id` for a stable artifact across runs. Soroswap rows carry
    /// the pair tokens; Phoenix rows carry `pool_type` + `wasm_hash` (hex).
    pub fn to_pool_rows(&self) -> Vec<PoolRegistryRow> {
        let mut rows: Vec<PoolRegistryRow> = self
            .venue
            .iter()
            .map(|(contract_id, venue)| {
                let mut row = PoolRegistryRow {
                    contract_id: contract_id.clone(),
                    venue: venue.as_source().to_string(),
                    token0: String::new(),
                    token1: String::new(),
                    pool_type: 0,
                    wasm_hash: String::new(),
                };
                match venue {
                    Venue::Soroswap => {
                        if let Some(p) = self.soroswap.lookup(contract_id) {
                            row.token0 = p.token0.clone();
                            row.token1 = p.token1.clone();
                        }
                    }
                    Venue::Phoenix => {
                        if let Some(p) = self.phoenix.lookup(contract_id) {
                            row.pool_type = p.pool_type;
                            if let Some(h) = p.wasm_hash {
                                row.wasm_hash = hex::encode(h);
                            }
                        }
                    }
                    Venue::Sushiswap => {
                        if let Some(p) = self.sushiswap.lookup(contract_id) {
                            row.token0 = p.token0.clone();
                            row.token1 = p.token1.clone();
                        }
                    }
                    Venue::Aquarius => {}
                }
                row
            })
            .collect();
        rows.sort_by(|a, b| a.contract_id.cmp(&b.contract_id));
        rows
    }

    /// The rows of [`to_pool_rows`](Self::to_pool_rows) that `persisted` does not
    /// already hold verbatim — a pool learned since the snapshot, or one whose
    /// row changed. `persisted` is keyed by `contract_id`.
    ///
    /// The live processor's pool write (task 0291): its registry is warm across
    /// invocations and grows from factory events, and before this it was never
    /// written back, so a cold start forgot every pool learned since the last
    /// backfill. Writing only this delta keeps the steady state at zero INSERTs,
    /// the same rule task 0132 set for assets.
    ///
    /// The snapshot must be built from this registry's OWN `to_pool_rows`, not
    /// from the raw table rows: [`load_pool_rows`](Self::load_pool_rows)
    /// normalises some rows (a malformed Phoenix `wasm_hash` loads as none), and
    /// a raw snapshot would report those as changed on every run.
    pub fn pool_rows_unpersisted(
        &self,
        persisted: &HashMap<String, PoolRegistryRow>,
    ) -> Vec<PoolRegistryRow> {
        self.to_pool_rows()
            .into_iter()
            .filter(|row| persisted.get(&row.contract_id) != Some(row))
            .collect()
    }

    /// Rehydrate registries from persisted rows (merged into `self`, so a load
    /// can seed a run that then keeps discovering). Rows with an unknown venue
    /// string are skipped.
    ///
    /// A pair-backed row (Soroswap, SushiSwap) whose `token0`/`token1` are
    /// blank registers its VENUE but not its pair. Registering a blank pair
    /// would be worse than not loading the row at all: `contains()` would
    /// answer true, `classify_amm_groups` would clear `pair_unresolved`, and
    /// the pool would be priced against asset `""` instead of being recorded in
    /// `unresolved_pools`. Leaving the pair out sends it down the
    /// venue-known-but-unpriced branch, which is exactly what a missing pair
    /// is. This mirrors the learn side, where a `pool_created` without a token
    /// pair learns nothing (task 0290 review).
    pub fn load_pool_rows(&mut self, rows: &[PoolRegistryRow]) {
        for row in rows {
            let Some(venue) = Venue::from_source(&row.venue) else {
                continue;
            };
            self.venue.insert(row.contract_id.clone(), venue.clone());
            let pair_complete = !row.token0.is_empty() && !row.token1.is_empty();
            match venue {
                Venue::Soroswap if pair_complete => {
                    self.soroswap.register(
                        row.contract_id.clone(),
                        row.token0.clone(),
                        row.token1.clone(),
                    );
                }
                Venue::Sushiswap if pair_complete => {
                    self.sushiswap.register(
                        row.contract_id.clone(),
                        row.token0.clone(),
                        row.token1.clone(),
                    );
                }
                Venue::Soroswap | Venue::Sushiswap => {}
                Venue::Phoenix => match hex_decode32(&row.wasm_hash) {
                    Some(hash) => self.phoenix.register_with_wasm(
                        row.contract_id.clone(),
                        row.pool_type,
                        hash,
                    ),
                    None => self
                        .phoenix
                        .register(row.contract_id.clone(), row.pool_type),
                },
                Venue::Aquarius => {}
            }
        }
    }
}

/// Parse a hex string into exactly 32 bytes; `None` for empty, wrong-length, or
/// malformed input. Thin wrapper over `hex::decode` that pins the 32-byte width
/// the Phoenix `wasm_hash` requires.
fn hex_decode32(s: &str) -> Option<[u8; 32]> {
    hex::decode(s).ok()?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let h = [
            0x16, 0x7a, 0xb4, 0x14, 0xff, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
            0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16,
            0x17, 0x18, 0x50, 0x6c,
        ];
        let s = hex::encode(h);
        assert_eq!(s.len(), 64);
        assert_eq!(hex_decode32(&s), Some(h));
    }

    #[test]
    fn hex_decode_rejects_bad_input() {
        assert_eq!(hex_decode32(""), None);
        assert_eq!(hex_decode32("zz"), None);
        assert_eq!(hex_decode32(&"g".repeat(64)), None);
    }

    #[test]
    fn registries_round_trip_through_rows() {
        let mut reg = Registries::new();
        // One of each venue.
        reg.venue.insert("CSOROSWAP".into(), Venue::Soroswap);
        reg.soroswap
            .register("CSOROSWAP".into(), "CTOKEN0".into(), "CTOKEN1".into());
        reg.venue.insert("CPHOENIX".into(), Venue::Phoenix);
        reg.phoenix
            .register_with_wasm("CPHOENIX".into(), 0, [0xab; 32]);
        reg.venue.insert("CAQUA".into(), Venue::Aquarius);
        // Task 0290: pair-backed like Soroswap, but its OWN registry.
        reg.venue.insert("CSUSHI".into(), Venue::Sushiswap);
        reg.sushiswap
            .register("CSUSHI".into(), "CSUSHI0".into(), "CSUSHI1".into());

        let rows = reg.to_pool_rows();
        assert_eq!(rows.len(), 4);
        // Sorted, stable order.
        assert_eq!(rows[0].contract_id, "CAQUA");

        let mut loaded = Registries::new();
        loaded.load_pool_rows(&rows);

        assert_eq!(loaded.venue.get("CSOROSWAP"), Some(&Venue::Soroswap));
        assert_eq!(loaded.venue.get("CPHOENIX"), Some(&Venue::Phoenix));
        assert_eq!(loaded.venue.get("CAQUA"), Some(&Venue::Aquarius));
        let sw = loaded.soroswap.lookup("CSOROSWAP").expect("soroswap pair");
        assert_eq!(
            (sw.token0.as_str(), sw.token1.as_str()),
            ("CTOKEN0", "CTOKEN1")
        );
        let ph = loaded.phoenix.lookup("CPHOENIX").expect("phoenix pool");
        assert_eq!(ph.wasm_hash, Some([0xab; 32]));

        // The sushiswap pool round-trips into its own registry, and the two
        // pair-backed venues stay disjoint (task 0290).
        assert_eq!(loaded.venue.get("CSUSHI"), Some(&Venue::Sushiswap));
        let su = loaded.sushiswap.lookup("CSUSHI").expect("sushiswap pair");
        assert_eq!(
            (su.token0.as_str(), su.token1.as_str()),
            ("CSUSHI0", "CSUSHI1")
        );
        assert!(!loaded.soroswap.contains("CSUSHI"));
        assert!(!loaded.sushiswap.contains("CSOROSWAP"));

        assert_eq!(loaded.pool_count(), reg.pool_count());
    }

    #[test]
    fn a_persisted_pair_row_without_tokens_loads_its_venue_but_no_pair() {
        // A hand-written or imported row that names a pair-backed venue but
        // carries blank tokens. Registering it would make `contains()` true and
        // price the pool against asset "" — it must stay pair-unresolved so
        // `classify_amm_groups` records it in `unresolved_pools` (task 0290).
        let rows: Vec<PoolRegistryRow> = ["soroswap", "sushiswap"]
            .iter()
            .map(|venue| PoolRegistryRow {
                contract_id: format!("CBLANK_{venue}"),
                venue: (*venue).to_string(),
                token0: String::new(),
                token1: String::new(),
                pool_type: 0,
                wasm_hash: String::new(),
            })
            .collect();

        let mut loaded = Registries::new();
        loaded.load_pool_rows(&rows);

        assert_eq!(
            loaded.venue.get("CBLANK_soroswap"),
            Some(&Venue::Soroswap),
            "the venue is known — only the pair is missing"
        );
        assert_eq!(
            loaded.venue.get("CBLANK_sushiswap"),
            Some(&Venue::Sushiswap)
        );
        assert!(!loaded.soroswap.contains("CBLANK_soroswap"));
        assert!(!loaded.sushiswap.contains("CBLANK_sushiswap"));
        assert_eq!(loaded.pool_count(), 0);
    }

    #[test]
    fn a_persisted_pair_row_missing_one_token_loads_no_pair_either() {
        let rows = vec![PoolRegistryRow {
            contract_id: "CHALFPAIR".into(),
            venue: "sushiswap".into(),
            token0: "CTOKEN0".into(),
            token1: String::new(),
            pool_type: 0,
            wasm_hash: String::new(),
        }];

        let mut loaded = Registries::new();
        loaded.load_pool_rows(&rows);

        assert_eq!(loaded.venue.get("CHALFPAIR"), Some(&Venue::Sushiswap));
        assert!(!loaded.sushiswap.contains("CHALFPAIR"));
    }

    fn snapshot(reg: &Registries) -> HashMap<String, PoolRegistryRow> {
        reg.to_pool_rows()
            .into_iter()
            .map(|r| (r.contract_id.clone(), r))
            .collect()
    }

    #[test]
    fn unpersisted_is_empty_right_after_the_snapshot() {
        let mut reg = Registries::new();
        reg.venue.insert("CAQUA".into(), Venue::Aquarius);
        reg.venue.insert("CSOROSWAP".into(), Venue::Soroswap);
        reg.soroswap
            .register("CSOROSWAP".into(), "CTOKEN0".into(), "CTOKEN1".into());
        let persisted = snapshot(&reg);
        assert!(reg.pool_rows_unpersisted(&persisted).is_empty());
    }

    #[test]
    fn unpersisted_yields_only_the_newly_learned_pool() {
        let mut reg = Registries::new();
        reg.venue.insert("CAQUA".into(), Venue::Aquarius);
        let persisted = snapshot(&reg);

        reg.venue.insert("CNEWPAIR".into(), Venue::Soroswap);
        reg.soroswap
            .register("CNEWPAIR".into(), "CTOKEN0".into(), "CTOKEN1".into());

        let rows = reg.pool_rows_unpersisted(&persisted);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].contract_id, "CNEWPAIR");
        assert_eq!(
            (rows[0].token0.as_str(), rows[0].token1.as_str()),
            ("CTOKEN0", "CTOKEN1")
        );
    }

    #[test]
    fn unpersisted_yields_a_pool_whose_row_changed() {
        let mut reg = Registries::new();
        reg.venue.insert("CPHOENIX".into(), Venue::Phoenix);
        reg.phoenix.register("CPHOENIX".into(), 0);
        let persisted = snapshot(&reg);

        reg.phoenix
            .register_with_wasm("CPHOENIX".into(), 0, [0xab; 32]);

        let rows = reg.pool_rows_unpersisted(&persisted);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].wasm_hash, hex::encode([0xab; 32]));
    }

    #[test]
    fn a_normalised_row_is_not_reported_as_changed() {
        // A malformed wasm_hash loads as "no hash", so the registry's own row
        // differs from the table's. The snapshot is built from the registry, so
        // this must not re-write the pool on every run.
        let mut reg = Registries::new();
        reg.load_pool_rows(&[PoolRegistryRow {
            contract_id: "CPHOENIX".into(),
            venue: "phoenix".into(),
            token0: String::new(),
            token1: String::new(),
            pool_type: 0,
            wasm_hash: "not-hex".into(),
        }]);
        let persisted = snapshot(&reg);
        assert!(reg.pool_rows_unpersisted(&persisted).is_empty());
    }

    #[test]
    fn load_skips_unknown_venue() {
        let mut reg = Registries::new();
        reg.load_pool_rows(&[PoolRegistryRow {
            contract_id: "CBAD".into(),
            venue: "uniswap".into(),
            token0: String::new(),
            token1: String::new(),
            pool_type: 0,
            wasm_hash: String::new(),
        }]);
        assert!(reg.venue.is_empty());
    }
}
