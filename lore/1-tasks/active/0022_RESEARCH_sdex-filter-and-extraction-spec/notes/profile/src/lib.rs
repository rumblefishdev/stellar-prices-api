//! Shared decoding + ClaimAtom-walk helpers for the SDEX filter+extraction
//! profile harness (task 0022).
//!
//! Galexie-style files in `.temp/` are zstd-compressed `LedgerCloseMetaBatch`
//! XDR payloads (one batch per file, typically one ledger). We decompress,
//! decode, walk `tx_processing[].result.result.result.results[]`, and pick
//! out `ClaimAtom`s from the five SDEX-relevant `OperationResult` variants.

use anyhow::{Context, Result, anyhow};
use stellar_xdr::curr::{
    ClaimAtom, LedgerCloseMeta, LedgerCloseMetaBatch, Limits, OperationResult,
    OperationResultTr, ReadXdr, TransactionResult, TransactionResultResult,
};

/// Cap on decompressed payload size. Galexie batches are typically 2-5 MiB; 64 MiB headroom.
pub const MAX_DECOMPRESSED_SIZE: usize = 64 * 1024 * 1024;

/// XDR decode limits matching BE's `xdr-parser` defaults (deep nested transaction trees).
pub fn xdr_limits() -> Limits {
    Limits {
        depth: 500,
        len: MAX_DECOMPRESSED_SIZE,
    }
}

/// Decompress + decode a zstd Galexie file into a `LedgerCloseMetaBatch`.
pub fn decode_file(path: &std::path::Path) -> Result<LedgerCloseMetaBatch> {
    let compressed = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let decoded = zstd::decode_all(compressed.as_slice()).context("zstd decode")?;
    anyhow::ensure!(
        decoded.len() <= MAX_DECOMPRESSED_SIZE,
        "decompressed payload exceeds limit: {} bytes",
        decoded.len()
    );
    LedgerCloseMetaBatch::from_xdr(&decoded, xdr_limits()).context("LedgerCloseMetaBatch::from_xdr")
}

/// Yield `(ledger_seq, close_time_unix)` for a single `LedgerCloseMeta`.
pub fn ledger_id(meta: &LedgerCloseMeta) -> (u32, i64) {
    match meta {
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

/// Uniform view over a per-transaction processing entry, regardless of
/// whether the source ledger is `LedgerCloseMeta::V0/V1` (with
/// `TransactionResultMeta`) or `V2` (with `TransactionResultMetaV1`). All
/// three carry the same `result.result` payload we care about.
pub struct TxView<'a> {
    pub transaction_hash: &'a [u8; 32],
    pub result: &'a TransactionResult,
}

/// Yield uniform `TxView`s across V0/V1/V2 ledger metas.
pub fn tx_views(meta: &LedgerCloseMeta) -> Vec<TxView<'_>> {
    match meta {
        LedgerCloseMeta::V0(v) => v
            .tx_processing
            .as_slice()
            .iter()
            .map(|m| TxView {
                transaction_hash: &m.result.transaction_hash.0,
                result: &m.result.result,
            })
            .collect(),
        LedgerCloseMeta::V1(v) => v
            .tx_processing
            .as_slice()
            .iter()
            .map(|m| TxView {
                transaction_hash: &m.result.transaction_hash.0,
                result: &m.result.result,
            })
            .collect(),
        LedgerCloseMeta::V2(v) => v
            .tx_processing
            .as_slice()
            .iter()
            .map(|m| TxView {
                transaction_hash: &m.result.transaction_hash.0,
                result: &m.result.result,
            })
            .collect(),
    }
}

/// Return the union of all `TransactionResult`s in a ledger (across V0/V1/V2 metas).
pub fn tx_results(meta: &LedgerCloseMeta) -> Vec<&TransactionResult> {
    tx_views(meta).into_iter().map(|v| v.result).collect()
}

/// Operation-result variants that carry SDEX trade claims.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdexOpKind {
    ManageSellOffer,
    ManageBuyOffer,
    CreatePassiveSellOffer,
    PathPaymentStrictReceive,
    PathPaymentStrictSend,
}

