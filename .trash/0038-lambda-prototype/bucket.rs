//! 1-minute OHLCV bucketing.
//!
//! Per ADR 0004 §Decision: incremental-merge update preserves `open`,
//! overwrites `close`, takes `GREATEST(high)` / `LEAST(low)`, sums
//! `volume_base` / `volume_quote` / `trade_count`, and accumulates
//! VWAP numerator/denominator pairs.
//!
//! Prototype simplifications (flagged for the BE meeting):
//! - Canonical `(base, quote)` is the lexicographically smaller /
//!   larger of `(token_in, token_out)`. Production policy may differ.
//! - Prices are `f64`. Production may want a fixed-point or rational
//!   representation; the merge formula is identical either way.

use std::collections::HashMap;

use extractors_core::{TradeRow, Venue};
use serde::Serialize;

const GRANULARITY_ONE_MINUTE: &str = "1m";
const ONE_MINUTE_SECS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct OhlcvKey {
    pub timestamp_minute: i64,
    pub asset_id: String,
    pub granularity: String,
    pub quote_asset_id: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OhlcvRow {
    pub key: OhlcvKey,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume_base: i128,
    pub volume_quote: i128,
    pub trade_count: u64,
    pub vwap_numerator: f64,
    pub vwap_denominator: i128,
}

impl OhlcvRow {
    fn merge(&mut self, price: f64, volume_base: i128, volume_quote: i128) {
        if price > self.high {
            self.high = price;
        }
        if price < self.low {
            self.low = price;
        }
        self.close = price;
        self.volume_base = self.volume_base.saturating_add(volume_base);
        self.volume_quote = self.volume_quote.saturating_add(volume_quote);
        self.trade_count += 1;
        self.vwap_numerator += price * (volume_quote as f64);
        self.vwap_denominator = self.vwap_denominator.saturating_add(volume_quote);
    }

    pub fn vwap(&self) -> Option<f64> {
        if self.vwap_denominator == 0 {
            None
        } else {
            Some(self.vwap_numerator / (self.vwap_denominator as f64))
        }
    }
}

pub struct Bucketer {
    by_key: HashMap<OhlcvKey, OhlcvRow>,
}

impl Bucketer {
    pub fn new() -> Self {
        Self {
            by_key: HashMap::new(),
        }
    }

    pub fn ingest(&mut self, closed_at_unix_seconds: i64, trade: &TradeRow) {
        let (asset_id, quote_asset_id, amount_base, amount_quote) = canonical_pair(
            &trade.token_in,
            &trade.token_out,
            trade.amount_in,
            trade.amount_out,
        );
        if amount_base == 0 {
            return;
        }
        let price = (amount_quote as f64) / (amount_base as f64);
        let key = OhlcvKey {
            timestamp_minute: floor_to_minute(closed_at_unix_seconds),
            asset_id,
            granularity: GRANULARITY_ONE_MINUTE.to_string(),
            quote_asset_id,
            source: venue_to_source(&trade.venue).to_string(),
        };
        self.by_key
            .entry(key.clone())
            .and_modify(|row| row.merge(price, amount_base, amount_quote))
            .or_insert_with(|| OhlcvRow {
                key,
                open: price,
                high: price,
                low: price,
                close: price,
                volume_base: amount_base,
                volume_quote: amount_quote,
                trade_count: 1,
                vwap_numerator: price * (amount_quote as f64),
                vwap_denominator: amount_quote,
            });
    }

    pub fn drain(&mut self) -> Vec<OhlcvRow> {
        let mut rows: Vec<OhlcvRow> = self.by_key.drain().map(|(_, v)| v).collect();
        rows.sort_by(|a, b| {
            a.key
                .timestamp_minute
                .cmp(&b.key.timestamp_minute)
                .then_with(|| a.key.asset_id.cmp(&b.key.asset_id))
                .then_with(|| a.key.quote_asset_id.cmp(&b.key.quote_asset_id))
                .then_with(|| a.key.source.cmp(&b.key.source))
        });
        rows
    }
}

impl Default for Bucketer {
    fn default() -> Self {
        Self::new()
    }
}

fn floor_to_minute(unix_seconds: i64) -> i64 {
    (unix_seconds / ONE_MINUTE_SECS) * ONE_MINUTE_SECS
}

fn canonical_pair(
    token_in: &str,
    token_out: &str,
    amount_in: i128,
    amount_out: i128,
) -> (String, String, i128, i128) {
    if token_in <= token_out {
        (
            token_in.to_string(),
            token_out.to_string(),
            amount_in,
            amount_out,
        )
    } else {
        (
            token_out.to_string(),
            token_in.to_string(),
            amount_out,
            amount_in,
        )
    }
}

fn venue_to_source(v: &Venue) -> &'static str {
    match v {
        Venue::Soroswap => "soroswap",
        Venue::Aquarius => "aquarius",
        Venue::Phoenix => "phoenix",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extractors_core::Venue;

    fn trade(
        venue: Venue,
        token_in: &str,
        token_out: &str,
        amount_in: i128,
        amount_out: i128,
    ) -> TradeRow {
        TradeRow {
            venue,
            contract_id: "C".into(),
            transaction_id: "T".into(),
            ledger_sequence: 1,
            first_event_index: 0,
            token_in: token_in.into(),
            token_out: token_out.into(),
            amount_in,
            amount_out,
            fee: None,
            trader: None,
        }
    }

    #[test]
    fn floor_to_minute_rounds_down() {
        assert_eq!(floor_to_minute(0), 0);
        assert_eq!(floor_to_minute(59), 0);
        assert_eq!(floor_to_minute(60), 60);
        assert_eq!(floor_to_minute(125), 120);
    }

    #[test]
    fn canonical_pair_orders_lexicographically() {
        let (b, q, ab, aq) = canonical_pair("USDC", "XLM", 100, 200);
        assert_eq!((b.as_str(), q.as_str(), ab, aq), ("USDC", "XLM", 100, 200));

        let (b, q, ab, aq) = canonical_pair("XLM", "USDC", 100, 200);
        assert_eq!((b.as_str(), q.as_str(), ab, aq), ("USDC", "XLM", 200, 100));
    }

    #[test]
    fn single_trade_seeds_open_high_low_close_equal() {
        let mut b = Bucketer::new();
        b.ingest(1_700_000_000, &trade(Venue::Phoenix, "USDC", "XLM", 10, 50));
        let rows = b.drain();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.open, 5.0);
        assert_eq!(r.high, 5.0);
        assert_eq!(r.low, 5.0);
        assert_eq!(r.close, 5.0);
        assert_eq!(r.volume_base, 10);
        assert_eq!(r.volume_quote, 50);
        assert_eq!(r.trade_count, 1);
        assert_eq!(r.vwap(), Some(5.0));
    }

    #[test]
    fn merges_two_trades_same_bucket() {
        let mut b = Bucketer::new();
        b.ingest(1_700_000_000, &trade(Venue::Phoenix, "USDC", "XLM", 10, 50)); // price=5
        b.ingest(1_700_000_030, &trade(Venue::Phoenix, "USDC", "XLM", 5, 30)); // price=6
        let rows = b.drain();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.open, 5.0);
        assert_eq!(r.close, 6.0);
        assert_eq!(r.high, 6.0);
        assert_eq!(r.low, 5.0);
        assert_eq!(r.volume_base, 15);
        assert_eq!(r.volume_quote, 80);
        assert_eq!(r.trade_count, 2);
        // VWAP = (5*50 + 6*30) / (50+30) = (250 + 180) / 80 = 430/80 = 5.375
        let vwap = r.vwap().unwrap();
        assert!((vwap - 5.375).abs() < 1e-9);
    }

    #[test]
    fn different_minute_separate_buckets() {
        let mut b = Bucketer::new();
        b.ingest(1_700_000_000, &trade(Venue::Phoenix, "USDC", "XLM", 10, 50));
        b.ingest(1_700_000_090, &trade(Venue::Phoenix, "USDC", "XLM", 10, 50));
        assert_eq!(b.drain().len(), 2);
    }

    #[test]
    fn reverse_direction_same_pair_merges() {
        // A swap USDC→XLM at price 5, then a swap XLM→USDC at amount_in=30, amount_out=5
        // → canonical pair is still (USDC, XLM) but with flipped base/quote on the input.
        let mut b = Bucketer::new();
        b.ingest(1_700_000_000, &trade(Venue::Phoenix, "USDC", "XLM", 10, 50));
        b.ingest(1_700_000_030, &trade(Venue::Phoenix, "XLM", "USDC", 30, 5));
        let rows = b.drain();
        assert_eq!(rows.len(), 1, "reverse direction must collapse to one key");
        let r = &rows[0];
        assert_eq!(r.trade_count, 2);
    }

    #[test]
    fn different_source_separate_buckets() {
        let mut b = Bucketer::new();
        b.ingest(1_700_000_000, &trade(Venue::Phoenix, "USDC", "XLM", 10, 50));
        b.ingest(
            1_700_000_000,
            &trade(Venue::Soroswap, "USDC", "XLM", 10, 50),
        );
        assert_eq!(b.drain().len(), 2);
    }

    #[test]
    fn zero_amount_in_skipped() {
        let mut b = Bucketer::new();
        b.ingest(1_700_000_000, &trade(Venue::Phoenix, "USDC", "XLM", 0, 50));
        assert_eq!(b.drain().len(), 0);
    }
}
