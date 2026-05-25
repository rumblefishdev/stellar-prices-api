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
