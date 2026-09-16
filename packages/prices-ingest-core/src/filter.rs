use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};

use stellar_xdr::{
    ClaimAtom, LedgerCloseMeta, LedgerEntryChange, LedgerEntryData, OperationResult,
    OperationResultTr, TransactionMeta, TransactionResultResult,
};

use crate::canonical::AssetIdentity;

/// Where a fill's price comes from (ADR 0287 §1, decisions D2/D3).
///
/// An order-book fill crossed a resting offer, and the offer's published
/// `price.n/d` IS the price that trade happened at — independent of how few
/// stroops changed hands. Everything else (pool fills, and order-book fills
/// whose offer entry could not be read) is priced from its own two amounts,
/// where a few stroops print an exact but meaningless fraction and the 0.1 %
/// rounding bound decides whether the fill may set a price at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceSource {
    /// The resting offer's own price, as `n/d` in the offer's own direction
    /// (selling → buying). Read by [`crate::price::offer_price`].
    Offer { n: i32, d: i32 },
    /// `amount_bought / amount_sold` — the only price a pool fill has.
    AmountRatio,
}

/// `offer_id → (price.n, price.d)` for the offers one operation touched.
type OfferPrices = HashMap<i64, (i32, i32)>;

#[derive(Debug, Clone)]
pub struct RawTrade {
    pub ledger_sequence: u32,
    pub closed_at: i64,
    /// Position of this fill's transaction in `tx_processing`, the ledger's
    /// apply order — the same index Horizon's TOID uses (task 0286 D1).
    pub transaction_index: u16,
    pub operation_index: u16,
    pub claim_index: u16,
    pub asset_sold: AssetIdentity,
    pub amount_sold: i64,
    pub asset_bought: AssetIdentity,
    pub amount_bought: i64,
    /// Whether this fill was priced by the offer it crossed or by its own two
    /// amounts (task 0286 phase 2). Decided here, where the ledger entries are;
    /// applied in `tick.rs`, which owns every pricing decision.
    pub price_source: PriceSource,
}

/// How many fills the offer lookup priced, missed, or never applied to.
///
/// The SHARE of order-book fills that fall back is the number that says whether
/// an era's metas carry the `State` pre-images at all: a protocol era falling
/// back wholesale reproduces the very dust pricing 0286 exists to fix, and no
/// volume or trade-count reconciliation can see it — the mispricing lives only
/// in OHLC. A once-per-process warn line cannot answer that; a count can
/// (task 0286, S4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OfferLookupCounts {
    /// Fills that crossed a resting offer, i.e. claims carrying an `offer_id`.
    pub order_book_fills: u64,
    /// Of those, the ones whose offer entry could not be read: priced from the
    /// amount ratio, and therefore subject to the rounding bound.
    pub offer_lookup_misses: u64,
    /// Liquidity-pool fills, which have no offer to look up at all — the share
    /// of fills that can never be offer-priced, and must not be confused with a
    /// miss.
    pub pool_fills: u64,
}

impl OfferLookupCounts {
    /// What happened between an earlier snapshot and this one — the tally a
    /// chunk summary reports.
    pub fn since(self, earlier: Self) -> Self {
        Self {
            order_book_fills: self
                .order_book_fills
                .saturating_sub(earlier.order_book_fills),
            offer_lookup_misses: self
                .offer_lookup_misses
                .saturating_sub(earlier.offer_lookup_misses),
            pool_fills: self.pool_fills.saturating_sub(earlier.pool_fills),
        }
    }

    fn note(&mut self, is_order_book: bool, price_source: PriceSource) {
        match (is_order_book, price_source) {
            (false, _) => self.pool_fills += 1,
            (true, PriceSource::Offer { .. }) => self.order_book_fills += 1,
            (true, PriceSource::AmountRatio) => {
                self.order_book_fills += 1;
                self.offer_lookup_misses += 1;
            }
        }
    }
}

static ORDER_BOOK_FILLS: AtomicU64 = AtomicU64::new(0);
static OFFER_LOOKUP_MISSES: AtomicU64 = AtomicU64::new(0);
static POOL_FILLS: AtomicU64 = AtomicU64::new(0);

/// The process-wide totals since start-up. Snapshot one at the start of a chunk
/// and [`OfferLookupCounts::since`] gives that chunk's own share.
pub fn offer_lookup_counts() -> OfferLookupCounts {
    OfferLookupCounts {
        order_book_fills: ORDER_BOOK_FILLS.load(Ordering::Relaxed),
        offer_lookup_misses: OFFER_LOOKUP_MISSES.load(Ordering::Relaxed),
        pool_fills: POOL_FILLS.load(Ordering::Relaxed),
    }
}

pub fn extract_trades(lcm: &LedgerCloseMeta) -> Vec<RawTrade> {
    extract_trades_with_counts(lcm).0
}

