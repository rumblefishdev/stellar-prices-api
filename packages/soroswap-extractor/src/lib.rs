//! Soroswap AMM swap extractor.
//!
//! Unlike Phoenix/Aquarius, a Soroswap pool `swap` event does NOT carry the
//! token addresses — only amounts. Token identity comes from a pool→(token0,
//! token1) registry built from the Soroswap factory `new_pair` events. The
//! extractor is therefore constructed with the resolved pair.
//!
//! Two pool-level data shapes are handled (from real mainnet samples + spec):
//!
//! - uniswap-v2 constant product: `Map{ amount_0_in, amount_1_in, amount_0_out, amount_1_out : i128 }`
//! - CLMM concentrated liquidity: `Map{ amount0, amount1 : i128 (signed; + into pool, − out) }`
//!
//! Router / aggregator wrapper `swap` events (simple {amount_in, amount_out})
//! are dropped upstream (VenueRegistry maps only pool contract_ids) to avoid
//! double-counting; they carry no token/direction info on their own.

use std::collections::HashMap;

use extractors_core::{
    ExtractError, ExtractResult, SorobanEventRow, SwapExtractor, TaggedValue, TradeRow, Venue,
};

/// A Soroswap pair's two tokens, in canonical (token0, token1) order as
/// reported by the factory `new_pair` event.
#[derive(Debug, Clone)]
pub struct SoroswapPair {
    pub token0: String,
    pub token1: String,
}

/// pool_address → (token0, token1). Populated from Soroswap factory `new_pair`
/// events (`[String("SoroswapFactory"), Symbol("new_pair")]`,
/// data = NewPairEvent{ token_0, token_1, pair, … }).
#[derive(Debug, Default)]
pub struct SoroswapPoolRegistry {
    pools: HashMap<String, SoroswapPair>,
}

impl SoroswapPoolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, pair: String, token0: String, token1: String) {
        self.pools.insert(pair, SoroswapPair { token0, token1 });
    }

    pub fn lookup(&self, pair: &str) -> Option<&SoroswapPair> {
        self.pools.get(pair)
    }

    pub fn contains(&self, pair: &str) -> bool {
        self.pools.contains_key(pair)
    }

    pub fn pool_count(&self) -> usize {
        self.pools.len()
    }

    pub fn from_fixture(entries: &[(&str, &str, &str)]) -> Self {
        let mut reg = Self::new();
        for &(pair, t0, t1) in entries {
            reg.register(pair.to_string(), t0.to_string(), t1.to_string());
        }
        reg
    }
}

fn map_get<'a>(map: &'a [(TaggedValue, TaggedValue)], key: &str) -> Option<&'a TaggedValue> {
    map.iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .map(|(_, v)| v)
}

/// The pool action symbol for a Soroswap event.
///
/// Real `SoroswapPair` contracts emit `topics = [String("SoroswapPair"),
/// Symbol(<action>)]` — the action (`swap`/`sync`/`deposit`/…) lives in
/// **topic[1]**, with the constant `"SoroswapPair"` in topic[0]. The action is
/// therefore read from topic[1] whenever topic[0] is `"SoroswapPair"`, and from
/// topic[0] otherwise (a bare `Symbol("swap")`, as older fixtures use). Returns
/// `None` when neither position carries a symbol/string.
fn swap_action(row: &SorobanEventRow) -> Option<&str> {
    let t0 = row.topics.first().and_then(|t| t.as_str())?;
    if t0 == "SoroswapPair" {
        row.topics.get(1).and_then(|t| t.as_str())
    } else {
        Some(t0)
    }
}

/// Extracts Soroswap swaps for a single pool, using its registered token pair.
pub struct SoroswapPairExtractor<'a> {
    pub pair: &'a SoroswapPair,
}

impl<'a> SoroswapPairExtractor<'a> {
    pub fn new(pair: &'a SoroswapPair) -> Self {
        Self { pair }
    }

