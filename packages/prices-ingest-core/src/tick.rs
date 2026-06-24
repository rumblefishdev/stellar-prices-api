use rust_decimal::Decimal;

use crate::canonical::{AssetRegistry, CanonicalPair, canonicalise};
use crate::filter::RawTrade;
use crate::price::compute_price;

#[derive(Debug, Clone)]
pub struct TradeTick {
    pub ledger_sequence: u32,
    pub closed_at: i64,
    pub operation_index: u16,
    pub claim_index: u16,
    pub base_id: u32,
    pub quote_id: u32,
    pub price: Decimal,
    pub volume_base: Decimal,
    pub volume_quote: Decimal,
}

impl TradeTick {
    pub fn lex_key(&self) -> (u32, u16, u16) {
        (self.ledger_sequence, self.operation_index, self.claim_index)
    }
}

pub fn raw_trade_to_tick(trade: &RawTrade, registry: &mut AssetRegistry) -> TradeTick {
    let pair = canonicalise(&trade.asset_sold, &trade.asset_bought, registry);
    let price = compute_price(trade.amount_sold, trade.amount_bought, pair.inverted);

    let (volume_base, volume_quote) = canonical_volumes(trade, &pair);

    TradeTick {
        ledger_sequence: trade.ledger_sequence,
        closed_at: trade.closed_at,
        operation_index: trade.operation_index,
        claim_index: trade.claim_index,
        base_id: pair.base_id,
        quote_id: pair.quote_id,
        price,
        volume_base,
        volume_quote,
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
