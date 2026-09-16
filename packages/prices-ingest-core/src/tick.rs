use rust_decimal::Decimal;

use crate::canonical::{AssetRegistry, CanonicalPair, canonicalise};
use crate::filter::{PriceSource, RawTrade};
use crate::price::{compute_price, offer_price, price_forming_i64};

#[derive(Debug, Clone)]
pub struct TradeTick {
    pub ledger_sequence: u32,
    pub closed_at: i64,
    /// Position of this fill's transaction in the ledger's apply order (task
    /// 0286 D1). `operation_index` RESTARTS in every transaction, so without
    /// this the "last" fill of a ledger is whichever transaction happened to
    /// use the highest operation index — wrong on 54 of 60 sampled days.
    pub transaction_index: u16,
    pub operation_index: u16,
    pub claim_index: u16,
    pub base_id: u32,
    pub quote_id: u32,
    pub price: Decimal,
    pub volume_base: Decimal,
    pub volume_quote: Decimal,
    /// May this fill's price set open/high/low/close (ADR 0287 §1)? A fill that
    /// fails the rounding bound still counts in `volume_*`, `vwap` and
    /// `trade_count` — it just never prices the candle.
    pub price_forming: bool,
}

impl TradeTick {
    /// The total fill order within a ledger. ⚠️ NOT the source of `version`:
    /// `bucket.rs` computes that from the NAMED `ledger_sequence` and
    /// `operation_index` fields, because a positional read of this tuple would
    /// silently make the transaction index the second term (task 0286 F4).
    pub fn lex_key(&self) -> (u32, u16, u16, u16) {
        (
            self.ledger_sequence,
            self.transaction_index,
            self.operation_index,
            self.claim_index,
        )
    }
}

pub fn raw_trade_to_tick(trade: &RawTrade, registry: &mut AssetRegistry) -> TradeTick {
    let pair = canonicalise(&trade.asset_sold, &trade.asset_bought, registry);

    // The one place a fill's price is decided (ADR 0287 §1). An order-book fill
    // crossed a resting offer, and that offer's own n/d is the price the trade
    // happened at — exact at any fill size, so it always forms price. A pool
    // fill, and an order-book fill whose offer entry could not be read, has only
    // its own two rounded amounts, and there the 0.1 % bound decides.
    let from_offer = match trade.price_source {
        PriceSource::Offer { n, d } => offer_price(n, d, pair.inverted),
        PriceSource::AmountRatio => None,
    };
    let (price, price_forming) = match from_offer {
        Some(price) => (price, true),
        None => (
            compute_price(trade.amount_sold, trade.amount_bought, pair.inverted),
            price_forming_i64(trade.amount_sold, trade.amount_bought),
        ),
    };

    let (volume_base, volume_quote) = canonical_volumes(trade, &pair);

    TradeTick {
        ledger_sequence: trade.ledger_sequence,
        closed_at: trade.closed_at,
        transaction_index: trade.transaction_index,
        operation_index: trade.operation_index,
        claim_index: trade.claim_index,
        base_id: pair.base_id,
        quote_id: pair.quote_id,
        price,
        volume_base,
        volume_quote,
        price_forming,
    }
}

fn canonical_volumes(trade: &RawTrade, pair: &CanonicalPair) -> (Decimal, Decimal) {
    use crate::price::stroops_to_decimal;

    let sold = stroops_to_decimal(trade.amount_sold);
    let bought = stroops_to_decimal(trade.amount_bought);

    if pair.inverted {
        (bought, sold)
    } else {
        (sold, bought)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::AssetIdentity;

    const USDC_ISSUER_ADDR: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";

    fn usdc() -> AssetIdentity {
        AssetIdentity::Credit {
            code: "USDC".to_string(),
            issuer: USDC_ISSUER_ADDR.to_string(),
        }
    }

    fn trade(amount_sold: i64, amount_bought: i64) -> RawTrade {
        RawTrade {
            ledger_sequence: 100,
            closed_at: 1_700_000_000,
            transaction_index: 3,
            operation_index: 1,
            claim_index: 2,
            asset_sold: AssetIdentity::Native,
            amount_sold,
            asset_bought: usdc(),
            amount_bought,
            price_source: PriceSource::AmountRatio,
        }
    }

    /// Task 0286 / ADR 0287 §1. The dust fill of the analysis: 17 stroops sold
    /// for 1 stroop prints the exact fraction 1/17, hundreds of percent off the
    /// market, purely because both amounts are tiny. It must not be allowed to
    /// price a candle — but it is a real trade, so its price and both volumes
    /// are computed exactly as before and still reach the accumulator.
    #[test]
    fn a_stroop_dust_fill_is_not_price_forming_but_keeps_its_volumes() {
        let mut registry = AssetRegistry::from_existing(vec![]);
        let dust = raw_trade_to_tick(&trade(17, 1), &mut registry);
        assert!(
            !dust.price_forming,
            "17 stroops against 1 cannot set a price"
        );
        assert_eq!(
            dust.price,
            compute_price(17, 1, false),
            "the price is still computed, unchanged"
        );
        assert_eq!(dust.volume_base, crate::price::stroops_to_decimal(17));
        assert_eq!(dust.volume_quote, crate::price::stroops_to_decimal(1));

        let ordinary = raw_trade_to_tick(&trade(50_000_000, 10_000_000), &mut registry);
        assert!(ordinary.price_forming, "5 XLM against 1 USDC forms price");
    }

    /// The transaction's apply order travels from the ledger through the raw
    /// trade onto the tick, and the fill key is the four-element one (D1).
    #[test]
    fn the_tick_carries_the_transaction_index_into_the_fill_key() {
        let mut registry = AssetRegistry::from_existing(vec![]);
        let tick = raw_trade_to_tick(&trade(50_000_000, 10_000_000), &mut registry);
        assert_eq!(tick.transaction_index, 3);
        assert_eq!(tick.lex_key(), (100, 3, 1, 2));
    }
}