/// One trade-shaped match from an SDEX op result.
#[derive(Debug)]
pub struct ExtractedClaim<'a> {
    pub op_kind: SdexOpKind,
    pub variant: ClaimAtomVariant,
    pub atom: &'a ClaimAtom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClaimAtomVariant {
    V0,
    OrderBook,
    LiquidityPool,
}

impl ClaimAtomVariant {
    pub fn of(atom: &ClaimAtom) -> Self {
        match atom {
            ClaimAtom::V0(_) => Self::V0,
            ClaimAtom::OrderBook(_) => Self::OrderBook,
            ClaimAtom::LiquidityPool(_) => Self::LiquidityPool,
        }
    }
}

/// Walk one tx's `OperationResult`s and yield trade-shaped `ClaimAtom`s
/// from successful tx + successful op + recognised SDEX-op variant.
///
/// `tx_success` short-circuits when the tx itself failed (Protocol rule:
/// failed tx reverts all ops, so the `would-have-run` atoms in
/// `TxFailed(results)` are not real trades).
pub fn walk_tx_claims<'a>(
    tx: &'a TransactionResult,
) -> impl Iterator<Item = (SdexOpKind, &'a ClaimAtom)> {
    let op_results: &'a [OperationResult] = match &tx.result {
        TransactionResultResult::TxSuccess(v) => v.as_slice(),
        _ => &[],
    };
    op_results.iter().flat_map(|op| {
        match op {
            OperationResult::OpInner(tr) => op_inner_claims(tr),
            _ => Box::new(std::iter::empty()) as Box<dyn Iterator<Item = (SdexOpKind, &'a ClaimAtom)>>,
        }
    })
}

