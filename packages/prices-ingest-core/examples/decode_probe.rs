use prices_ingest_core::bucket::CandleAccumulator;
use prices_ingest_core::canonical::AssetRegistry;
use prices_ingest_core::decode::decode_object;
use prices_ingest_core::filter::extract_trades;
use prices_ingest_core::tick::raw_trade_to_tick;
use std::{env, fs};
use stellar_xdr::curr::LedgerCloseMeta;

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: decode_probe <file.xdr.zst>");
    let bytes = fs::read(&path).expect("read file");
    let lcms = decode_object(&bytes).expect("decode_object");
    let mut registry = AssetRegistry::from_existing(vec![]);
    let mut acc = CandleAccumulator::new();
    let mut total_trades = 0usize;
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
            let tick = raw_trade_to_tick(t, &mut registry);
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
    for c in candles.iter().take(3) {
        println!(
            "   candle minute={} base={} quote={} close={} vol_base={} version={}",
            c.minute_start, c.asset_id, c.quote_asset_id, c.close, c.volume_base, c.version
        );
    }
}
