//! Comet weighted-pool swap extractor (task 0300).
//!
//! Comet is a Balancer-style weighted pool. The one indexed is Blend's backstop
//! BLND/USDC pool `CAS3FL6TLZKDGGSISDBWGGPXT3NRR4DYTZD7YOD3HMYO6LTJUVGRVEAM`,
//! whose first swap is at ledger 51,500,460. It emits ONE event per swap, with
//! the tokens and the raw amounts inline in a data map — so, like Aquarius, it
//! needs no pool→token registry. Shape (real production payloads):
//!
//!   topics = [ Symbol("POOL"), Symbol("swap") ]
//!   data   = Map { caller:           Address,
//!                  token_amount_in:  I128,
//!                  token_amount_out: I128,
//!                  token_in:         Address,
//!                  token_out:        Address }
//!
//! The pool's other `POOL/*` topics — `deposit`, `join_pool`, `exit_pool`,
//! `withdraw` — and the SEP-41 `transfer`/`approve`/`burn` events of its own LP
//! token are liquidity, not trades, and are skipped.
//!
//! A newer Comet wasm (`61e1fc54…`) emits a three-symbol
//! `[swap_event, POOL, swap]` instead. This extractor deliberately does not
//! match it: it is out of scope for task 0300.
//!
//! The decoder does not judge a swap: a self-swap (`token_in == token_out`,
//! 1,459 of them on this pool) decodes faithfully and is dropped downstream by
//! the venue-neutral guard in `prices-ingest-core`'s `amm_trade_to_tick`.

use extractors_core::{
    ExtractError, ExtractResult, SorobanEventRow, SwapExtractor, TaggedValue, TradeRow, Venue,
};

/// The Symbol/String value of topic `i`. [`TaggedValue::as_str`] also answers
/// for an Address, which must never pass for an event name.
fn topic_name(row: &SorobanEventRow, i: usize) -> Option<&str> {
    match row.topics.get(i)? {
        TaggedValue::Symbol(s) | TaggedValue::String(s) => Some(s),
        _ => None,
    }
}

/// Whether `row` is a Comet pool swap: topic 0 is `"POOL"` and topic 1 is
/// `"swap"`, each a Symbol or String (an Address never passes).
///
/// The ONE predicate for this shape: `dispatch` decodes exactly these rows, and
/// `prices-ingest-core`'s dispatch-error and unregistered-pool sensors count
/// exactly these rows, so what is priced and what is counted cannot drift.
pub fn is_pool_swap(row: &SorobanEventRow) -> bool {
    topic_name(row, 0) == Some("POOL") && topic_name(row, 1) == Some("swap")
}

pub struct CometPoolExtractor;

impl CometPoolExtractor {
    /// Decode one `POOL/swap` row into a [`TradeRow`].
    fn extract_one(row: &SorobanEventRow) -> Result<TradeRow, ExtractError> {
        let TaggedValue::Map(map) = &row.data else {
            return Err(ExtractError::UnexpectedTopicShape(row.event_index));
        };
        let field = |key: &str| {
            map.iter()
                .find(|(k, _)| k.as_str() == Some(key))
                .map(|(_, v)| v)
        };
        let address = |key: &str| {
            field(key)
                .and_then(TaggedValue::as_address)
                .map(String::from)
                .ok_or_else(|| ExtractError::MissingField(key.to_string()))
        };
        let amount = |key: &str| {
            field(key)
                .and_then(TaggedValue::as_i128)
                .ok_or_else(|| ExtractError::MissingField(key.to_string()))
        };

        Ok(TradeRow {
            venue: Venue::Comet,
            contract_id: row.contract_id.clone(),
            transaction_id: row.transaction_id.clone(),
            ledger_sequence: row.ledger_sequence,
            first_event_index: row.event_index,
            token_in: address("token_in")?,
            token_out: address("token_out")?,
            amount_in: amount("token_amount_in")?,
            amount_out: amount("token_amount_out")?,
            // Weights and the swap fee are not decoded (out of scope, task 0300).
            fee: None,
            trader: field("caller")
                .and_then(TaggedValue::as_address)
                .map(String::from),
        })
    }
}

