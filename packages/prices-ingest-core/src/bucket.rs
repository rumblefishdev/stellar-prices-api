use std::collections::HashMap;

use rust_decimal::Decimal;

use crate::tick::TradeTick;

#[derive(Debug, Clone)]
pub struct OhlcvCandle {
    pub minute_start: u32,
    pub asset_id: u32,
    pub quote_asset_id: u32,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume_base: Decimal,
    pub volume_quote: Decimal,
    pub vwap: Decimal,
    pub trade_count: u32,
    pub version: u64,
    open_lex: (u32, u16, u16),
    close_lex: (u32, u16, u16),
}

type BucketKey = (u32, u32, u32); // (minute_start, asset_id, quote_asset_id)

pub struct CandleAccumulator {
    candles: HashMap<BucketKey, OhlcvCandle>,
}

impl Default for CandleAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl CandleAccumulator {
    pub fn new() -> Self {
        Self {
            candles: HashMap::new(),
        }
    }

    pub fn merge(&mut self, tick: &TradeTick) {
        let minute_start = (tick.closed_at as u32 / 60) * 60;
        let key = (minute_start, tick.base_id, tick.quote_id);
        let lex = tick.lex_key();
        let version = tick.ledger_sequence as u64 * 1000 + tick.operation_index as u64;

        let candle = self.candles.entry(key).or_insert_with(|| OhlcvCandle {
            minute_start,
            asset_id: tick.base_id,
            quote_asset_id: tick.quote_id,
            open: tick.price,
            high: tick.price,
            low: tick.price,
            close: tick.price,
            volume_base: Decimal::ZERO,
            volume_quote: Decimal::ZERO,
            vwap: Decimal::ZERO,
            trade_count: 0,
            version,
            open_lex: lex,
            close_lex: lex,
        });

        if lex < candle.open_lex {
            candle.open = tick.price;
            candle.open_lex = lex;
        }
        if lex > candle.close_lex {
            candle.close = tick.price;
            candle.close_lex = lex;
        }

        candle.high = candle.high.max(tick.price);
        candle.low = candle.low.min(tick.price);
        candle.volume_base += tick.volume_base;
        candle.volume_quote += tick.volume_quote;
        candle.trade_count += 1;
        if candle.version < version {
            candle.version = version;
        }
    }

    pub fn flush_older_than(&mut self, current_minute: u32) -> Vec<OhlcvCandle> {
        let mut flushed = Vec::new();
        self.candles.retain(|key, candle| {
            if key.0 < current_minute {
                finalise_vwap(candle);
                flushed.push(candle.clone());
                false
            } else {
                true
            }
        });
        flushed
    }

    pub fn flush_all(&mut self) -> Vec<OhlcvCandle> {
        let mut flushed: Vec<OhlcvCandle> = self
            .candles
            .drain()
            .map(|(_, mut c)| {
                finalise_vwap(&mut c);
                c
            })
            .collect();
        flushed.sort_by_key(|c| (c.minute_start, c.asset_id, c.quote_asset_id));
        flushed
    }
}