/// [`extract_trades`], with this ledger's own offer-lookup tally.
///
/// The same tally is added to the process-wide counters ([`offer_lookup_counts`])
/// whichever entry point is used, so a caller that does not care still feeds the
/// chunk summary.
pub fn extract_trades_with_counts(lcm: &LedgerCloseMeta) -> (Vec<RawTrade>, OfferLookupCounts) {
    let mut trades = Vec::new();
    let mut counts = OfferLookupCounts::default();

    let (sequence, closed_at) = ledger_header(lcm);
    let tx_processing = tx_processing_entries(lcm);

    // `tx_processing` is in APPLY order, and that position is the missing term
    // of the fill key: `operation_index` restarts at 0 in every transaction, so
    // two fills of one ledger were previously ordered as if their operations
    // shared a numbering (task 0286 D1).
    for (tx_idx, tx_ref) in tx_processing.iter().enumerate() {
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

        // The offers this transaction's operations touched, one map per
        // operation index. Built once per transaction: a claim is matched to an
        // offer by `offer_id` WITHIN its own operation's changes.
        let offers = offer_prices_by_operation(tx_ref.meta, ops.len(), sequence);

        for (op_idx, op_result) in ops.iter().enumerate() {
            let claims = match op_result {
                OperationResult::OpInner(tr) => extract_claims(tr),
                _ => continue,
            };

            for (claim_idx, claim) in claims.iter().enumerate() {
                let price_source = price_source_for(claim, offers.as_deref(), op_idx, sequence);
                let is_order_book = claim_offer_id(claim).is_some();
                if let Some(trade) = claim_to_raw_trade(
                    claim,
                    sequence,
                    closed_at,
                    tx_idx as u16,
                    op_idx as u16,
                    claim_idx as u16,
                    price_source,
                ) {
                    // Counted per EMITTED fill, so the tally and the candle see
                    // the same population: a zero-amount claim is dropped above
                    // and belongs in neither.
                    counts.note(is_order_book, price_source);
                    trades.push(trade);
                }
            }
        }
    }

    ORDER_BOOK_FILLS.fetch_add(counts.order_book_fills, Ordering::Relaxed);
    OFFER_LOOKUP_MISSES.fetch_add(counts.offer_lookup_misses, Ordering::Relaxed);
    POOL_FILLS.fetch_add(counts.pool_fills, Ordering::Relaxed);

    (trades, counts)
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

#[allow(clippy::too_many_arguments)]
fn claim_to_raw_trade(
    claim: &ClaimAtom,
    ledger_sequence: u32,
    closed_at: i64,
    transaction_index: u16,
    operation_index: u16,
    claim_index: u16,
    price_source: PriceSource,
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
        transaction_index,
        operation_index,
        claim_index,
        asset_sold,
        amount_sold,
        asset_bought,
        amount_bought,
        price_source,
    })
}

/// How this claim is priced: the offer it crossed, if that offer's entry is in
/// its own operation's changes, else the amount ratio (ADR 0287 D3).
fn price_source_for(
    claim: &ClaimAtom,
    offers: Option<&[OfferPrices]>,
    operation_index: usize,
    ledger_sequence: u32,
) -> PriceSource {
    let Some(offer_id) = claim_offer_id(claim) else {
        // A pool fill has no offer at all — the type says so.
        return PriceSource::AmountRatio;
    };
    match offers
        .and_then(|per_operation| per_operation.get(operation_index))
        .and_then(|operation| operation.get(&offer_id))
    {
        Some(&(n, d)) => PriceSource::Offer { n, d },
        None => {
            // Measurable rather than guessed: the share of order-book fills
            // that fall back is what says whether the offer lookup is worth
            // what it costs (task 0286, S4). The share is counted in
            // `OfferLookupCounts`; this line only names the first one.
            OFFER_LOOKUP_MISS_WARNED.call_once(|| {
                tracing::warn!(
                    ledger_sequence,
                    operation_index,
                    offer_id,
                    "no offer entry for an order-book claim; pricing it from the amount ratio \
                     (warned once per process)"
                );
            });
            PriceSource::AmountRatio
        }
    }
}

/// The offer id a claim crossed, or `None` for a liquidity-pool fill — which
/// has no `offer_id` field in the XDR at all.
fn claim_offer_id(claim: &ClaimAtom) -> Option<i64> {
    match claim {
        ClaimAtom::V0(c) => Some(c.offer_id),
        ClaimAtom::OrderBook(c) => Some(c.offer_id),
        ClaimAtom::LiquidityPool(_) => None,
    }
}

/// One `offer_id → price` map per operation of the transaction.
///
/// `None` means the transaction's metas and results cannot be aligned by index,
/// which is the one case where pricing from an offer would be actively wrong:
/// a mis-aligned offer is a plausible-looking price from a different market.
/// The whole transaction then falls back to the amount ratio.
fn offer_prices_by_operation(
    meta: &TransactionMeta,
    operation_count: usize,
    ledger_sequence: u32,
) -> Option<Vec<OfferPrices>> {
    let changes = operation_changes(meta);
    if changes.len() != operation_count {
        META_OPERATION_MISMATCH_WARNED.call_once(|| {
            tracing::warn!(
                ledger_sequence,
                meta_operations = changes.len(),
                result_operations = operation_count,
                "transaction meta and result disagree on the operation count; pricing the whole \
                 transaction from amount ratios (warned once per process)"
            );
        });
        return None;
    }
    Some(changes.iter().map(|c| offers_in_changes(c)).collect())
}

/// `operations[i].changes` for every `TransactionMeta` version. V4 is the only
/// one that differs, and only by the wrapper (`OperationMetaV2`) — the field is
/// `changes` throughout.
fn operation_changes(meta: &TransactionMeta) -> Vec<&[LedgerEntryChange]> {
    match meta {
        TransactionMeta::V0(operations) => operations.iter().map(|o| &o.changes[..]).collect(),
        TransactionMeta::V1(m) => m.operations.iter().map(|o| &o.changes[..]).collect(),
        TransactionMeta::V2(m) => m.operations.iter().map(|o| &o.changes[..]).collect(),
        TransactionMeta::V3(m) => m.operations.iter().map(|o| &o.changes[..]).collect(),
        TransactionMeta::V4(m) => m.operations.iter().map(|o| &o.changes[..]).collect(),
    }
}

