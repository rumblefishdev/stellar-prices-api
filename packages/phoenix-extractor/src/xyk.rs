use extractors_core::{
    ExtractError, ExtractResult, SorobanEventRow, SwapExtractor, TradeRow, Venue,
};

use crate::PHOENIX_XYK_EVENT_COUNT;

/// Extracts a single Phoenix XYK swap from 8 contiguous Soroban event rows.
///
/// Each row has topics = [String("swap"), String("<field>")] and data = <value>.
/// The 8 fields in emission order:
///   sender, sell_token, offer_amount, actual received amount,
///   buy_token, return_amount, spread_amount, referral_fee_amount
pub struct PhoenixXykExtractor;

impl SwapExtractor for PhoenixXykExtractor {
    fn extract(&self, rows: &[SorobanEventRow]) -> Result<ExtractResult, ExtractError> {
        if rows.len() < PHOENIX_XYK_EVENT_COUNT {
            return Err(ExtractError::InsufficientRows {
                expected: PHOENIX_XYK_EVENT_COUNT,
                actual: rows.len(),
            });
        }

        let group = &rows[..PHOENIX_XYK_EVENT_COUNT];

        let mut sender = None;
        let mut sell_token = None;
        let mut offer_amount = None;
        let mut buy_token = None;
        let mut return_amount = None;

        for row in group {
            let field_name = row
                .topics
                .get(1)
                .and_then(|t| t.as_str())
                .ok_or(ExtractError::UnexpectedTopicShape(row.event_index))?;

            match field_name {
                "sender" => sender = row.data.as_address().map(|s| s.to_string()),
                "sell_token" => sell_token = row.data.as_address().map(|s| s.to_string()),
                "offer_amount" => offer_amount = row.data.as_i128(),
                "buy_token" => buy_token = row.data.as_address().map(|s| s.to_string()),
                "return_amount" => return_amount = row.data.as_i128(),
                "actual received amount" | "spread_amount" | "referral_fee_amount" => {}
                _ => {}
            }
        }

        let first = &group[0];

        let trade = TradeRow {
            venue: Venue::Phoenix,
            contract_id: first.contract_id.clone(),
            transaction_id: first.transaction_id.clone(),
            ledger_sequence: first.ledger_sequence,
            first_event_index: first.event_index,
            token_in: sell_token.ok_or_else(|| ExtractError::MissingField("sell_token".into()))?,
            token_out: buy_token.ok_or_else(|| ExtractError::MissingField("buy_token".into()))?,
            amount_in: offer_amount
                .ok_or_else(|| ExtractError::MissingField("offer_amount".into()))?,
            amount_out: return_amount
                .ok_or_else(|| ExtractError::MissingField("return_amount".into()))?,
            fee: None,
            trader: sender,
        };

        Ok(ExtractResult {
            trades: vec![trade],
            rows_consumed: PHOENIX_XYK_EVENT_COUNT,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::*;

    #[test]
    fn xyk_extractor_decodes_8_event_group() {
        let rows = make_phoenix_xyk_events(XLM_USDC_POOL, 5);
        let result = PhoenixXykExtractor.extract(&rows).unwrap();

        assert_eq!(result.rows_consumed, 8);
        assert_eq!(result.trades.len(), 1);

        let trade = &result.trades[0];
        assert_eq!(trade.venue, Venue::Phoenix);
        assert_eq!(trade.contract_id, XLM_USDC_POOL);
        assert_eq!(trade.token_in, XLM_SAC);
        assert_eq!(trade.token_out, USDC_SAC);
        assert_eq!(trade.amount_in, 11659417676);
        assert_eq!(trade.amount_out, 1857322909);
        assert_eq!(trade.trader.as_deref(), Some(TRADER));
        assert_eq!(trade.fee, None);
    }

    #[test]
    fn xyk_extractor_works_for_pho_usdc_pool_with_alt_wasm() {
        let rows = make_phoenix_xyk_events(PHO_USDC_POOL, 0);
        let result = PhoenixXykExtractor.extract(&rows).unwrap();

        assert_eq!(result.trades.len(), 1);
        let trade = &result.trades[0];
        assert_eq!(trade.contract_id, PHO_USDC_POOL);
        assert_eq!(trade.venue, Venue::Phoenix);
        assert_eq!(trade.amount_in, 11659417676);
        assert_eq!(trade.amount_out, 1857322909);
    }

    #[test]
    fn xyk_extractor_rejects_fewer_than_8_rows() {
        let rows = make_phoenix_xyk_events(XLM_USDC_POOL, 0);
        let result = PhoenixXykExtractor.extract(&rows[..5]);
        assert!(result.is_err());
    }

    #[test]
    fn xyk_extractor_tolerates_unordered_fields() {
        let mut rows = make_phoenix_xyk_events(XLM_USDC_POOL, 0);
        rows.swap(1, 4);
        let result = PhoenixXykExtractor.extract(&rows).unwrap();

        let trade = &result.trades[0];
        assert_eq!(trade.token_in, XLM_SAC);
        assert_eq!(trade.token_out, USDC_SAC);
    }
}
