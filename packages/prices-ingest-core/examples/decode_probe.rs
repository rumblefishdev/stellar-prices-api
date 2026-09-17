use prices_ingest_core::bucket::CandleAccumulator;
use prices_ingest_core::canonical::AssetRegistry;
use prices_ingest_core::decode::decode_object;
use prices_ingest_core::filter::{OfferLookupCounts, extract_trades_with_counts};
use prices_ingest_core::tick::{PricedFrom, raw_trade_to_tick_with_source};
use std::{env, fs};
use stellar_xdr::LedgerCloseMeta;

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: decode_probe <file.xdr.zst>");
    let bytes = fs::read(&path).expect("read file");
    let lcms = decode_object(&bytes).expect("decode_object");
    let mut registry = AssetRegistry::from_existing(vec![]);
    let mut acc = CandleAccumulator::new();
    let mut total_trades = 0usize;
    // Task 0286 phase 2: how many fills were priced by the offer they crossed
    // rather than by their own two amounts, and how many of the rest the 0.1 %
    // bound admits. This is the measurement that says what the offer lookup
    // buys on real ledgers — and whether a whole protocol era falls back.
    //
    // `offer_priced` counts the DECISION, not the intent: a fill whose offer
    // entry was found but turned out unusable (degenerate n/d, non-positive
    // amount) fell back to the ratio and belongs with the fallbacks. The share
    // is taken over ORDER-BOOK fills, because a pool fill can never be
    // offer-priced and would otherwise dilute the number the era is judged on.
    let mut offer_priced = 0usize;
    let mut price_forming = 0usize;
    let mut counts = OfferLookupCounts::default();
    println!("file={path} ledgers_in_file={}", lcms.len());
    for lcm in &lcms {
        let (seq, variant, txn) = match lcm {
            LedgerCloseMeta::V0(v) => (
                v.ledger_header.header.ledger_seq,
                "V0",
                v.tx_processing.len(),
            ),
            LedgerCloseMeta::V1(v) => (
                v.ledger_header.header.ledger_seq,
                "V1",
                v.tx_processing.len(),
            ),
            LedgerCloseMeta::V2(v) => (
                v.ledger_header.header.ledger_seq,
                "V2",
                v.tx_processing.len(),
            ),
        };
        let (trades, ledger_counts) = extract_trades_with_counts(lcm);
        total_trades += trades.len();
        counts.order_book_fills += ledger_counts.order_book_fills;
        counts.offer_lookup_misses += ledger_counts.offer_lookup_misses;
        counts.pool_fills += ledger_counts.pool_fills;
        for t in &trades {
            let (tick, priced_from) = raw_trade_to_tick_with_source(t, &mut registry);
            if priced_from == PricedFrom::Offer {
                offer_priced += 1;
            }
            if tick.price_forming {
                price_forming += 1;
            }
            acc.merge(&tick);
        }
        println!(
            "  seq={seq} meta={variant} tx_processing={txn} trades_extracted={}",
            trades.len()
        );
    }
    let candles = acc.flush_all();
    println!(
        "=> total_trades={total_trades} candles_produced={}",
        candles.len()
    );
    let order_book = counts.order_book_fills as usize;
    // Found an offer, could not use it. The residual the share would otherwise
    // hide behind "offer-priced".
    let unusable = order_book
        .saturating_sub(counts.offer_lookup_misses as usize)
        .saturating_sub(offer_priced);
    println!(
        "   order_book_fills={order_book} offer_priced={offer_priced} ({} of order-book fills) \
         offer_lookup_misses={} offer_unusable={unusable} pool_fills={}",
        share(offer_priced, order_book),
        counts.offer_lookup_misses,
        counts.pool_fills
    );
    println!(
        "   price_forming={price_forming} ({} of all fills)",
        share(price_forming, total_trades)
    );
    for c in candles.iter().take(3) {
        println!(
            "   candle minute={} base={} quote={} close={} vol_base={} version={}",
            c.minute_start, c.asset_id, c.quote_asset_id, c.close, c.volume_base, c.version
        );
    }
}

/// `n/total` as a percentage, and "n/a" rather than a division by zero when the
/// probe found no trades at all.
fn share(n: usize, total: usize) -> String {
    if total == 0 {
        return "n/a".to_string();
    }
    format!("{:.2}%", n as f64 * 100.0 / total as f64)
}
