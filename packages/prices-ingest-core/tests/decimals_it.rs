//! Task 0329 — `prices.asset_decimals` round-trips through the writer.
//!
//!     tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!     cargo test -p prices-ingest-core --test decimals_it -- --ignored --test-threads=1
//!
//! The writer hardcodes `prices.asset_decimals`, so this resets the real
//! `prices` database's table. Never run against a shared cluster.

use clickhouse::Client;
use prices_ingest_core::{AssetIdentity, AssetRegistry, DecimalsRow, OhlcvWriter};

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

const SOLVBTC: &str = "CBIJBDNZNF4X35BJ4FFZWCDBSCKOP5NB4PLG4SNENRMLAPYG4P5FM6VN";
const BNUSD: &str = "CCT4ZYIYZ3TUO2AWQFEOFGBZ6HQP3GW5TA37CK7CRZVFRDXYTHTYX7KP";

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn written_decimals_load_back_into_the_registry() {
    let c = Client::default().with_url(ch_url());
    prices_clickhouse::apply_sql(&c, prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    c.query("TRUNCATE TABLE prices.asset_decimals")
        .execute()
        .await
        .unwrap();

    let writer = OhlcvWriter::new(c.clone());
    let row = |contract: &str, decimals| DecimalsRow {
        contract_address: contract.to_string(),
        decimals,
    };
    writer
        .write_decimals(&[row(SOLVBTC, 8), row(BNUSD, 18)])
        .await
        .unwrap();
    // A re-resolve writes the same row again; FINAL collapses it.
    writer.write_decimals(&[row(SOLVBTC, 8)]).await.unwrap();
    writer.write_decimals(&[]).await.unwrap();

    let mut assets = AssetRegistry::from_existing(vec![]);
    writer.load_decimals(&mut assets).await.unwrap();
    let of = |c: &str| assets.decimals_of(&AssetIdentity::Contract(c.to_string()));
    assert_eq!(of(SOLVBTC), Some(8));
    assert_eq!(of(BNUSD), Some(18));
    assert_eq!(
        of("CC2RBGYNCFBCVENIDL5BFBWPH4OUZM2UA3OD2K2N54GLMWCC4KWPVAGO"),
        None,
        "a token with no row stays unknown"
    );

    c.query("TRUNCATE TABLE prices.asset_decimals")
        .execute()
        .await
        .unwrap();
}
