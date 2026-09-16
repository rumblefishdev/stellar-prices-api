use rust_decimal::Decimal;

use crate::canonical::{AssetRegistry, CanonicalPair, canonicalise};
use crate::filter::{PriceSource, RawTrade};
use crate::price::{compute_price, offer_price, price_forming_i64, price_survives_column_scale};

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
    /// fails the rounding bound — or whose price underflows the candle's
    /// `Decimal(38, 14)` price columns and would store as 0 — still counts in
    /// `volume_*`, `vwap` and `trade_count`; it just never prices the candle.
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

/// Which of ADR 0287 §1's two price definitions actually produced a tick's
/// `price`.
///
/// [`crate::filter::PriceSource`] records what the ledger entries OFFERED; this
/// records what the pricing rule USED. The two differ whenever the offer entry
/// turns out to be unusable (a degenerate `n/d`, a non-positive amount), so any
/// measurement of "how many fills were offer-priced" must read this one or it
/// drifts from the ingest (task 0286, S4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricedFrom {
    /// The resting offer's own `price.n/d`.
    Offer,
    /// The fill's own two amounts, subject to the 0.1 % rounding bound.
    AmountRatio,
}

pub fn raw_trade_to_tick(trade: &RawTrade, registry: &mut AssetRegistry) -> TradeTick {
    raw_trade_to_tick_with_source(trade, registry).0
}

