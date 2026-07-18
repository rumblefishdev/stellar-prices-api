//! Aquarius AMM swap extractor.
//!
//! Aquarius constant-product / stableswap pools emit a single `trade` event per
//! swap that carries the token addresses INLINE in the topics, so no pool→token
//! registry is needed (unlike Soroswap). Shape (from real mainnet samples):
//!
//!   topics = [ Symbol("trade"),
//!              Address(<sold_token>),
//!              Address(<bought_token>),
//!              Address(<trader>) ]
//!   data   = Vec[ I128(<amount_sold>), I128(<amount_bought>), I128(<fee>) ]
//!
//! The Aquarius *router* (`CBQDHNB…`) additionally emits a `swap` summary event;
//! that is a wrapper over the pool-level `trade` and is dropped upstream (the
//! VenueRegistry only maps pool contract_ids to `Venue::Aquarius`) to avoid
//! double-counting.

use extractors_core::{
    ExtractError, ExtractResult, SorobanEventRow, SwapExtractor, TradeRow, Venue,
};

/// Number of i128 entries in the `trade` event data vec: (sold, bought, fee).
const AQUARIUS_TRADE_DATA_LEN: usize = 3;

pub struct AquariusPoolExtractor;

impl AquariusPoolExtractor {
    /// Decode one Aquarius `trade` event row into a [`TradeRow`].
    fn extract_one(row: &SorobanEventRow) -> Result<TradeRow, ExtractError> {
        let topic0 = row
            .topics
            .first()
            .and_then(|t| t.as_str())
            .ok_or(ExtractError::UnexpectedTopicShape(row.event_index))?;
        if topic0 != "trade" {
            return Err(ExtractError::UnexpectedTopicShape(row.event_index));
        }

        let token_in = row
            .topics
            .get(1)
            .and_then(|t| t.as_address())
            .ok_or_else(|| ExtractError::MissingField("sold_token".into()))?
            .to_string();
        let token_out = row
            .topics
            .get(2)
            .and_then(|t| t.as_address())
            .ok_or_else(|| ExtractError::MissingField("bought_token".into()))?
            .to_string();
        let trader = row
            .topics
            .get(3)
            .and_then(|t| t.as_address())
            .map(String::from);

        let amounts = match &row.data {
            extractors_core::TaggedValue::Vec(v) => v,
            _ => return Err(ExtractError::UnexpectedTopicShape(row.event_index)),
        };
        if amounts.len() < AQUARIUS_TRADE_DATA_LEN {
            return Err(ExtractError::InsufficientRows {
                expected: AQUARIUS_TRADE_DATA_LEN,
                actual: amounts.len(),
            });
        }

        let amount_in = amounts[0]
            .as_i128()
            .ok_or_else(|| ExtractError::MissingField("amount_sold".into()))?;
        let amount_out = amounts[1]
            .as_i128()
            .ok_or_else(|| ExtractError::MissingField("amount_bought".into()))?;
        let fee = amounts[2].as_i128();

        Ok(TradeRow {
            venue: Venue::Aquarius,
            contract_id: row.contract_id.clone(),
            transaction_id: row.transaction_id.clone(),
            ledger_sequence: row.ledger_sequence,
            first_event_index: row.event_index,
            token_in,
            token_out,
            amount_in,
            amount_out,
            fee,
            trader,
        })
    }
}

impl SwapExtractor for AquariusPoolExtractor {
    /// A group is the events for one (transaction_id, contract_id). An Aquarius
    /// pool emits one `trade` per swap; multiple swaps in the same tx against the
    /// same pool produce multiple `trade` rows, each decoded independently.
    fn extract(&self, rows: &[SorobanEventRow]) -> Result<ExtractResult, ExtractError> {
        if rows.is_empty() {
            return Err(ExtractError::InsufficientRows {
                expected: 1,
                actual: 0,
            });
        }

        let mut trades = Vec::new();
        for row in rows {
            let topic0 = row.topics.first().and_then(|t| t.as_str());
            if topic0 == Some("trade") {
                trades.push(Self::extract_one(row)?);
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
    use extractors_core::TaggedValue;

    const POOL: &str = "CDE57N6XTUPBKYYDGQMXX7E7SLNOLFY3JEQB4MULSMR2AKTSAENGX2HC";
    const SOLD: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
    const BOUGHT: &str = "CAUIKL3IYGMERDRUN6YSCLWVAKIFG5Q4YJHUKM4S4NJZQIA3BAS6OJPK";
    const TRADER: &str = "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK";

    fn trade_event() -> SorobanEventRow {
        // From real sample: ledger 62078348, amount_sold 930000000,
        // amount_bought 423899086439, fee 465000.
        SorobanEventRow {
            contract_id: POOL.to_string(),
            transaction_id: "TX1".to_string(),
            ledger_sequence: 62078348,
            event_index: 5,
            topics: vec![
                TaggedValue::Symbol("trade".to_string()),
                TaggedValue::Address(SOLD.to_string()),
                TaggedValue::Address(BOUGHT.to_string()),
                TaggedValue::Address(TRADER.to_string()),
            ],
            data: TaggedValue::Vec(vec![
                TaggedValue::I128(930000000),
                TaggedValue::I128(423899086439),
                TaggedValue::I128(465000),
            ]),
        }
    }

    #[test]
    fn decodes_constant_product_trade() {
        let result = AquariusPoolExtractor.extract(&[trade_event()]).unwrap();
        assert_eq!(result.trades.len(), 1);
        let t = &result.trades[0];
        assert_eq!(t.venue, Venue::Aquarius);
        assert_eq!(t.contract_id, POOL);
        assert_eq!(t.token_in, SOLD);
        assert_eq!(t.token_out, BOUGHT);
        assert_eq!(t.amount_in, 930000000);
        assert_eq!(t.amount_out, 423899086439);
        assert_eq!(t.fee, Some(465000));
        assert_eq!(t.trader.as_deref(), Some(TRADER));
    }

    #[test]
    fn skips_non_trade_events_in_group() {
        let mut sync = trade_event();
        sync.topics[0] = TaggedValue::Symbol("sync".to_string());
        let result = AquariusPoolExtractor
            .extract(&[sync, trade_event()])
            .unwrap();
        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.rows_consumed, 2);
    }

    #[test]
    fn rejects_short_data_vec() {
        let mut ev = trade_event();
        ev.data = TaggedValue::Vec(vec![TaggedValue::I128(1)]);
        assert!(AquariusPoolExtractor.extract(&[ev]).is_err());
    }

    #[test]
    fn empty_group_errors() {
        assert!(AquariusPoolExtractor.extract(&[]).is_err());
    }
}