/// The offers named in one operation's changes.
///
/// `State` is the offer AS IT RESTED — the price the fill actually crossed — so
/// it wins wherever it exists, whatever order the changes arrive in. `Updated`
/// is the remainder after a partial fill, whose price is the same one and which
/// is all that survives when no pre-image was emitted. A fully consumed offer
/// leaves only a `Removed` key, which carries no price at all; those fills fall
/// back to the amount ratio.
///
/// `offer_id` is ledger-unique, so within one operation it identifies an offer
/// outright: the only possible collision is the same offer appearing as both
/// `State` and `Updated`, which is exactly what the preference resolves.
fn offers_in_changes(changes: &[LedgerEntryChange]) -> OfferPrices {
    let mut offers: HashMap<i64, (bool, (i32, i32))> = HashMap::new();
    for change in changes {
        let (entry, from_state) = match change {
            LedgerEntryChange::State(entry) => (entry, true),
            LedgerEntryChange::Updated(entry) => (entry, false),
            _ => continue,
        };
        let LedgerEntryData::Offer(offer) = &entry.data else {
            continue;
        };
        let price = (offer.price.n, offer.price.d);
        match offers.entry(offer.offer_id) {
            Entry::Vacant(slot) => {
                slot.insert((from_state, price));
            }
            Entry::Occupied(mut slot) => {
                if from_state && !slot.get().0 {
                    slot.insert((true, price));
                }
            }
        }
    }
    offers
        .into_iter()
        .map(|(offer_id, (_, price))| (offer_id, price))
        .collect()
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

/// Warned once per process each: the first occurrence is worth a line with its
/// ledger and offer id in it. The RATE is carried by [`offer_lookup_counts`] —
/// the warn says it happens, the counter says how often.
static OFFER_LOOKUP_MISS_WARNED: Once = Once::new();
static META_OPERATION_MISMATCH_WARNED: Once = Once::new();

struct TxProcessingRef<'a> {
    result: &'a stellar_xdr::TransactionResultPair,
    /// The transaction's apply meta, where the crossed offers' ledger entries
    /// live (`tx_apply_processing`, uniform across all three ledger variants).
    meta: &'a TransactionMeta,
}