fn finalise_vwap(candle: &mut OhlcvCandle) {
    if !candle.volume_base.is_zero() {
        candle.vwap = candle.volume_quote / candle.volume_base;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A minute is `floor(closed_at / 60) * 60`. These three timestamps: the
    // first two share minute M0, the third is in the next minute M1.
    const T_M0_A: i64 = 1_700_000_000; // minute 1_699_999_980
    const T_M0_B: i64 = 1_700_000_030; // same minute
    const T_M1: i64 = 1_700_000_100; // minute 1_700_000_100
    const M0: u32 = 1_699_999_980;
    const M1: u32 = 1_700_000_100;

    #[allow(clippy::too_many_arguments)]
    fn tick(
        ledger: u32,
        op: u16,
        claim: u16,
        base: u32,
        quote: u32,
        price: i64,
        vol_base: i64,
        vol_quote: i64,
        closed_at: i64,
    ) -> TradeTick {
        TradeTick {
            ledger_sequence: ledger,
            closed_at,
            operation_index: op,
            claim_index: claim,
            base_id: base,
            quote_id: quote,
            price: Decimal::from(price),
            volume_base: Decimal::from(vol_base),
            volume_quote: Decimal::from(vol_quote),
        }
    }

    #[test]
    fn single_trade_seeds_flat_ohlc() {
        let mut acc = CandleAccumulator::new();
        acc.merge(&tick(100, 0, 0, 1, 2, 7, 3, 21, T_M0_A));
        let out = acc.flush_all();
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!(c.minute_start, M0);
        assert_eq!((c.asset_id, c.quote_asset_id), (1, 2));
        assert_eq!(
            (c.open, c.high, c.low, c.close),
            (7.into(), 7.into(), 7.into(), 7.into())
        );
        assert_eq!(c.trade_count, 1);
        assert_eq!(c.volume_base, 3.into());
        assert_eq!(c.volume_quote, 21.into());
        assert_eq!(c.vwap, 7.into()); // 21 / 3
        assert_eq!(c.version, 100_000); // ledger*1000 + op
    }

    #[test]
    fn open_and_close_follow_lex_order_not_arrival_order() {
        let mut acc = CandleAccumulator::new();
        // Insert out of ledger/op order; open must be the lowest lex, close the
        // highest — regardless of insertion order.
        acc.merge(&tick(100, 2, 0, 1, 2, 5, 1, 5, T_M0_A)); // mid lex, price 5
        acc.merge(&tick(100, 0, 0, 1, 2, 10, 1, 10, T_M0_A)); // first lex, price 10
        acc.merge(&tick(100, 5, 0, 1, 2, 20, 1, 20, T_M0_B)); // last lex, price 20
        let c = &acc.flush_all()[0];
        assert_eq!(c.open, 10.into(), "open = earliest lex trade");
        assert_eq!(c.close, 20.into(), "close = latest lex trade");
        assert_eq!(c.high, 20.into());
        assert_eq!(c.low, 5.into());
        assert_eq!(c.trade_count, 3);
        assert_eq!(c.volume_base, 3.into());
        assert_eq!(c.volume_quote, 35.into());
        assert_eq!(c.version, 100_005, "version = max(ledger*1000 + op)");
    }

    #[test]
    fn two_pairs_in_one_minute_are_separate_candles() {
        // Scenario 1: XLM/USDC (1,2); Scenario 2: PHO/USDC (3,2) — same minute,
        // must not collide.
        let mut acc = CandleAccumulator::new();
        acc.merge(&tick(100, 0, 0, 1, 2, 10, 2, 20, T_M0_A));
        acc.merge(&tick(100, 1, 0, 3, 2, 4, 5, 20, T_M0_B));
        let out = acc.flush_all();
        assert_eq!(out.len(), 2);
        let xlm = out.iter().find(|c| c.asset_id == 1).unwrap();
        let pho = out.iter().find(|c| c.asset_id == 3).unwrap();
        assert_eq!(xlm.close, 10.into());
        assert_eq!(xlm.vwap, 10.into()); // 20/2
        assert_eq!(pho.close, 4.into());
        assert_eq!(pho.vwap, 4.into()); // 20/5
    }

    #[test]
    fn flush_older_than_keeps_the_current_minute() {
        let mut acc = CandleAccumulator::new();
        acc.merge(&tick(100, 0, 0, 1, 2, 10, 1, 10, T_M0_A)); // minute M0
        acc.merge(&tick(101, 0, 0, 1, 2, 12, 1, 12, T_M1)); // minute M1
        // Flushing "older than M1" emits only the completed M0 candle.
        let flushed = acc.flush_older_than(M1);
        assert_eq!(flushed.len(), 1);
        assert_eq!(flushed[0].minute_start, M0);
        assert_eq!(flushed[0].vwap, 10.into(), "vwap finalised on flush");
        // M1 is still open and flushes on flush_all.
        let rest = acc.flush_all();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].minute_start, M1);
    }

    #[test]
    fn flush_all_sorts_by_minute_then_pair() {
        let mut acc = CandleAccumulator::new();
        acc.merge(&tick(101, 0, 0, 3, 2, 1, 1, 1, T_M1)); // M1, pair 3
        acc.merge(&tick(100, 0, 0, 3, 2, 1, 1, 1, T_M0_A)); // M0, pair 3
        acc.merge(&tick(100, 1, 0, 1, 2, 1, 1, 1, T_M0_B)); // M0, pair 1
        let out = acc.flush_all();
        let keys: Vec<_> = out.iter().map(|c| (c.minute_start, c.asset_id)).collect();
        assert_eq!(keys, vec![(M0, 1), (M0, 3), (M1, 3)]);
    }
}
