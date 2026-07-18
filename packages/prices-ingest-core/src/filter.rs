use stellar_xdr::{
    ClaimAtom, LedgerCloseMeta, OperationResult, OperationResultTr, TransactionResultResult,
};

use crate::canonical::AssetIdentity;

#[derive(Debug, Clone)]
pub struct RawTrade {
    pub ledger_sequence: u32,
    pub closed_at: i64,
    pub operation_index: u16,
    pub claim_index: u16,
    pub asset_sold: AssetIdentity,
    pub amount_sold: i64,
    pub asset_bought: AssetIdentity,
    pub amount_bought: i64,
}

pub fn extract_trades(lcm: &LedgerCloseMeta) -> Vec<RawTrade> {
    let mut trades = Vec::new();

    let (sequence, closed_at) = ledger_header(lcm);
    let tx_processing = tx_processing_entries(lcm);

    for tx_ref in tx_processing {
        let result = &tx_ref.result.result.result;
        let ops = match result {
            TransactionResultResult::TxSuccess(ops)
            | TransactionResultResult::TxFeeBumpInnerSuccess(
                stellar_xdr::InnerTransactionResultPair {
                    result:
                        stellar_xdr::InnerTransactionResult {
                            result: stellar_xdr::InnerTransactionResultResult::TxSuccess(ops),
                            ..
                        },
                    ..
                },
            ) => ops,
            _ => continue,
        };

        for (op_idx, op_result) in ops.iter().enumerate() {
            let claims = match op_result {
                OperationResult::OpInner(tr) => extract_claims(tr),
                _ => continue,
            };

            for (claim_idx, claim) in claims.iter().enumerate() {
                if let Some(trade) =
                    claim_to_raw_trade(claim, sequence, closed_at, op_idx as u16, claim_idx as u16)
                {
                    trades.push(trade);
                }
            }
        }
    }

    trades
}

fn extract_claims(tr: &OperationResultTr) -> &[ClaimAtom] {
    use OperationResultTr::*;
    match tr {
        ManageSellOffer(stellar_xdr::ManageSellOfferResult::Success(s)) => &s.offers_claimed,
        ManageBuyOffer(stellar_xdr::ManageBuyOfferResult::Success(s)) => &s.offers_claimed,
        CreatePassiveSellOffer(stellar_xdr::ManageSellOfferResult::Success(s)) => &s.offers_claimed,
        PathPaymentStrictReceive(stellar_xdr::PathPaymentStrictReceiveResult::Success(s)) => {
            &s.offers
        }
        PathPaymentStrictSend(stellar_xdr::PathPaymentStrictSendResult::Success(s)) => &s.offers,
        _ => &[],
    }
}

fn claim_to_raw_trade(
    claim: &ClaimAtom,
    ledger_sequence: u32,
    closed_at: i64,
    operation_index: u16,
    claim_index: u16,
) -> Option<RawTrade> {
    let (asset_sold, amount_sold, asset_bought, amount_bought) = match claim {
        ClaimAtom::V0(c) => (
            AssetIdentity::from_xdr(&c.asset_sold),
            c.amount_sold,
            AssetIdentity::from_xdr(&c.asset_bought),
            c.amount_bought,
        ),
        ClaimAtom::OrderBook(c) => (
            AssetIdentity::from_xdr(&c.asset_sold),
            c.amount_sold,
            AssetIdentity::from_xdr(&c.asset_bought),
            c.amount_bought,
        ),
        ClaimAtom::LiquidityPool(c) => (
            AssetIdentity::from_xdr(&c.asset_sold),
            c.amount_sold,
            AssetIdentity::from_xdr(&c.asset_bought),
            c.amount_bought,
        ),
    };

    if amount_sold == 0 || amount_bought == 0 {
        tracing::warn!(
            ledger_sequence,
            operation_index,
            claim_index,
            "skipping claim with zero amount"
        );
        return None;
    }

    Some(RawTrade {
        ledger_sequence,
        closed_at,
        operation_index,
        claim_index,
        asset_sold,
        amount_sold,
        asset_bought,
        amount_bought,
    })
}

fn ledger_header(lcm: &LedgerCloseMeta) -> (u32, i64) {
    match lcm {
        LedgerCloseMeta::V0(v) => (
            v.ledger_header.header.ledger_seq,
            v.ledger_header.header.scp_value.close_time.0 as i64,
        ),
        LedgerCloseMeta::V1(v) => (
            v.ledger_header.header.ledger_seq,
            v.ledger_header.header.scp_value.close_time.0 as i64,
        ),
        LedgerCloseMeta::V2(v) => (
            v.ledger_header.header.ledger_seq,
            v.ledger_header.header.scp_value.close_time.0 as i64,
        ),
    }
}

struct TxProcessingRef<'a> {
    result: &'a stellar_xdr::TransactionResultPair,
}

fn tx_processing_entries(lcm: &LedgerCloseMeta) -> Vec<TxProcessingRef<'_>> {
    match lcm {
        LedgerCloseMeta::V0(v) => v
            .tx_processing
            .iter()
            .map(|t| TxProcessingRef { result: &t.result })
            .collect(),
        LedgerCloseMeta::V1(v) => v
            .tx_processing
            .iter()
            .map(|t| TxProcessingRef { result: &t.result })
            .collect(),
        LedgerCloseMeta::V2(v) => v
            .tx_processing
            .iter()
            .map(|t| TxProcessingRef { result: &t.result })
            .collect(),
    }
}