fn tx_processing_entries(lcm: &LedgerCloseMeta) -> Vec<TxProcessingRef<'_>> {
    match lcm {
        LedgerCloseMeta::V0(v) => v
            .tx_processing
            .iter()
            .map(|t| TxProcessingRef {
                result: &t.result,
                meta: &t.tx_apply_processing,
            })
            .collect(),
        LedgerCloseMeta::V1(v) => v
            .tx_processing
            .iter()
            .map(|t| TxProcessingRef {
                result: &t.result,
                meta: &t.tx_apply_processing,
            })
            .collect(),
        LedgerCloseMeta::V2(v) => v
            .tx_processing
            .iter()
            .map(|t| TxProcessingRef {
                result: &t.result,
                meta: &t.tx_apply_processing,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bucket::CandleAccumulator;
    use crate::canonical::AssetRegistry;
    use crate::price::{compute_price, stroops_to_decimal};
    use crate::tick::{TradeTick, raw_trade_to_tick};
    use rust_decimal::Decimal;
    use stellar_xdr::{
        AccountId, AlphaNum4, Asset, AssetCode4, ClaimLiquidityAtom, ClaimOfferAtom,
        ClaimOfferAtomV0, LedgerCloseMetaV1, LedgerCloseMetaV2, LedgerEntry, LedgerEntryChange,
        LedgerEntryChanges, LedgerEntryData, LedgerHeader, LedgerHeaderHistoryEntry, LedgerKey,
        LedgerKeyOffer, ManageBuyOfferResult, ManageOfferSuccessResult, ManageSellOfferResult,
        OfferEntry, OperationMeta, OperationMetaV2, PathPaymentStrictReceiveResult,
        PathPaymentStrictReceiveResultSuccess, PathPaymentStrictSendResult,
        PathPaymentStrictSendResultSuccess, Price, PublicKey, StellarValue, TimePoint,
        TransactionMeta, TransactionMetaV1, TransactionMetaV2, TransactionMetaV3,
        TransactionMetaV4, TransactionResult, TransactionResultMeta, TransactionResultMetaV1,
        TransactionResultPair, Uint256,
    };

    const LEDGER: u32 = 100;
    const CLOSED_AT: u64 = 1_700_000_000;
    const OFFER_ID: i64 = 4_242;
    /// The offer of the task's phase-2 AC: 0.0794, exactly, as `n/d`.
    const OFFER_N: i32 = 397;
    const OFFER_D: i32 = 5_000;

    // ---- XDR fixtures ------------------------------------------------------
    // The first `LedgerCloseMeta` builders in the repo. Everything on the path
    // derives `Default`, so each builder names only the fields the behaviour
    // under test depends on.

    fn credit(code: &str) -> Asset {
        let mut bytes = [0u8; 4];
        bytes[..code.len()].copy_from_slice(code.as_bytes());
        Asset::CreditAlphanum4(AlphaNum4 {
            asset_code: AssetCode4(bytes),
            issuer: AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([0u8; 32]))),
        })
    }

    /// `AAAA` sold for `BBBB`: neither is a preferred quote, so `canonicalise`
    /// keeps the claim's own direction (base = the sold asset) and the tick
    /// takes `compute_price`'s NON-inverted arm — the one that must equal the
    /// offer's `n/d`.
    fn sold_asset() -> Asset {
        credit("AAAA")
    }

    fn bought_asset() -> Asset {
        credit("BBBB")
    }

    fn order_book_claim(offer_id: i64, amount_sold: i64, amount_bought: i64) -> ClaimAtom {
        ClaimAtom::OrderBook(ClaimOfferAtom {
            offer_id,
            asset_sold: sold_asset(),
            amount_sold,
            asset_bought: bought_asset(),
            amount_bought,
            ..Default::default()
        })
    }

    /// The pre-Protocol-18 claim: the same fill, on a different struct
    /// (`seller_ed25519` rather than `seller_id`) — and the variant the majority
    /// of the ~64 M ledgers phase 3 re-ingests decodes to.
    fn v0_claim(offer_id: i64, amount_sold: i64, amount_bought: i64) -> ClaimAtom {
        ClaimAtom::V0(ClaimOfferAtomV0 {
            offer_id,
            asset_sold: sold_asset(),
            amount_sold,
            asset_bought: bought_asset(),
            amount_bought,
            ..Default::default()
        })
    }

    /// The same fill with the assets swapped, which makes `canonicalise` invert
    /// the pair — the orientation in which the offer price must be read `d/n`.
    fn inverted_claim(offer_id: i64, amount_sold: i64, amount_bought: i64) -> ClaimAtom {
        ClaimAtom::OrderBook(ClaimOfferAtom {
            offer_id,
            asset_sold: bought_asset(),
            amount_sold,
            asset_bought: sold_asset(),
            amount_bought,
            ..Default::default()
        })
    }

    /// A pool fill has no `offer_id` in the XDR at all — the type-level reason
    /// pool fills can never be offer-priced (ADR 0287, amendment 2).
    fn pool_claim(amount_sold: i64, amount_bought: i64) -> ClaimAtom {
        ClaimAtom::LiquidityPool(ClaimLiquidityAtom {
            asset_sold: sold_asset(),
            amount_sold,
            asset_bought: bought_asset(),
            amount_bought,
            ..Default::default()
        })
    }

    #[derive(Clone, Copy)]
    enum ChangeKind {
        State,
        Updated,
        Removed,
    }

    fn offer_change(kind: ChangeKind, offer_id: i64, n: i32, d: i32) -> LedgerEntryChange {
        let entry = LedgerEntry {
            data: LedgerEntryData::Offer(OfferEntry {
                offer_id,
                price: Price { n, d },
                ..Default::default()
            }),
            ..Default::default()
        };
        match kind {
            ChangeKind::State => LedgerEntryChange::State(entry),
            ChangeKind::Updated => LedgerEntryChange::Updated(entry),
            // A fully consumed offer leaves only a key behind — no price.
            ChangeKind::Removed => LedgerEntryChange::Removed(LedgerKey::Offer(LedgerKeyOffer {
                offer_id,
                ..Default::default()
            })),
        }
    }

    fn changes(list: Vec<LedgerEntryChange>) -> LedgerEntryChanges {
        list.try_into().expect("changes fit VecM")
    }

    #[derive(Clone, Copy, Debug)]
    enum MetaVersion {
        V0,
        V1,
        V2,
        V3,
        V4,
    }

    const EVERY_META_VERSION: [MetaVersion; 5] = [
        MetaVersion::V0,
        MetaVersion::V1,
        MetaVersion::V2,
        MetaVersion::V3,
        MetaVersion::V4,
    ];

    /// One transaction meta per version, carrying one `LedgerEntryChanges` per
    /// operation. Only V4 wraps them differently (`OperationMetaV2`).
    fn meta(version: MetaVersion, ops: Vec<LedgerEntryChanges>) -> TransactionMeta {
        let v1_ops: Vec<OperationMeta> = ops
            .iter()
            .cloned()
            .map(|changes| OperationMeta { changes })
            .collect();
        let pack = |ops: Vec<OperationMeta>| ops.try_into().expect("operations fit VecM");
        match version {
            MetaVersion::V0 => TransactionMeta::V0(pack(v1_ops)),
            MetaVersion::V1 => TransactionMeta::V1(TransactionMetaV1 {
                operations: pack(v1_ops),
                ..Default::default()
            }),
            MetaVersion::V2 => TransactionMeta::V2(TransactionMetaV2 {
                operations: pack(v1_ops),
                ..Default::default()
            }),
            MetaVersion::V3 => TransactionMeta::V3(TransactionMetaV3 {
                operations: pack(v1_ops),
                ..Default::default()
            }),
            MetaVersion::V4 => TransactionMeta::V4(TransactionMetaV4 {
                operations: ops
                    .into_iter()
                    .map(|changes| OperationMetaV2 {
                        changes,
                        ..Default::default()
                    })
                    .collect::<Vec<_>>()
                    .try_into()
                    .expect("operations fit VecM"),
                ..Default::default()
            }),
        }
    }

    /// The five operation results that carry claims (`extract_claims`). A path
    /// payment is the one whose changes legitimately hold offers from SEVERAL
    /// markets, and it is the shape of the 2026-04-02 ledger in the analysis.
    #[derive(Clone, Copy, Debug)]
    enum Carrier {
        ManageSellOffer,
        ManageBuyOffer,
        CreatePassiveSellOffer,
        PathPaymentStrictReceive,
        PathPaymentStrictSend,
    }

    const EVERY_CLAIM_CARRIER: [Carrier; 5] = [
        Carrier::ManageSellOffer,
        Carrier::ManageBuyOffer,
        Carrier::CreatePassiveSellOffer,
        Carrier::PathPaymentStrictReceive,
        Carrier::PathPaymentStrictSend,
    ];

    fn op_result(carrier: Carrier, claims: Vec<ClaimAtom>) -> OperationResult {
        let claimed = || ManageOfferSuccessResult {
            offers_claimed: claims.clone().try_into().expect("claims fit VecM"),
            ..Default::default()
        };
        let offers = || claims.clone().try_into().expect("claims fit VecM");
        OperationResult::OpInner(match carrier {
            Carrier::ManageSellOffer => {
                OperationResultTr::ManageSellOffer(ManageSellOfferResult::Success(claimed()))
            }
            Carrier::ManageBuyOffer => {
                OperationResultTr::ManageBuyOffer(ManageBuyOfferResult::Success(claimed()))
            }
            Carrier::CreatePassiveSellOffer => {
                OperationResultTr::CreatePassiveSellOffer(ManageSellOfferResult::Success(claimed()))
            }
            Carrier::PathPaymentStrictReceive => OperationResultTr::PathPaymentStrictReceive(
                PathPaymentStrictReceiveResult::Success(PathPaymentStrictReceiveResultSuccess {
                    offers: offers(),
                    ..Default::default()
                }),
            ),
            Carrier::PathPaymentStrictSend => OperationResultTr::PathPaymentStrictSend(
                PathPaymentStrictSendResult::Success(PathPaymentStrictSendResultSuccess {
                    offers: offers(),
                    ..Default::default()
                }),
            ),
        })
    }

    fn result_pair(ops: Vec<Vec<ClaimAtom>>) -> TransactionResultPair {
        result_pair_with(&vec![Carrier::ManageSellOffer; ops.len()], ops)
    }

    /// One carrier per operation, so a transaction can mix a path payment with a
    /// manage-offer the way a real ledger does.
    fn result_pair_with(carriers: &[Carrier], ops: Vec<Vec<ClaimAtom>>) -> TransactionResultPair {
        let op_results: Vec<OperationResult> = carriers
            .iter()
            .zip(ops)
            .map(|(carrier, claims)| op_result(*carrier, claims))
            .collect();
        TransactionResultPair {
            result: TransactionResult {
                result: TransactionResultResult::TxSuccess(
                    op_results.try_into().expect("results fit VecM"),
                ),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn transaction(ops: Vec<Vec<ClaimAtom>>, meta: TransactionMeta) -> TransactionResultMeta {
        transaction_with(&vec![Carrier::ManageSellOffer; ops.len()], ops, meta)
    }

    fn transaction_with(
        carriers: &[Carrier],
        ops: Vec<Vec<ClaimAtom>>,
        meta: TransactionMeta,
    ) -> TransactionResultMeta {
        TransactionResultMeta {
            result: result_pair_with(carriers, ops),
            tx_apply_processing: meta,
            ..Default::default()
        }
    }

    fn header() -> LedgerHeaderHistoryEntry {
        LedgerHeaderHistoryEntry {
            header: LedgerHeader {
                ledger_seq: LEDGER,
                scp_value: StellarValue {
                    close_time: TimePoint(CLOSED_AT),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn ledger(txs: Vec<TransactionResultMeta>) -> LedgerCloseMeta {
        LedgerCloseMeta::V1(LedgerCloseMetaV1 {
            ledger_header: header(),
            tx_processing: txs.try_into().expect("transactions fit VecM"),
            ..Default::default()
        })
    }

    /// `LedgerCloseMeta::V2` keeps the same meta one wrapper deeper
    /// (`TransactionResultMetaV1`) — the variant every ledger after Protocol 23
    /// arrives in.
    fn ledger_v2(ops: Vec<Vec<ClaimAtom>>, meta: TransactionMeta) -> LedgerCloseMeta {
        LedgerCloseMeta::V2(LedgerCloseMetaV2 {
            ledger_header: header(),
            tx_processing: vec![TransactionResultMetaV1 {
                result: result_pair(ops),
                tx_apply_processing: meta,
                ..Default::default()
            }]
            .try_into()
            .expect("transactions fit VecM"),
            ..Default::default()
        })
    }

    /// One fixture ledger through the whole classic path: extract, canonicalise,
    /// price. What the candle sees.
    fn ticks(lcm: &LedgerCloseMeta) -> Vec<TradeTick> {
        let mut registry = AssetRegistry::from_existing(vec![]);
        extract_trades(lcm)
            .iter()
            .map(|t| raw_trade_to_tick(t, &mut registry))
            .collect()
    }

    /// A single-operation transaction whose one claim crosses `OFFER_ID`.
    fn one_fill(
        version: MetaVersion,
        claim: ClaimAtom,
        ops: Vec<LedgerEntryChange>,
    ) -> Vec<TradeTick> {
        ticks(&ledger(vec![transaction(
            vec![vec![claim]],
            meta(version, vec![changes(ops)]),
        )]))
    }

    fn offer_price_decimal() -> Decimal {
        Decimal::from(OFFER_N) / Decimal::from(OFFER_D)
    }

    // ---- ADR 0287 D2/D3: the resting offer prices an order-book fill -------

    /// The task's phase-2 AC. 17 stroops against 1 print the exact fraction
    /// 1/17 — dust, hundreds of percent off the market — but the fill crossed a
    /// resting offer at 0.0794, and THAT is the price the trade happened at. An
    /// offer price does not come from the fill's own rounded amounts, so no
    /// rounding bound applies to it: it always forms price.
    #[test]
    fn an_order_book_fill_is_priced_by_the_resting_offer_and_always_forms_price() {
        let ticks = one_fill(
            MetaVersion::V1,
            order_book_claim(OFFER_ID, 17, 1),
            vec![offer_change(ChangeKind::State, OFFER_ID, OFFER_N, OFFER_D)],
        );
        assert_eq!(ticks.len(), 1);
        assert_eq!(
            ticks[0].price,
            offer_price_decimal(),
            "the offer's own 397/5000, not the 1/17 the two amounts print"
        );
        assert!(
            ticks[0].price_forming,
            "an offer price is exact at any fill size"
        );
    }

    /// A pool fill has no offer to read, so it keeps the amount ratio and the
    /// 0.1 % bound decides — 34 stroops against 5 is dust. It is still a real
    /// trade: both volumes reach the candle.
    #[test]
    fn a_pool_dust_fill_keeps_the_amount_ratio_and_does_not_form_price() {
        let ticks = one_fill(MetaVersion::V1, pool_claim(34, 5), vec![]);
        assert_eq!(ticks[0].price, compute_price(34, 5, false), "5/34");
        assert!(!ticks[0].price_forming, "34 stroops against 5 is dust");
        assert_eq!(ticks[0].volume_base, stroops_to_decimal(34));
        assert_eq!(ticks[0].volume_quote, stroops_to_decimal(5));
    }

    /// The offer lives in `operations[i].changes` in every `TransactionMeta`
    /// version — V4 only wraps it in `OperationMetaV2`.
    #[test]
    fn the_offer_is_found_in_every_transaction_meta_version() {
        for version in EVERY_META_VERSION {
            let ticks = one_fill(
                version,
                order_book_claim(OFFER_ID, 17, 1),
                vec![offer_change(ChangeKind::State, OFFER_ID, OFFER_N, OFFER_D)],
            );
            assert_eq!(
                ticks[0].price,
                offer_price_decimal(),
                "meta {version:?} must yield the offer price"
            );
            assert!(ticks[0].price_forming, "meta {version:?}");
        }
    }

    /// The same, through `LedgerCloseMeta::V2`'s deeper tx_processing wrapper.
    #[test]
    fn a_ledger_close_meta_v2_reaches_the_same_offer() {
        let mut registry = AssetRegistry::from_existing(vec![]);
        let lcm = ledger_v2(
            vec![vec![order_book_claim(OFFER_ID, 17, 1)]],
            meta(
                MetaVersion::V4,
                vec![changes(vec![offer_change(
                    ChangeKind::State,
                    OFFER_ID,
                    OFFER_N,
                    OFFER_D,
                )])],
            ),
        );
        let trades = extract_trades(&lcm);
        let tick = raw_trade_to_tick(&trades[0], &mut registry);
        assert_eq!(tick.price, offer_price_decimal());
        assert!(tick.price_forming);
    }

    /// `State` is the offer as it rested — the price the fill crossed.
    /// `Updated` is the remainder afterwards, the second choice, and it is not
    /// preferred just because it appears first in the change list.
    #[test]
    fn the_state_pre_image_wins_over_the_updated_remainder() {
        let ticks = one_fill(
            MetaVersion::V1,
            order_book_claim(OFFER_ID, 17, 1),
            vec![
                offer_change(ChangeKind::Updated, OFFER_ID, 1, 2),
                offer_change(ChangeKind::State, OFFER_ID, OFFER_N, OFFER_D),
            ],
        );
        assert_eq!(ticks[0].price, offer_price_decimal());
    }

    /// A partially filled offer whose pre-image is absent still has its price in
    /// the `Updated` remainder — better than falling back to the amounts.
    #[test]
    fn the_updated_remainder_prices_the_fill_when_no_pre_image_exists() {
        let ticks = one_fill(
            MetaVersion::V1,
            order_book_claim(OFFER_ID, 17, 1),
            vec![offer_change(
                ChangeKind::Updated,
                OFFER_ID,
                OFFER_N,
                OFFER_D,
            )],
        );
        assert_eq!(ticks[0].price, offer_price_decimal());
        assert!(ticks[0].price_forming);
    }

    /// A fully consumed offer leaves a `Removed` key, which carries no price —
    /// and an offer belonging to another operation is not this fill's. Both are
    /// "no entry found": the amount ratio and the bound take over.
    #[test]
    fn a_missing_offer_entry_falls_back_to_the_amount_ratio_and_the_bound() {
        let removed = one_fill(
            MetaVersion::V1,
            order_book_claim(OFFER_ID, 17, 1),
            vec![offer_change(
                ChangeKind::Removed,
                OFFER_ID,
                OFFER_N,
                OFFER_D,
            )],
        );
        assert_eq!(removed[0].price, compute_price(17, 1, false), "1/17");
        assert!(!removed[0].price_forming);

        let other_offer = one_fill(
            MetaVersion::V1,
            order_book_claim(OFFER_ID, 17, 1),
            vec![offer_change(
                ChangeKind::State,
                OFFER_ID + 1,
                OFFER_N,
                OFFER_D,
            )],
        );
        assert_eq!(other_offer[0].price, compute_price(17, 1, false));
        assert!(!other_offer[0].price_forming);
    }

    /// `Price` is two `i32`s and nothing in the XDR forbids a zero or negative
    /// one. A degenerate price is not a price: fall back, never divide by zero.
    #[test]
    fn a_degenerate_offer_price_falls_back_to_the_amount_ratio() {
        for (n, d) in [(OFFER_N, 0), (0, OFFER_D), (-OFFER_N, OFFER_D)] {
            let ticks = one_fill(
                MetaVersion::V1,
                order_book_claim(OFFER_ID, 17, 1),
                vec![offer_change(ChangeKind::State, OFFER_ID, n, d)],
            );
            assert_eq!(
                ticks[0].price,
                compute_price(17, 1, false),
                "offer {n}/{d} is unusable"
            );
            assert!(!ticks[0].price_forming, "offer {n}/{d} is unusable");
        }
    }

    /// The offer's `n/d` is the price of what it sells in terms of what it buys.
    /// When `canonicalise` inverts the pair, the candle's price is the other way
    /// up — `d/n`, exactly as `compute_price` inverts the amount ratio.
    #[test]
    fn an_inverted_pair_reads_the_offer_price_upside_down() {
        let ticks = one_fill(
            MetaVersion::V1,
            inverted_claim(OFFER_ID, 17, 1),
            vec![offer_change(ChangeKind::State, OFFER_ID, OFFER_N, OFFER_D)],
        );
        assert_eq!(
            ticks[0].price,
            Decimal::from(OFFER_D) / Decimal::from(OFFER_N),
            "5000/397"
        );
        assert!(ticks[0].price_forming);
    }

    /// Each operation's changes belong to that operation. A lookup that lost the
    /// operation index would price a fill from a neighbour's offer and never
    /// say so.
    #[test]
    fn each_operation_is_priced_from_its_own_changes() {
        let ticks = ticks(&ledger(vec![transaction(
            vec![
                vec![order_book_claim(OFFER_ID, 17, 1)],
                vec![order_book_claim(OFFER_ID + 1, 17, 1)],
            ],
            meta(
                MetaVersion::V1,
                vec![
                    changes(vec![offer_change(
                        ChangeKind::State,
                        OFFER_ID,
                        OFFER_N,
                        OFFER_D,
                    )]),
                    changes(vec![offer_change(ChangeKind::State, OFFER_ID + 1, 1, 4)]),
                ],
            ),
        )]));
        assert_eq!(ticks[0].price, offer_price_decimal());
        assert_eq!(ticks[1].price, Decimal::from(1) / Decimal::from(4));
    }

    /// If the meta's operation count does not match the result's, the two lists
    /// cannot be aligned by index, and a mis-aligned offer is worse than no
    /// offer: the WHOLE transaction falls back to the amount ratio.
    #[test]
    fn a_meta_with_a_different_operation_count_falls_back_for_the_whole_transaction() {
        let ticks = ticks(&ledger(vec![transaction(
            vec![
                vec![order_book_claim(OFFER_ID, 17, 1)],
                vec![order_book_claim(OFFER_ID + 1, 17, 1)],
            ],
            meta(
                MetaVersion::V1,
                vec![changes(vec![offer_change(
                    ChangeKind::State,
                    OFFER_ID,
                    OFFER_N,
                    OFFER_D,
                )])],
            ),
        )]));
        assert_eq!(ticks.len(), 2);
        for tick in &ticks {
            assert_eq!(tick.price, compute_price(17, 1, false));
            assert!(!tick.price_forming);
        }
    }

    /// `ClaimAtom::V0` is what every pre-Protocol-18 result decodes to — the
    /// majority of the history phase 3 re-ingests — and its `offer_id` sits on a
    /// DIFFERENT struct. If that arm ever stops yielding an id, the whole
    /// pre-2021 SDEX history silently reverts to ratio + bound pricing, which is
    /// the defect 0286 exists to fix, and no volume check can see it.
    #[test]
    fn a_pre_protocol_18_claim_is_priced_by_its_offer_too() {
        for claim in [order_book_claim(OFFER_ID, 17, 1), v0_claim(OFFER_ID, 17, 1)] {
            let ticks = one_fill(
                MetaVersion::V0,
                claim,
                vec![offer_change(ChangeKind::State, OFFER_ID, OFFER_N, OFFER_D)],
            );
            assert_eq!(ticks[0].price, offer_price_decimal());
            assert!(ticks[0].price_forming);
        }
    }

    /// The production shape: ONE operation sweeping several resting offers. The
    /// map built for that operation then holds many entries, and each claim must
    /// take the offer its own `offer_id` names — a lookup that took "the offer in
    /// this operation" would price every fill of a book sweep from some other
    /// maker's offer and never say so.
    #[test]
    fn two_claims_in_one_operation_each_take_their_own_offer() {
        let ticks = ticks(&ledger(vec![transaction(
            vec![vec![
                order_book_claim(OFFER_ID, 17, 1),
                order_book_claim(OFFER_ID + 1, 17, 1),
            ]],
            meta(
                MetaVersion::V1,
                vec![changes(vec![
                    offer_change(ChangeKind::State, OFFER_ID, OFFER_N, OFFER_D),
                    offer_change(ChangeKind::State, OFFER_ID + 1, 1, 4),
                ])],
            ),
        )]));
        assert_eq!(ticks.len(), 2);
        assert_eq!(
            ticks[0].price,
            offer_price_decimal(),
            "claim 0 takes 397/5000"
        );
        assert_eq!(
            ticks[1].price,
            Decimal::from(1) / Decimal::from(4),
            "claim 1 takes its OWN offer, 1/4"
        );
    }

    /// All five claim carriers reach the lookup, not just the manage-sell-offer
    /// every other fixture uses. A regression that loses one carrier's claims
    /// loses every fill of that operation type.
    #[test]
    fn every_claim_carrier_reaches_the_offer_lookup() {
        for carrier in EVERY_CLAIM_CARRIER {
            let ticks = ticks(&ledger(vec![transaction_with(
                &[carrier],
                vec![vec![order_book_claim(OFFER_ID, 17, 1)]],
                meta(
                    MetaVersion::V1,
                    vec![changes(vec![offer_change(
                        ChangeKind::State,
                        OFFER_ID,
                        OFFER_N,
                        OFFER_D,
                    )])],
                ),
            )]));
            assert_eq!(ticks.len(), 1, "{carrier:?} must yield its claim");
            assert_eq!(ticks[0].price, offer_price_decimal(), "{carrier:?}");
            assert!(ticks[0].price_forming, "{carrier:?}");
        }
    }

    /// The 2026-04-02 ledger of the analysis, through the real carriers: a path
    /// payment in the FIRST transaction sweeps two offers (its last fill is claim
    /// 1), a manage-offer in the SECOND fills once (claim 0). Both operations are
    /// at index 0 — only the transaction index ranks them — so a key without it
    /// closes the minute on the path payment's fill instead of the manage-offer's.
    #[test]
    fn a_path_payment_sweep_closes_before_a_later_manage_offer() {
        let lcm = ledger(vec![
            transaction_with(
                &[Carrier::PathPaymentStrictReceive],
                vec![vec![
                    order_book_claim(OFFER_ID, 17, 1),
                    order_book_claim(OFFER_ID + 1, 17, 1),
                ]],
                meta(
                    MetaVersion::V1,
                    vec![changes(vec![
                        offer_change(ChangeKind::State, OFFER_ID, OFFER_N, OFFER_D),
                        offer_change(ChangeKind::State, OFFER_ID + 1, 1, 8),
                    ])],
                ),
            ),
            transaction_with(
                &[Carrier::ManageSellOffer],
                vec![vec![order_book_claim(OFFER_ID + 2, 17, 1)]],
                meta(
                    MetaVersion::V1,
                    vec![changes(vec![offer_change(
                        ChangeKind::State,
                        OFFER_ID + 2,
                        1,
                        4,
                    )])],
                ),
            ),
        ]);
        let ticks = ticks(&lcm);
        let keys: Vec<_> = ticks.iter().map(|t| t.lex_key()).collect();
        assert_eq!(
            keys,
            vec![(LEDGER, 0, 0, 0), (LEDGER, 0, 0, 1), (LEDGER, 1, 0, 0)]
        );

        let mut acc = CandleAccumulator::new();
        for tick in &ticks {
            acc.merge(tick);
        }
        let candle = &acc.flush_all()[0];
        assert_eq!(
            candle.open,
            offer_price_decimal(),
            "open = the path payment's first fill"
        );
        assert_eq!(
            candle.close,
            Decimal::from(1) / Decimal::from(4),
            "close = the manage-offer fill: its transaction applied second"
        );
    }

    // ---- WR-05: the fallback share is counted, not warned about once -------

    /// A single `Once` warn line says nothing about the SHARE of fills that fell
    /// back — and a protocol era whose metas carry no `State` pre-image falls
    /// back wholesale while every volume and trade-count reconciliation passes.
    /// One ledger with one of each kind: found, missed, never applicable.
    #[test]
    fn the_offer_lookup_counts_what_it_priced_missed_and_never_applied_to() {
        let lcm = ledger(vec![transaction(
            vec![vec![
                order_book_claim(OFFER_ID, 17, 1),
                order_book_claim(OFFER_ID + 9, 17, 1),
                pool_claim(34, 5),
            ]],
            meta(
                MetaVersion::V1,
                vec![changes(vec![offer_change(
                    ChangeKind::State,
                    OFFER_ID,
                    OFFER_N,
                    OFFER_D,
                )])],
            ),
        )]);
        let (trades, counts) = extract_trades_with_counts(&lcm);
        assert_eq!(trades.len(), 3);
        assert_eq!(
            counts,
            OfferLookupCounts {
                order_book_fills: 2,
                offer_lookup_misses: 1,
                pool_fills: 1,
            }
        );
    }

    /// The same tally accumulates process-wide, which is what a chunk summary
    /// reports without a log line per fill.
    #[test]
    fn the_process_wide_counters_accumulate_across_ledgers() {
        let before = offer_lookup_counts();
        let _ = extract_trades(&ledger(vec![transaction(
            vec![vec![order_book_claim(OFFER_ID, 17, 1), pool_claim(34, 5)]],
            meta(MetaVersion::V1, vec![changes(vec![])]),
        )]));
        // `>=`: other tests in this binary share the process-wide counters.
        let delta = offer_lookup_counts().since(before);
        assert!(delta.order_book_fills >= 1, "{delta:?}");
        assert!(delta.offer_lookup_misses >= 1, "{delta:?}");
        assert!(delta.pool_fills >= 1, "{delta:?}");
    }

    /// The fill key of task 0286 D1 still comes from the ledger, not from the
    /// offer lookup: one transaction's operations and claims number from zero.
    #[test]
    fn the_fill_key_counts_transactions_operations_and_claims() {
        let lcm = ledger(vec![
            transaction(
                vec![vec![pool_claim(34, 5), pool_claim(34, 5)]],
                meta(MetaVersion::V1, vec![changes(vec![])]),
            ),
            transaction(
                vec![vec![pool_claim(34, 5)]],
                meta(MetaVersion::V1, vec![changes(vec![])]),
            ),
        ]);
        let keys: Vec<_> = ticks(&lcm).iter().map(|t| t.lex_key()).collect();
        assert_eq!(
            keys,
            vec![(LEDGER, 0, 0, 0), (LEDGER, 0, 0, 1), (LEDGER, 1, 0, 0)]
        );
    }
}
