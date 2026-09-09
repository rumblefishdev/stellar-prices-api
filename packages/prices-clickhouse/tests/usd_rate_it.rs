//! Live-ClickHouse integration test for `prices.usd_rate` (task 0167).
//!
//!   docker compose up -d clickhouse
//!   cargo test -p prices-clickhouse --test usd_rate_it -- --ignored

use clickhouse::Client;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

async fn setup(db: &str) -> Client {
    let client = Client::default().with_url(ch_url());
    client
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!("CREATE DATABASE {db}"))
        .execute()
        .await
        .unwrap();
    prices_clickhouse::apply_sql(&client, &rewrite(prices_clickhouse::INIT_SQL, db))
        .await
        .unwrap();
    client
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn usd_rate_has_the_0154_shape_keyed_on_natural_identity() {
    let db = "it_usd_rate_shape";
    let client = setup(db).await;

    let cols: Vec<(String, String)> = client
        .query(
            "SELECT name, type FROM system.columns \
             WHERE database = ? AND table = 'usd_rate' ORDER BY position",
        )
        .bind(db)
        .fetch_all::<(String, String)>()
        .await
        .unwrap();
    let names: Vec<&str> = cols.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "asset_kind",
            "asset_code",
            "issuer_address",
            "contract_address",
            "timestamp",
            "usd_rate",
            "method",
            "reference_asset",
            "hops",
            "version",
        ],
        "0154's exact column set, in order"
    );

    // ⚠️ The key must be natural identity, NOT asset_id — task 0139 is confirmed
    // asset_id collisions, so an asset_id key would be non-unique by construction.
    let (engine, sorting): (String, String) = client
        .query(
            "SELECT engine_full, sorting_key FROM system.tables \
             WHERE database = ? AND name = 'usd_rate'",
        )
        .bind(db)
        .fetch_one::<(String, String)>()
        .await
        .unwrap();
    assert!(
        engine.contains("ReplacingMergeTree(version)"),
        "must dedupe on version, got {engine}"
    );
    assert!(
        !sorting.contains("asset_id"),
        "usd_rate must NOT be keyed on asset_id (task 0139), got {sorting}"
    );
    for col in [
        "asset_kind",
        "asset_code",
        "issuer_address",
        "contract_address",
        "timestamp",
    ] {
        assert!(
            sorting.contains(col),
            "sorting key missing {col}: {sorting}"
        );
    }

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}