/// [`raw_trade_to_tick`], and which definition priced it.
pub fn raw_trade_to_tick_with_source(
    trade: &RawTrade,
    registry: &mut AssetRegistry,
) -> (TradeTick, PricedFrom) {
    let pair = canonicalise(&trade.asset_sold, &trade.asset_bought, registry);

    // The one place a fill's price is decided (ADR 0287 §1). An order-book fill
    // crossed a resting offer, and that offer's own n/d is the price the trade
    // happened at — exact at any fill size, so it always forms price. A pool
    // fill, and an order-book fill whose offer entry could not be read, has only
    // its own two rounded amounts, and there the 0.1 % bound decides.
    //
    // The offer arm carries the SAME non-positive precondition as
    // `price_forming_i64`: an offer price is exact at any fill size, but a claim
    // whose amount is negative is not a fill at all, and pricing it from the
    // offer would let it set open/high/low/close and contribute a negative
    // `volume_base` to `pf_volume`.
    let from_offer = match trade.price_source {
        PriceSource::Offer { n, d } if trade.amount_sold > 0 && trade.amount_bought > 0 => {
            offer_price(n, d, pair.inverted)
        }
        _ => None,
    };
    let (price, priced, priced_from) = match from_offer {
        Some(price) => (price, true, PricedFrom::Offer),
        None => (
            compute_price(trade.amount_sold, trade.amount_bought, pair.inverted),
            price_forming_i64(trade.amount_sold, trade.amount_bought),
            PricedFrom::AmountRatio,
        ),
    };
    // Whatever priced it, a fill that cannot PRINT a price does not form one
    // (task 0286, VERIFY-0286-local discrepancy 4). Applied to both arms on
    // purpose: an i32 `n/d` cannot reach the column's 1e-14 floor, so today the
    // rule only ever binds on a ratio price, but the invariant it protects —
    // `pf_trade_count > 0` implies a non-zero price — belongs to the candle,
    // not to one of the two pricing rules.
    let price_forming = priced && price_survives_column_scale(price);

    let (volume_base, volume_quote) = canonical_volumes(trade, &pair);

    let tick = TradeTick {
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
    };
    (tick, priced_from)
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

    /// Task 0286, VERIFY-0286-local discrepancy 4. The rounding bound passes a
    /// fill whose two legs are both enormous, and says nothing about where the
    /// quotient lands: 33 387 840 110.63 base units for 0.0001622 quote prices
    /// at ~4.86e-15, under the `Decimal(38, 14)` resolution, and stores as 0.
    /// Such a fill is NOT price-forming — otherwise its minute is written
    /// `pf_trade_count = 1` with `open = high = low = close = 0`, the shape ADR
    /// 0287 rules out. Its volumes are untouched: it is still a real trade.
    #[test]
    fn a_fill_whose_price_underflows_the_price_column_is_not_price_forming() {
        const BASE_STROOPS: i64 = 333_878_401_106_300_000;
        const QUOTE_STROOPS: i64 = 1_622;

        let mut registry = AssetRegistry::from_existing(vec![]);
        let tick = raw_trade_to_tick(&trade(BASE_STROOPS, QUOTE_STROOPS), &mut registry);
        assert!(
            crate::price::price_forming_i64(BASE_STROOPS, QUOTE_STROOPS),
            "the bound itself passes this fill — the fixture proves nothing otherwise"
        );
        assert!(
            !tick.price_forming,
            "a price that stores as 0 cannot open, close, cap or floor a candle"
        );
        assert_eq!(
            tick.price,
            compute_price(BASE_STROOPS, QUOTE_STROOPS, false),
            "the price is still computed, unchanged"
        );
        assert_eq!(
            tick.volume_base,
            crate::price::stroops_to_decimal(BASE_STROOPS)
        );
        assert_eq!(
            tick.volume_quote,
            crate::price::stroops_to_decimal(QUOTE_STROOPS)
        );
    }

    /// An offer price is exact at any fill SIZE, but the amounts still have to
    /// BE a fill. `price_forming_i64` rejects a non-positive amount because no
    /// valid claim carries one; honouring an offer price there would make such a
    /// fill set open/high/low/close and push a NEGATIVE `volume_base` into
    /// `pf_volume` — two negatives that divide back into a plausible `pf_vwap`.
    /// The offer arm is subject to the same precondition as the ratio arm.
    #[test]
    fn an_offer_price_does_not_rescue_a_non_positive_amount() {
        let mut registry = AssetRegistry::from_existing(vec![]);

        let mut negative = trade(-50_000_000, 10_000_000);
        negative.price_source = PriceSource::Offer { n: 397, d: 5_000 };
        let tick = raw_trade_to_tick(&negative, &mut registry);
        assert!(
            !tick.price_forming,
            "a claim with a negative amount is not a fill, whatever offer it names"
        );
        assert_eq!(
            tick.price,
            compute_price(-50_000_000, 10_000_000, false),
            "it falls back to the amount ratio, exactly as an unusable offer does"
        );

        let mut ordinary = trade(50_000_000, 10_000_000);
        ordinary.price_source = PriceSource::Offer { n: 397, d: 5_000 };
        let priced = raw_trade_to_tick(&ordinary, &mut registry);
        assert!(
            priced.price_forming,
            "the offer path is otherwise untouched"
        );
        assert_eq!(priced.price, Decimal::from(397) / Decimal::from(5_000));
    }

    /// The measurement the phase-3 go/no-go reads must count what the ingest
    /// DID, not what the filter FOUND: a `RawTrade` may name an offer the tick
    /// then refuses (a degenerate `n/d`, a non-positive amount). The tick
    /// reports the definition that actually priced it, so no consumer can drift
    /// from the pricing rule.
    #[test]
    fn the_tick_reports_which_definition_priced_it() {
        let mut registry = AssetRegistry::from_existing(vec![]);

        let mut offered = trade(50_000_000, 10_000_000);
        offered.price_source = PriceSource::Offer { n: 397, d: 5_000 };
        let (tick, source) = raw_trade_to_tick_with_source(&offered, &mut registry);
        assert_eq!(source, PricedFrom::Offer);
        assert_eq!(tick.price, Decimal::from(397) / Decimal::from(5_000));

        let mut degenerate = trade(50_000_000, 10_000_000);
        degenerate.price_source = PriceSource::Offer { n: 397, d: 0 };
        assert_eq!(
            raw_trade_to_tick_with_source(&degenerate, &mut registry).1,
            PricedFrom::AmountRatio,
            "an offer entry that cannot price anything did not price this fill"
        );

        assert_eq!(
            raw_trade_to_tick_with_source(&trade(17, 1), &mut registry).1,
            PricedFrom::AmountRatio,
        );
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