fn op_inner_claims<'a>(
    tr: &'a OperationResultTr,
) -> Box<dyn Iterator<Item = (SdexOpKind, &'a ClaimAtom)> + 'a> {
    use stellar_xdr::curr::{
        ManageBuyOfferResult, ManageSellOfferResult, PathPaymentStrictReceiveResult,
        PathPaymentStrictSendResult,
    };
    match tr {
        OperationResultTr::ManageSellOffer(ManageSellOfferResult::Success(s)) => Box::new(
            s.offers_claimed
                .as_slice()
                .iter()
                .map(|c| (SdexOpKind::ManageSellOffer, c)),
        ),
        OperationResultTr::ManageBuyOffer(ManageBuyOfferResult::Success(s)) => Box::new(
            s.offers_claimed
                .as_slice()
                .iter()
                .map(|c| (SdexOpKind::ManageBuyOffer, c)),
        ),
        OperationResultTr::CreatePassiveSellOffer(ManageSellOfferResult::Success(s)) => Box::new(
            s.offers_claimed
                .as_slice()
                .iter()
                .map(|c| (SdexOpKind::CreatePassiveSellOffer, c)),
        ),
        OperationResultTr::PathPaymentStrictReceive(PathPaymentStrictReceiveResult::Success(s)) => {
            Box::new(
                s.offers
                    .as_slice()
                    .iter()
                    .map(|c| (SdexOpKind::PathPaymentStrictReceive, c)),
            )
        }
        OperationResultTr::PathPaymentStrictSend(PathPaymentStrictSendResult::Success(s)) => Box::new(
            s.offers
                .as_slice()
                .iter()
                .map(|c| (SdexOpKind::PathPaymentStrictSend, c)),
        ),
        _ => Box::new(std::iter::empty()),
    }
}

/// Count claims in one ledger, broken down by variant. Cheap walk.
#[derive(Default, Clone, Copy, Debug)]
pub struct LedgerClaimStats {
    pub total: u32,
    pub v0: u32,
    pub order_book: u32,
    pub liquidity_pool: u32,
    pub trade_bearing_ops: u32,
    pub total_ops: u32,
    pub successful_txs: u32,
    pub failed_txs: u32,
}

impl LedgerClaimStats {
    pub fn from_meta(meta: &LedgerCloseMeta) -> Self {
        let mut s = Self::default();
        for tx in tx_results(meta) {
            match &tx.result {
                TransactionResultResult::TxSuccess(ops) => {
                    s.successful_txs += 1;
                    for op in ops.as_slice() {
                        s.total_ops += 1;
                        if let OperationResult::OpInner(tr) = op {
                            let before = s.total;
                            for (_, atom) in op_inner_claims(tr) {
                                s.total += 1;
                                match ClaimAtomVariant::of(atom) {
                                    ClaimAtomVariant::V0 => s.v0 += 1,
                                    ClaimAtomVariant::OrderBook => s.order_book += 1,
                                    ClaimAtomVariant::LiquidityPool => s.liquidity_pool += 1,
                                }
                            }
                            if s.total > before {
                                s.trade_bearing_ops += 1;
                            }
                        }
                    }
                }
                _ => {
                    s.failed_txs += 1;
                }
            }
        }
        s
    }

    pub fn is_trade_bearing(&self) -> bool {
        self.total > 0
    }
}

/// Pretty-print a `ClaimAtom` for the worked-example notes. Returns a
/// `serde_json::Value` so the dump-examples binary can write it as JSON.
pub fn claim_atom_to_json(atom: &ClaimAtom) -> Result<serde_json::Value> {
    use stellar_xdr::curr::Asset;
    fn asset_json(a: &Asset) -> serde_json::Value {
        match a {
            Asset::Native => serde_json::json!({ "type": "native" }),
            Asset::CreditAlphanum4(c) => {
                let code = std::str::from_utf8(c.asset_code.0.as_slice())
                    .unwrap_or("<invalid>")
                    .trim_end_matches('\0')
                    .to_string();
                serde_json::json!({
                    "type": "credit_alphanum4",
                    "code": code,
                    "issuer": c.issuer.0.to_string(),
                })
            }
            Asset::CreditAlphanum12(c) => {
                let code = std::str::from_utf8(c.asset_code.0.as_slice())
                    .unwrap_or("<invalid>")
                    .trim_end_matches('\0')
                    .to_string();
                serde_json::json!({
                    "type": "credit_alphanum12",
                    "code": code,
                    "issuer": c.issuer.0.to_string(),
                })
            }
        }
    }
    Ok(match atom {
        ClaimAtom::V0(a) => serde_json::json!({
            "variant": "v0",
            "seller_ed25519_hex": hex::encode(a.seller_ed25519.0),
            "offer_id": a.offer_id,
            "asset_sold": asset_json(&a.asset_sold),
            "amount_sold_stroops": a.amount_sold,
            "asset_bought": asset_json(&a.asset_bought),
            "amount_bought_stroops": a.amount_bought,
        }),
        ClaimAtom::OrderBook(a) => serde_json::json!({
            "variant": "order_book",
            "seller_id": a.seller_id.0.to_string(),
            "offer_id": a.offer_id,
            "asset_sold": asset_json(&a.asset_sold),
            "amount_sold_stroops": a.amount_sold,
            "asset_bought": asset_json(&a.asset_bought),
            "amount_bought_stroops": a.amount_bought,
        }),
        ClaimAtom::LiquidityPool(a) => serde_json::json!({
            "variant": "liquidity_pool",
            "pool_id_hex": hex::encode(a.liquidity_pool_id.0.as_slice()),
            "asset_sold": asset_json(&a.asset_sold),
            "amount_sold_stroops": a.amount_sold,
            "asset_bought": asset_json(&a.asset_bought),
            "amount_bought_stroops": a.amount_bought,
        }),
    })
}

/// Convenience: extract all `LedgerCloseMeta`s from a batch (typically 1 per file).
pub fn batch_metas(batch: &LedgerCloseMetaBatch) -> Vec<&LedgerCloseMeta> {
    batch
        .ledger_close_metas
        .as_slice()
        .iter()
        .collect()
}

#[allow(dead_code)]
fn _unused() -> Result<()> {
    Err(anyhow!("placeholder"))
}