    fn decode_swap(&self, row: &SorobanEventRow) -> Result<TradeRow, ExtractError> {
        if swap_action(row) != Some("swap") {
            return Err(ExtractError::UnexpectedTopicShape(row.event_index));
        }

        let map = match &row.data {
            TaggedValue::Map(m) => m.as_slice(),
            _ => return Err(ExtractError::UnexpectedTopicShape(row.event_index)),
        };

        let (token_in, token_out, amount_in, amount_out) =
            if map_get(map, "amount_0_in").is_some() || map_get(map, "amount_1_in").is_some() {
                // uniswap-v2 constant product
                let a0_in = map_get(map, "amount_0_in")
                    .and_then(|v| v.as_i128())
                    .unwrap_or(0);
                let a1_in = map_get(map, "amount_1_in")
                    .and_then(|v| v.as_i128())
                    .unwrap_or(0);
                let a0_out = map_get(map, "amount_0_out")
                    .and_then(|v| v.as_i128())
                    .unwrap_or(0);
                let a1_out = map_get(map, "amount_1_out")
                    .and_then(|v| v.as_i128())
                    .unwrap_or(0);
                if a0_in >= a1_in {
                    (
                        self.pair.token0.clone(),
                        self.pair.token1.clone(),
                        a0_in,
                        a1_out,
                    )
                } else {
                    (
                        self.pair.token1.clone(),
                        self.pair.token0.clone(),
                        a1_in,
                        a0_out,
                    )
                }
            } else {
                // CLMM: signed amount0 / amount1 (+ into pool, − out of pool)
                let a0 = map_get(map, "amount0")
                    .and_then(|v| v.as_i128())
                    .ok_or_else(|| ExtractError::MissingField("amount0".into()))?;
                let a1 = map_get(map, "amount1")
                    .and_then(|v| v.as_i128())
                    .ok_or_else(|| ExtractError::MissingField("amount1".into()))?;
                if a0 >= 0 {
                    (
                        self.pair.token0.clone(),
                        self.pair.token1.clone(),
                        a0,
                        a1.abs(),
                    )
                } else {
                    (
                        self.pair.token1.clone(),
                        self.pair.token0.clone(),
                        a1,
                        a0.abs(),
                    )
                }
            };

        let trader = map_get(map, "sender")
            .or_else(|| map_get(map, "recipient"))
            .or_else(|| map_get(map, "to"))
            .and_then(|v| v.as_address())
            .map(String::from);

        Ok(TradeRow {
            venue: Venue::Soroswap,
            contract_id: row.contract_id.clone(),
            transaction_id: row.transaction_id.clone(),
            ledger_sequence: row.ledger_sequence,
            first_event_index: row.event_index,
            token_in,
            token_out,
            amount_in,
            amount_out,
            fee: None,
            trader,
        })
    }
}

