use prices_ingest_core::bucket::CandleAccumulator;
use prices_ingest_core::canonical::AssetRegistry;
use prices_ingest_core::decode::decode_object;
use prices_ingest_core::filter::{PriceSource, extract_trades};
use prices_ingest_core::tick::raw_trade_to_tick;
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
    // buys on real ledgers.
    let mut offer_priced = 0usize;
    let mut price_forming = 0usize;
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
        let trades = extract_trades(lcm);
        total_trades += trades.len();
        for t in &trades {
            if matches!(t.price_source, PriceSource::Offer { .. }) {
                offer_priced += 1;
            }
            let tick = raw_trade_to_tick(t, &mut registry);
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
    println!(
        "   offer_priced={offer_priced} ({}) price_forming={price_forming} ({})",
        share(offer_priced, total_trades),
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
