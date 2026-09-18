//! Task 0286 / ADR 0287 — a real candle, through the real writer, into a real
//! ClickHouse, read back on all eighteen columns.
//!
//!     cargo test -p prices-ingest-core --test candle_write_it -- --ignored
//!
//! The property this exists for cannot be seen from either side alone. The
//! clickhouse crate routes an INSERT by struct field NAME, so a candle column
//! `OhlcvRow` does not name is not an error — the server computes it from the
//! column DEFAULT. And the pf columns' DEFAULTs are exactly the pre-0286
//! meaning (`pf_trade_count DEFAULT trade_count`), so a writer that forgot them
//! would report this minute's dust fill as price-forming and every unit test on
//! both sides would still pass.
//!
//! The minute here holds two ordinary fills and one stroop-dust fill. The
//! assertion that matters is `pf_trade_count = 2` — the value the writer sent —
//! and NOT 3, the value the DEFAULT would have produced from `trade_count`.
//!
//! Owns an isolated scratch database and drops it at the end; the real `prices`
//! database is never touched. This is what `OhlcvWriter::write_candles_into`
//! exists for — `write_candles` hardcodes the production table name.

use clickhouse::Client;
use prices_ingest_core::{
    AssetIdentity, AssetRegistry, CandleAccumulator, OhlcvCandle, OhlcvWriter, PriceSource,
    RawTrade, raw_trade_to_tick,
};

const USDC_ISSUER_ADDR: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";
/// 2023-11-14T22:13:20Z. Its minute starts at 1_699_999_980.
const CLOSED_AT: i64 = 1_700_000_000;
const MINUTE_START: u32 = 1_699_999_980;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

/// One classic XLM→USDC fill, through the REAL classifier — the test never sets
/// `price_forming` by hand, so a regression in the bound shows up here too.
fn fill(tx: u16, xlm_stroops: i64, usdc_stroops: i64) -> RawTrade {
    RawTrade {
        ledger_sequence: 100,
        closed_at: CLOSED_AT,
        transaction_index: tx,
        operation_index: 0,
        claim_index: 0,
        asset_sold: AssetIdentity::Native,
        amount_sold: xlm_stroops,
        asset_bought: AssetIdentity::Credit {
            code: "USDC".to_string(),
            issuer: USDC_ISSUER_ADDR.to_string(),
        },
        amount_bought: usdc_stroops,
        price_source: PriceSource::AmountRatio,
    }
}

/// The minute under test: 5 XLM at 0.2, then 10 XLM at 0.3, then 17 stroops
/// against 1 — dust, last in fill order, which before this task would have been
/// the minute's close and its low.
fn candles() -> Vec<OhlcvCandle> {
    let mut registry = AssetRegistry::from_existing(vec![]);
    let mut acc = CandleAccumulator::new();
    for (tx, sold, bought) in [
        (0u16, 50_000_000i64, 10_000_000i64),
        (2, 100_000_000, 30_000_000),
        (3, 17, 1),
    ] {
        let trade = fill(tx, sold, bought);
        acc.merge(&raw_trade_to_tick(&trade, &mut registry));
    }
    acc.flush_all()
}

fn approx(got: f64, want: f64, what: &str) {
    assert!(
        (got - want).abs() < 1e-9,
        "{what}: expected {want}, got {got}"
    );
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_written_candle_round_trips_every_column_including_the_pf_ones() {
    let db = "it_candle_write";
    let admin = Client::default().with_url(ch_url());

    admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!("CREATE DATABASE {db}"))
        .execute()
        .await
        .unwrap();
    prices_clickhouse::apply_sql(&admin, &rewrite(prices_clickhouse::INIT_SQL, db))
        .await
        .expect("apply init schema");

    let candles = candles();
    assert_eq!(candles.len(), 1, "one minute, one pair, one candle");

    OhlcvWriter::plaintext(&ch_url())
        .write_candles_into(&format!("{db}.price_ohlcv_1m"), &candles, "sdex")
        .await
        .expect("write the candle");

    let rows: u64 = admin
        .query(&format!("SELECT count() FROM {db}.price_ohlcv_1m FINAL"))
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(rows, 1);

    // Identity, counts and version.
    let (timestamp, asset_id, quote_asset_id, source, trade_count, version, pf_trade_count): (
        u32,
        u32,
        u32,
        String,
        u32,
        u64,
        u32,
    ) = admin
        .query(&format!(
            "SELECT toUInt32(toUnixTimestamp(timestamp)), asset_id, quote_asset_id, source, \
             trade_count, version, pf_trade_count \
             FROM {db}.price_ohlcv_1m FINAL"
        ))
        .fetch_one()
        .await
        .expect("identity columns");
    assert_eq!(timestamp, MINUTE_START);
    assert_ne!(asset_id, quote_asset_id);
    assert_eq!(source, "sdex");
    assert_eq!(trade_count, 3, "the dust fill is still a trade");
    assert_eq!(version, 100_000, "ledger 100 * 1000 + operation index 0");
    // THE assertion this whole test exists for: 2, the value the writer sent —
    // not 3, which is what `pf_trade_count DEFAULT trade_count` would have
    // produced had the row struct omitted the column (task 0286 F6b).
    assert_eq!(
        pf_trade_count, 2,
        "pf_trade_count must be the written value, not the DEFAULT derived from trade_count"
    );

    // Prices, volumes and the pf aggregates.
    let (open, high, low, close, volume_base, volume_quote, vwap, pf_volume, pf_price_volume): (
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
    ) = admin
        .query(&format!(
            "SELECT toFloat64(open), toFloat64(high), toFloat64(low), toFloat64(close), \
             toFloat64(volume_base), toFloat64(volume_quote), toFloat64(vwap), \
             toFloat64(pf_volume), toFloat64(pf_price_volume) \
             FROM {db}.price_ohlcv_1m FINAL"
        ))
        .fetch_one()
        .await
        .expect("price columns");
    approx(open, 0.2, "open — the first price-forming fill");
    approx(
        close,
        0.3,
        "close — the last price-forming fill, not the dust",
    );
    approx(high, 0.3, "high");
    approx(low, 0.2, "low — the dust does not drag it down");
    approx(volume_base, 15.0000017, "volume_base counts the dust");
    approx(volume_quote, 4.0000001, "volume_quote counts the dust");
    approx(vwap, 4.0000001 / 15.0000017, "vwap is over ALL fills");
    approx(pf_volume, 15.0, "5 XLM + 10 XLM");
    approx(pf_price_volume, 4.0, "0.2 * 5 + 0.3 * 10");

    // Enrichment's columns are still the writer's zeros.
    let (volume_quote_usd, close_usd): (f64, f64) = admin
        .query(&format!(
            "SELECT toFloat64(volume_quote_usd), toFloat64(close_usd) \
             FROM {db}.price_ohlcv_1m FINAL"
        ))
        .fetch_one()
        .await
        .expect("usd columns");
    approx(volume_quote_usd, 0.0, "volume_quote_usd is enrichment's");
    approx(close_usd, 0.0, "close_usd is enrichment's");

    admin
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}