impl SwapExtractor for SoroswapPairExtractor<'_> {
    fn extract(&self, rows: &[SorobanEventRow]) -> Result<ExtractResult, ExtractError> {
        if rows.is_empty() {
            return Err(ExtractError::InsufficientRows {
                expected: 1,
                actual: 0,
            });
        }
        let mut trades = Vec::new();
        for row in rows {
            if swap_action(row) == Some("swap") {
                trades.push(self.decode_swap(row)?);
            }
        }
        Ok(ExtractResult {
            rows_consumed: rows.len(),
            trades,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POOL: &str = "CCR2CH4GQVCZHG7CHFVMNANCK45CU5DVKXZIIITDZQAU3CEJZ7RQH2MQ";
    const T0: &str = "CTOKEN0AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const T1: &str = "CTOKEN1AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn clmm_swap() -> SorobanEventRow {
        // Real sample: amount0 2871373757 (in), amount1 -439878710 (out).
        SorobanEventRow {
            contract_id: POOL.to_string(),
            transaction_id: "TX".to_string(),
            ledger_sequence: 62078346,
            event_index: 4,
            topics: vec![TaggedValue::Symbol("swap".to_string())],
            data: TaggedValue::Map(vec![
                (
                    TaggedValue::Symbol("amount0".into()),
                    TaggedValue::I128(2871373757),
                ),
                (
                    TaggedValue::Symbol("amount1".into()),
                    TaggedValue::I128(-439878710),
                ),
                (
                    TaggedValue::Symbol("sender".into()),
                    TaggedValue::Address(
                        "GDCRZPZYBZ24RHRO3WBPJGFDL7NDFKUQBS3ZDB6YGBJB3TGKMFYBQ3LD".into(),
                    ),
                ),
            ]),
        }
    }

    #[test]
    fn decodes_clmm_swap_with_sign_direction() {
        let pair = SoroswapPair {
            token0: T0.into(),
            token1: T1.into(),
        };
        let result = SoroswapPairExtractor::new(&pair)
            .extract(&[clmm_swap()])
            .unwrap();
        assert_eq!(result.trades.len(), 1);
        let t = &result.trades[0];
        assert_eq!(t.venue, Venue::Soroswap);
        assert_eq!(t.token_in, T0);
        assert_eq!(t.token_out, T1);
        assert_eq!(t.amount_in, 2871373757);
        assert_eq!(t.amount_out, 439878710);
        assert!(t.trader.is_some());
    }

    #[test]
    fn decodes_clmm_swap_reverse_direction() {
        let mut ev = clmm_swap();
        ev.data = TaggedValue::Map(vec![
            (
                TaggedValue::Symbol("amount0".into()),
                TaggedValue::I128(-1000),
            ),
            (
                TaggedValue::Symbol("amount1".into()),
                TaggedValue::I128(2500),
            ),
        ]);
        let pair = SoroswapPair {
            token0: T0.into(),
            token1: T1.into(),
        };
        let t = &SoroswapPairExtractor::new(&pair)
            .extract(&[ev])
            .unwrap()
            .trades[0];
        assert_eq!(t.token_in, T1);
        assert_eq!(t.token_out, T0);
        assert_eq!(t.amount_in, 2500);
        assert_eq!(t.amount_out, 1000);
    }

    #[test]
    fn decodes_uniswap_v2_swap() {
        let pair = SoroswapPair {
            token0: T0.into(),
            token1: T1.into(),
        };
        let ev = SorobanEventRow {
            contract_id: POOL.to_string(),
            transaction_id: "TX".to_string(),
            ledger_sequence: 62078346,
            event_index: 4,
            topics: vec![TaggedValue::Symbol("swap".to_string())],
            data: TaggedValue::Map(vec![
                (
                    TaggedValue::Symbol("amount_0_in".into()),
                    TaggedValue::I128(1000),
                ),
                (
                    TaggedValue::Symbol("amount_1_in".into()),
                    TaggedValue::I128(0),
                ),
                (
                    TaggedValue::Symbol("amount_0_out".into()),
                    TaggedValue::I128(0),
                ),
                (
                    TaggedValue::Symbol("amount_1_out".into()),
                    TaggedValue::I128(2490),
                ),
            ]),
        };
        let t = &SoroswapPairExtractor::new(&pair)
            .extract(&[ev])
            .unwrap()
            .trades[0];
        assert_eq!(t.token_in, T0);
        assert_eq!(t.token_out, T1);
        assert_eq!(t.amount_in, 1000);
        assert_eq!(t.amount_out, 2490);
    }

    #[test]
    fn decodes_real_soroswap_pair_swap_shape() {
        // Real prod shape (BE soroban_events): topics = [String("SoroswapPair"),
        // Symbol("swap")] — the action is in topic[1], not topic[0], and the
        // trader is in `to`. This is the shape ALL of our 221 seeded pools emit
        // (~824k swaps); the extractor previously read the action from topic[0]
        // and silently produced zero trades (task 0096 root cause).
        let pair = SoroswapPair {
            token0: T0.into(),
            token1: T1.into(),
        };
        let ev = SorobanEventRow {
            contract_id: POOL.to_string(),
            transaction_id: "TX".to_string(),
            ledger_sequence: 50_704_650,
            event_index: 5,
            topics: vec![
                TaggedValue::String("SoroswapPair".to_string()),
                TaggedValue::Symbol("swap".to_string()),
            ],
            data: TaggedValue::Map(vec![
                (
                    TaggedValue::Symbol("amount_0_in".into()),
                    TaggedValue::I128(1_000_000),
                ),
                (
                    TaggedValue::Symbol("amount_0_out".into()),
                    TaggedValue::I128(0),
                ),
                (
                    TaggedValue::Symbol("amount_1_in".into()),
                    TaggedValue::I128(0),
                ),
                (
                    TaggedValue::Symbol("amount_1_out".into()),
                    TaggedValue::I128(914_145),
                ),
                (
                    TaggedValue::Symbol("to".into()),
                    TaggedValue::Address(
                        "GCEKBLVMSTPZDTQR263QKZYJNOXESD6LZYGN4SH64Z7FHW4TYR5Y235K".into(),
                    ),
                ),
            ]),
        };
        let result = SoroswapPairExtractor::new(&pair).extract(&[ev]).unwrap();
        assert_eq!(
            result.trades.len(),
            1,
            "SoroswapPair-shaped swap must decode"
        );
        let t = &result.trades[0];
        assert_eq!(t.token_in, T0);
        assert_eq!(t.token_out, T1);
        assert_eq!(t.amount_in, 1_000_000);
        assert_eq!(t.amount_out, 914_145);
        assert_eq!(
            t.trader.as_deref(),
            Some("GCEKBLVMSTPZDTQR263QKZYJNOXESD6LZYGN4SH64Z7FHW4TYR5Y235K"),
            "trader resolved from the `to` field"
        );
    }

    #[test]
    fn ignores_non_swap_soroswap_pair_actions() {
        // `sync` (826k events — MORE than swaps), `deposit`, `withdraw`, `skim`
        // share the [String("SoroswapPair"), Symbol(action)] envelope but are not
        // trades. `extract` must skip them, or they'd flood price_ohlcv_1m with
        // garbage candles.
        let pair = SoroswapPair {
            token0: T0.into(),
            token1: T1.into(),
        };
        let sync = SorobanEventRow {
            contract_id: POOL.to_string(),
            transaction_id: "TX".to_string(),
            ledger_sequence: 50_688_706,
            event_index: 7,
            topics: vec![
                TaggedValue::String("SoroswapPair".to_string()),
                TaggedValue::Symbol("sync".to_string()),
            ],
            data: TaggedValue::Map(vec![
                (
                    TaggedValue::Symbol("new_reserve_0".into()),
                    TaggedValue::I128(1_250_000_000_000),
                ),
                (
                    TaggedValue::Symbol("new_reserve_1".into()),
                    TaggedValue::I128(2_000_000_000_000),
                ),
            ]),
        };
        let result = SoroswapPairExtractor::new(&pair).extract(&[sync]).unwrap();
        assert!(
            result.trades.is_empty(),
            "a non-swap SoroswapPair action must not produce a trade"
        );
    }

    #[test]
    fn registry_roundtrip() {
        let reg = SoroswapPoolRegistry::from_fixture(&[(POOL, T0, T1)]);
        assert_eq!(reg.pool_count(), 1);
        let p = reg.lookup(POOL).unwrap();
        assert_eq!(p.token0, T0);
        assert_eq!(p.token1, T1);
    }
}
