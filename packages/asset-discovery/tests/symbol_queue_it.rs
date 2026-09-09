//! The `asset_symbol` retry queue (task 0210), against a local Docker
//! ClickHouse with the `prices` schema applied.
//!
//!     docker compose up -d clickhouse
//!     cargo test -p asset-discovery --test symbol_queue_it -- --ignored
//!
//! Uses the real `prices` database, so it is destructive to those local tables —
//! fine for the ephemeral Docker instance, never against a shared cluster.

use asset_discovery::symbols::{MAX_SYMBOL_ATTEMPTS, load_unresolved_contracts};
use prices_ingest_core::OhlcvWriter;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

const CONTRACT: &str = "CBIJTEST000000000000000000000000000000000000000000000QUEUE";

/// Fresh `assets` + `asset_symbol` holding exactly one Soroban contract.
///
/// Drops `asset_symbol` rather than truncating it: the DDL is a
/// `CREATE TABLE IF NOT EXISTS`, so a local table left over from before the
/// `attempts` column existed would survive the schema apply and fail with
/// NO_SUCH_COLUMN. Prod has no such table yet, which is why the plain `CREATE`
/// is still the right shape there.
async fn setup(client: &clickhouse::Client) {
    client
        .query("DROP TABLE IF EXISTS prices.asset_symbol")
        .execute()
        .await
        .expect("drop asset_symbol");
    prices_clickhouse::apply_sql(client, prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    client
        .query("TRUNCATE TABLE IF EXISTS prices.assets")
        .execute()
        .await
        .expect("truncate assets");
    client
        .query(
            "INSERT INTO prices.assets (asset_id, asset_code, issuer_address, contract_address, \
             asset_type) VALUES (1, '', '', ?, 'soroban')",
        )
        .bind(CONTRACT)
        .execute()
        .await
        .expect("seed a soroban asset");
}

async fn attempts_for(client: &clickhouse::Client, contract: &str) -> Option<u8> {
    load_unresolved_contracts(client, 100)
        .await
        .expect("queue reads")
        .into_iter()
        .find(|(c, _)| c == contract)
        .map(|(_, a)| a)
}

/// A negative answer must not remove a contract from the queue until it has
/// been given [`MAX_SYMBOL_ATTEMPTS`] times. This is the whole point of the
/// counter: the simulation error that produces it cannot distinguish a contract
/// with no `symbol()` from a node that failed to read a ledger entry, so one
/// negative answer is not allowed to publish a permanent empty symbol.
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn a_negative_answer_is_retried_until_the_count_is_exhausted() {
    let writer = OhlcvWriter::plaintext(&ch_url());
    let client = writer.client();
    setup(client).await;

    // No row yet: queued, nothing counted against it.
    assert_eq!(attempts_for(client, CONTRACT).await, Some(0), "unseen");

    // Each negative answer short of the cap keeps it queued, carrying the count
    // forward so the next run can increment it rather than restarting at zero.
    for n in 1..MAX_SYMBOL_ATTEMPTS {
        client
            .query("INSERT INTO prices.asset_symbol (contract_address, symbol, attempts) VALUES (?, '', ?)")
            .bind(CONTRACT)
            .bind(n)
            .execute()
            .await
            .expect("record a negative answer");
        assert_eq!(
            attempts_for(client, CONTRACT).await,
            Some(n),
            "still retryable after {n} negative answer(s)"
        );
    }

    // At the cap it leaves for good.
    client
        .query("INSERT INTO prices.asset_symbol (contract_address, symbol, attempts) VALUES (?, '', ?)")
        .bind(CONTRACT)
        .bind(MAX_SYMBOL_ATTEMPTS)
        .execute()
        .await
        .expect("record the exhausting answer");
    assert_eq!(
        attempts_for(client, CONTRACT).await,
        None,
        "sentinelled at {MAX_SYMBOL_ATTEMPTS} attempts"
    );
}

/// A resolved symbol leaves the queue whatever the count says — and because the
/// writer resets `attempts` to 0 on success, a stale high count must not keep
/// re-queueing a contract that has since answered.
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn a_resolved_symbol_leaves_the_queue_even_after_failures() {
    let writer = OhlcvWriter::plaintext(&ch_url());
    let client = writer.client();
    setup(client).await;

    client
        .query("INSERT INTO prices.asset_symbol (contract_address, symbol, attempts) VALUES (?, '', 2)")
        .bind(CONTRACT)
        .execute()
        .await
        .expect("two prior failures");
    assert_eq!(
        attempts_for(client, CONTRACT).await,
        Some(2),
        "still queued"
    );

    client
        .query("INSERT INTO prices.asset_symbol (contract_address, symbol, attempts) VALUES (?, 'SolvBTC', 0)")
        .bind(CONTRACT)
        .execute()
        .await
        .expect("later success");
    assert_eq!(attempts_for(client, CONTRACT).await, None, "resolved");
}
