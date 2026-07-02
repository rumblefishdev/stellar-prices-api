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
                                row.wasm_hash = hex_encode(&h);
                            }
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

    /// Rehydrate registries from persisted rows (merged into `self`, so a load
    /// can seed a run that then keeps discovering). Rows with an unknown venue
    /// string are skipped.
    pub fn load_pool_rows(&mut self, rows: &[PoolRegistryRow]) {
        for row in rows {
            let Some(venue) = Venue::from_source(&row.venue) else {
                continue;
            };
            self.venue.insert(row.contract_id.clone(), venue.clone());
            match venue {
                Venue::Soroswap => {
                    self.soroswap.register(
                        row.contract_id.clone(),
                        row.token0.clone(),
                        row.token1.clone(),
                    );
                }
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

fn hex_encode(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Parse a 64-char hex string into 32 bytes; `None` for empty or malformed input.
fn hex_decode32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
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
        let s = hex_encode(&h);
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

        let rows = reg.to_pool_rows();
        assert_eq!(rows.len(), 3);
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
        assert_eq!(loaded.pool_count(), reg.pool_count());
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
