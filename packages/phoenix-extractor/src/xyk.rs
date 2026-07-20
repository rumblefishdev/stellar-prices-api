use extractors_core::{
    ExtractError, ExtractResult, SorobanEventRow, SwapExtractor, TradeRow, Venue,
};

use crate::{PHOENIX_XYK_EVENT_COUNT, PHOENIX_XYK_MIN_EVENT_COUNT};

/// Extracts a single Phoenix XYK swap from a group of contiguous Soroban event
/// rows — **variable length**, because Phoenix omits optional fields.
///
/// Each row has topics = [String("swap"), String("<field>")] and data = <value>.
/// The fields in emission order, at most:
///   sender, sell_token, offer_amount, actual received amount,
///   buy_token, return_amount, spread_amount, referral_fee_amount
///
/// Only four are REQUIRED — `sell_token`, `offer_amount`, `buy_token`,
/// `return_amount`. The group is validated by their presence, never by row
/// count: a real 7-event group (no `actual received amount`) prices exactly as
/// well as an 8-event one, and 5,175 such swaps were being discarded by a
/// count gate. Groups that are not swaps at all (Phoenix liquidity events carry
/// `token_a` / `shares_amount` under a non-`swap` topic0) are still rejected —
/// by the topic0 check and by the required fields simply being absent.
pub struct PhoenixXykExtractor;

impl SwapExtractor for PhoenixXykExtractor {
    fn extract(&self, rows: &[SorobanEventRow]) -> Result<ExtractResult, ExtractError> {
        if rows.len() < PHOENIX_XYK_MIN_EVENT_COUNT {
            return Err(ExtractError::InsufficientRows {
                expected: PHOENIX_XYK_MIN_EVENT_COUNT,
                actual: rows.len(),
            });
        }

        // Cap at one swap's worth: a longer group is >1 swap in a single
        // (tx, contract), where only the first is consumed. Unobserved on prod
        // (max Phoenix group = 8), and capping preserves the pre-existing
        // behaviour rather than silently merging two swaps into one bad trade.
        let group = &rows[..rows.len().min(PHOENIX_XYK_EVENT_COUNT)];

        let mut sender = None;
        let mut sell_token = None;
        let mut offer_amount = None;
        let mut buy_token = None;
        let mut return_amount = None;

        for row in group {
            let topic0 = row
                .topics
                .first()
                .and_then(|t| t.as_str())
                .ok_or(ExtractError::UnexpectedTopicShape(row.event_index))?;
            if topic0 != "swap" {
                return Err(ExtractError::UnexpectedTopicShape(row.event_index));
            }

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
            // What this group ACTUALLY held — not the 8-event upper bound, which
            // would over-report consumption for a 7-event swap.
            trades: vec![trade],
            rows_consumed: group.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::*;

    /// The 5,175-swap regression: a 7-event group (no `actual received amount`)
    /// is a REAL, fully-priceable swap. The old `len() < 8` gate rejected every
    /// one of them — in the backfill and in live.
    #[test]
    fn xyk_extractor_decodes_7_event_group_identically_to_8() {
        let full = PhoenixXykExtractor
            .extract(&make_phoenix_xyk_events_with_fields(
                XLM_USDC_POOL,
                5,
                XYK_FIELDS_FULL,
            ))
            .unwrap();
        let seven = PhoenixXykExtractor
            .extract(&make_phoenix_xyk_events_with_fields(
                XLM_USDC_POOL,
                5,
                XYK_FIELDS_NO_ACTUAL_RECEIVED,
            ))
            .expect("7-event group is a priceable swap");

        assert_eq!(seven.trades.len(), 1);
        assert_eq!(seven.rows_consumed, 7, "consumed what the group held");

        // Every priced field must be identical — the dropped field is unused.
        let (a, b) = (&full.trades[0], &seven.trades[0]);
        assert_eq!(b.token_in, a.token_in);
        assert_eq!(b.token_out, a.token_out);
        assert_eq!(b.amount_in, a.amount_in);
        assert_eq!(b.amount_out, a.amount_out);
        assert_eq!(b.trader, a.trader);
    }

    /// `sender` is optional (`TradeRow::trader` is an `Option`), so the minimum
    /// priceable group is the four required fields.
    #[test]
    fn xyk_extractor_decodes_minimal_4_field_group() {
        let rows = make_phoenix_xyk_events_with_fields(
            XLM_USDC_POOL,
            5,
            &["sell_token", "offer_amount", "buy_token", "return_amount"],
        );
        let result = PhoenixXykExtractor.extract(&rows).unwrap();
        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].amount_in, 11659417676);
        assert_eq!(result.trades[0].trader, None, "sender absent → no trader");
    }

    /// Relaxing the count gate must NOT let Phoenix liquidity events through:
    /// 8,130 real groups shaped [sender, token_a, token_a-amount, token_b,
    /// token_b-amount] carry no `sell_token`/`buy_token` and are not swaps.
    #[test]
    fn xyk_extractor_rejects_liquidity_group() {
        let rows = make_phoenix_xyk_events_with_fields(XLM_USDC_POOL, 5, LIQUIDITY_FIELDS);
        let err = PhoenixXykExtractor
            .extract(&rows)
            .expect_err("liquidity group must not price as a swap");
        assert!(
            matches!(err, ExtractError::MissingField(_)),
            "expected MissingField, got: {err}"
        );
    }

    /// Below the four required fields there is nothing to price.
    #[test]
    fn xyk_extractor_rejects_group_under_minimum() {
        let rows = make_phoenix_xyk_events_with_fields(XLM_USDC_POOL, 5, &["sender", "sell_token"]);
        assert!(matches!(
            PhoenixXykExtractor.extract(&rows),
            Err(ExtractError::InsufficientRows {
                expected: 4,
                actual: 2
            })
        ));
    }

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

    /// Rejection must key off a MISSING REQUIRED FIELD, never the row count —
    /// this test was `xyk_extractor_rejects_fewer_than_8_rows` and encoded the
    /// count gate that discarded 5,175 real 7-event swaps. The first 5 emitted
    /// fields stop short of `return_amount`, so this group is genuinely
    /// unpriceable; assert on the reason, not on the length.
    #[test]
    fn xyk_extractor_rejects_group_missing_a_required_field() {
        let rows = make_phoenix_xyk_events(XLM_USDC_POOL, 0);
        let err = PhoenixXykExtractor
            .extract(&rows[..5])
            .expect_err("no return_amount → not priceable");
        assert!(
            matches!(&err, ExtractError::MissingField(f) if f == "return_amount"),
            "expected MissingField(return_amount), got: {err}"
        );
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