impl SwapExtractor for CometPoolExtractor {
    /// A group is the events of one (transaction, contract). Every
    /// `POOL/swap` in it is decoded independently; every other row is skipped.
    /// A malformed `POOL/swap` fails the whole group, as Aquarius does.
    fn extract(&self, rows: &[SorobanEventRow]) -> Result<ExtractResult, ExtractError> {
        if rows.is_empty() {
            return Err(ExtractError::InsufficientRows {
                expected: 1,
                actual: 0,
            });
        }

        let trades = rows
            .iter()
            .filter(|row| is_pool_swap(row))
            .map(Self::extract_one)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ExtractResult {
            rows_consumed: rows.len(),
            trades,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMET: &str = "CAS3FL6TLZKDGGSISDBWGGPXT3NRR4DYTZD7YOD3HMYO6LTJUVGRVEAM";
    const BLND: &str = "CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY";
    const USDC: &str = "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75";

    fn sym(s: &str) -> TaggedValue {
        TaggedValue::Symbol(s.to_string())
    }

    fn addr(s: &str) -> TaggedValue {
        TaggedValue::Address(s.to_string())
    }

    /// A `POOL/swap` row carrying a real event's values, in BE's key order.
    fn swap_row(
        ledger: u64,
        tx: &str,
        event_index: u32,
        caller: &str,
        amounts: (i128, i128),
        tokens: (&str, &str),
    ) -> SorobanEventRow {
        SorobanEventRow {
            contract_id: COMET.to_string(),
            transaction_id: tx.to_string(),
            ledger_sequence: ledger,
            event_index,
            topics: vec![sym("POOL"), sym("swap")],
            data: TaggedValue::Map(vec![
                (sym("caller"), addr(caller)),
                (sym("token_amount_in"), TaggedValue::I128(amounts.0)),
                (sym("token_amount_out"), TaggedValue::I128(amounts.1)),
                (sym("token_in"), addr(tokens.0)),
                (sym("token_out"), addr(tokens.1)),
            ]),
        }
    }

    /// `typical_usdc_to_blnd_recent` — ledger 64,570,597, tx idx 627, op 0, ev 21.
    fn typical_usdc_to_blnd_recent() -> SorobanEventRow {
        swap_row(
            64_570_597,
            "64570597:627",
            21,
            "CBNVK5PE7JCL773P5SHWE3YUCHQVVVPVOG72SKCEFVTZOLLQWVTCPLZ3",
            (3454229, 621636466),
            (USDC, BLND),
        )
    }

    /// `typical_blnd_to_usdc_recent` — ledger 64,573,919, tx idx 233, op 0, ev 9.
    fn typical_blnd_to_usdc_recent() -> SorobanEventRow {
        swap_row(
            64_573_919,
            "64573919:233",
            9,
            "CBNVK5PE7JCL773P5SHWE3YUCHQVVVPVOG72SKCEFVTZOLLQWVTCPLZ3",
            (1263056538, 6938342),
            (BLND, USDC),
        )
    }

    /// `largest_blnd_to_usdc_pre_exploit` — ledger 61,341,361, tx idx 299, op 0,
    /// ev 1.
    fn largest_blnd_to_usdc_pre_exploit() -> SorobanEventRow {
        swap_row(
            61_341_361,
            "61341361:299",
            1,
            "CB3JAPDEIMA3OOSALUHLYRGM2QTXGVD3EASALPFMVEU2POLLULJBT2XN",
            (20754799754016, 836304108819),
            (BLND, USDC),
        )
    }

    /// `largest_usdc_to_blnd` — ledger 61,341,820, tx idx 348, op 0, ev 1.
    fn largest_usdc_to_blnd() -> SorobanEventRow {
        swap_row(
            61_341_820,
            "61341820:348",
            1,
            "CB3JAPDEIMA3OOSALUHLYRGM2QTXGVD3EASALPFMVEU2POLLULJBT2XN",
            (200000000000, 5093532302262),
            (USDC, BLND),
        )
    }

    /// `self_swap_usdc` — ledger 64,112,340, tx idx 367, op 0, ev 4: the
    /// 2026-08-25 exploit's USDC → USDC swap.
    fn self_swap_usdc() -> SorobanEventRow {
        swap_row(
            64_112_340,
            "64112340:367",
            4,
            "CA27AABNFZJRDEPW4AGB5IEC4VNYCSWYOZJNZ6GI6GLQALFUWLTLJHYQ",
            (2594103172416, 1941196544582),
            (USDC, USDC),
        )
    }

    /// `multi_swap_tx` — ledger 64,302,520, tx idx 151, op 0, events 10, 14,
    /// 19, 23 and 27: five swaps against the pool in one transaction.
    fn multi_swap_tx() -> Vec<SorobanEventRow> {
        const CALLER: &str = "CAUA3I56LPLZUTWP43AF67DA3BSNSOLJ6M2CD46BGXCAEVI4IQMQ4EWX";
        let row =
            |ev, amounts, tokens| swap_row(64_302_520, "64302520:151", ev, CALLER, amounts, tokens);
        vec![
            row(10, (42270699754, 2606025902783), (USDC, BLND)),
            row(14, (66544251298, 961255886637), (USDC, BLND)),
            row(19, (1416931973976, 81363391287), (BLND, USDC)),
            row(23, (1889242631968, 25821262208), (BLND, USDC)),
            row(27, (261107183476, 1520891050), (BLND, USDC)),
        ]
    }

    /// SYNTHETIC — no real non-swap payload was captured. The pool's liquidity
    /// topics and its LP token's SEP-41 events; only the topics matter.
    fn liquidity_and_lp_rows() -> Vec<SorobanEventRow> {
        let row = |event_index, topics| SorobanEventRow {
            contract_id: COMET.to_string(),
            transaction_id: "tx".to_string(),
            ledger_sequence: 64_000_000,
            event_index,
            topics,
            data: TaggedValue::I128(1),
        };
        vec![
            row(0, vec![sym("POOL"), sym("deposit")]),
            row(1, vec![sym("POOL"), sym("join_pool")]),
            row(2, vec![sym("POOL"), sym("exit_pool")]),
            row(3, vec![sym("POOL"), sym("withdraw")]),
            row(4, vec![sym("transfer"), addr(COMET), addr(USDC)]),
            row(5, vec![sym("approve"), addr(COMET), addr(USDC)]),
            row(6, vec![sym("burn"), addr(COMET)]),
        ]
    }

    fn only_trade(row: SorobanEventRow) -> TradeRow {
        let mut result = CometPoolExtractor.extract(&[row]).unwrap();
        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.rows_consumed, 1);
        result.trades.remove(0)
    }

    #[test]
    fn decodes_a_real_usdc_to_blnd_swap() {
        let t = only_trade(typical_usdc_to_blnd_recent());
        assert_eq!(t.venue, Venue::Comet);
        assert_eq!(t.contract_id, COMET);
        assert_eq!(t.transaction_id, "64570597:627");
        assert_eq!(t.ledger_sequence, 64_570_597);
        assert_eq!(t.first_event_index, 21);
        assert_eq!(t.token_in, USDC);
        assert_eq!(t.token_out, BLND);
        assert_eq!(t.amount_in, 3454229);
        assert_eq!(t.amount_out, 621636466);
        assert_eq!(t.fee, None);
        assert_eq!(
            t.trader.as_deref(),
            Some("CBNVK5PE7JCL773P5SHWE3YUCHQVVVPVOG72SKCEFVTZOLLQWVTCPLZ3")
        );
    }

    #[test]
    fn decodes_a_real_blnd_to_usdc_swap() {
        let t = only_trade(typical_blnd_to_usdc_recent());
        assert_eq!(t.venue, Venue::Comet);
        assert_eq!(t.first_event_index, 9);
        assert_eq!((t.token_in.as_str(), t.token_out.as_str()), (BLND, USDC));
        assert_eq!((t.amount_in, t.amount_out), (1263056538, 6938342));
        assert_eq!(t.fee, None);
    }

    #[test]
    fn decodes_the_largest_real_swaps_exactly() {
        let t = only_trade(largest_blnd_to_usdc_pre_exploit());
        assert_eq!((t.token_in.as_str(), t.token_out.as_str()), (BLND, USDC));
        assert_eq!((t.amount_in, t.amount_out), (20754799754016, 836304108819));
        assert_eq!(t.first_event_index, 1);
        assert_eq!(
            t.trader.as_deref(),
            Some("CB3JAPDEIMA3OOSALUHLYRGM2QTXGVD3EASALPFMVEU2POLLULJBT2XN")
        );

        let t = only_trade(largest_usdc_to_blnd());
        assert_eq!((t.token_in.as_str(), t.token_out.as_str()), (USDC, BLND));
        assert_eq!((t.amount_in, t.amount_out), (200000000000, 5093532302262));
    }

    /// The decoder does not judge: a self-swap decodes as it is, and task 0300
    /// D1's guard in `amm_trade_to_tick` drops it for every venue.
    #[test]
    fn a_self_swap_decodes_faithfully() {
        let t = only_trade(self_swap_usdc());
        assert_eq!(t.token_in, USDC);
        assert_eq!(t.token_out, USDC);
        assert_eq!((t.amount_in, t.amount_out), (2594103172416, 1941196544582));
    }

    #[test]
    fn five_swaps_in_one_transaction_decode_in_event_order() {
        let result = CometPoolExtractor.extract(&multi_swap_tx()).unwrap();
        assert_eq!(result.rows_consumed, 5);
        let events: Vec<u32> = result.trades.iter().map(|t| t.first_event_index).collect();
        assert_eq!(events, vec![10, 14, 19, 23, 27]);
        assert_eq!(result.trades[0].amount_in, 42270699754);
        assert_eq!(result.trades[4].amount_out, 1520891050);
    }

    #[test]
    fn liquidity_and_lp_token_events_are_skipped() {
        let rows = liquidity_and_lp_rows();
        let result = CometPoolExtractor.extract(&rows).unwrap();
        assert!(result.trades.is_empty());
        assert_eq!(result.rows_consumed, rows.len());

        // Beside a real swap, only the swap is a trade.
        let mut mixed = rows;
        mixed.push(typical_usdc_to_blnd_recent());
        let result = CometPoolExtractor.extract(&mixed).unwrap();
        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.rows_consumed, mixed.len());
    }

    #[test]
    fn a_pool_topic_followed_by_an_address_is_not_a_swap() {
        let mut row = typical_usdc_to_blnd_recent();
        row.topics[1] = addr("swap");
        assert!(!is_pool_swap(&row));
        assert!(
            CometPoolExtractor
                .extract(&[row])
                .unwrap()
                .trades
                .is_empty()
        );
    }

    #[test]
    fn a_swap_missing_a_field_is_an_error_naming_it() {
        for key in [
            "token_in",
            "token_out",
            "token_amount_in",
            "token_amount_out",
        ] {
            let mut row = typical_usdc_to_blnd_recent();
            let TaggedValue::Map(m) = &mut row.data else {
                unreachable!()
            };
            m.retain(|(k, _)| k.as_str() != Some(key));
            match CometPoolExtractor.extract(&[row]) {
                Err(ExtractError::MissingField(f)) => assert_eq!(f, key),
                other => panic!("{key}: expected MissingField, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_swap_whose_data_is_not_a_map_is_an_unexpected_shape() {
        let mut row = typical_usdc_to_blnd_recent();
        row.data = TaggedValue::Vec(vec![TaggedValue::I128(1)]);
        assert!(matches!(
            CometPoolExtractor.extract(&[row]),
            Err(ExtractError::UnexpectedTopicShape(21))
        ));
    }

    #[test]
    fn empty_group_errors() {
        assert!(matches!(
            CometPoolExtractor.extract(&[]),
            Err(ExtractError::InsufficientRows {
                expected: 1,
                actual: 0
            })
        ));
    }

    #[test]
    fn is_pool_swap_matches_symbols_and_strings_only() {
        let with_topics = |topics| SorobanEventRow {
            topics,
            ..typical_usdc_to_blnd_recent()
        };
        assert!(is_pool_swap(&with_topics(vec![sym("POOL"), sym("swap")])));
        assert!(is_pool_swap(&with_topics(vec![
            TaggedValue::String("POOL".into()),
            sym("swap"),
        ])));
        assert!(!is_pool_swap(&with_topics(vec![addr("POOL"), sym("swap")])));
        assert!(!is_pool_swap(&with_topics(vec![sym("POOL"), addr("swap")])));
        assert!(!is_pool_swap(&with_topics(vec![
            sym("POOL"),
            sym("deposit")
        ])));
        assert!(!is_pool_swap(&with_topics(vec![sym("swap")])));
        // The newer Comet wasm 61e1fc54…'s shape — out of scope (task 0300).
        assert!(!is_pool_swap(&with_topics(vec![
            sym("swap_event"),
            sym("POOL"),
            sym("swap"),
        ])));
    }
}
