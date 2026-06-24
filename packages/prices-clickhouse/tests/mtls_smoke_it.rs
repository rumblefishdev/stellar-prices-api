//! Live mTLS round-trip smoke test (task 0052 — the "live mTLS round-trip"
//! acceptance criterion).
//!
//! Proves the full `aws-mtls` transport end to end against a real
//! Caddy-fronted ClickHouse: present the client cert, let Caddy map the cert CN
//! to a CH user (ADR 0007 §3.5), and run a query. Unlike the Lambda entry point
//! (`client_from_lambda_env`, which needs the Parameters & Secrets Extension on
//! `localhost:2773`), this drives the workstation-friendly `client_with_mtls`
//! with a bundle read from local PEM files — so an operator can validate a dev
//! endpoint without a Lambda.
//!
//! Triple-gated so it never runs by accident:
//!   1. the whole file is `#![cfg(feature = "aws-mtls")]` — absent from the
//!      default test build;
//!   2. the test is `#[ignore]` — skipped by a bare `cargo test`;
//!   3. it self-skips (returns early) when the bundle env vars are unset, so a
//!      blanket `--features aws-mtls ... -- --ignored` run on a machine without
//!      a bundle skips cleanly instead of failing.
//!
//! Run against a reachable endpoint with a real bundle on disk:
//!
//! ```sh
//! CH_DOMAIN=<caddy-host> \
//! MTLS_CERT_PATH=/path/client.crt \
//! MTLS_KEY_PATH=/path/client.key \
//! MTLS_CA_PATH=/path/ca.crt \
//! cargo test -p prices-clickhouse --features aws-mtls \
//!   --test mtls_smoke_it -- --ignored --nocapture
//! ```
//!
//! The cert/key/ca paths point at the operator's per-env client bundle
//! (provisioned by task 0063; never committed). The key is read straight into
//! rustls and never logged — `MtlsBundle`'s Debug redacts all PEM, and this test
//! prints only the table count, never the material.
#![cfg(feature = "aws-mtls")]

use prices_clickhouse::mtls::{MtlsBundle, client_with_mtls};

/// Read a required env var, treating empty as unset.
fn opt_env(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

/// Assemble `(domain, bundle, database)` from the env-pointed PEM files, or
/// `None` if the round-trip env contract is not fully set (→ test self-skips).
/// `CH_DATABASE` defaults to the production `prices` layout.
fn load_target() -> Option<(String, MtlsBundle, String)> {
    let domain = opt_env("CH_DOMAIN")?;
    let cert_path = opt_env("MTLS_CERT_PATH")?;
    let key_path = opt_env("MTLS_KEY_PATH")?;
    let ca_path = opt_env("MTLS_CA_PATH")?;
    let database = opt_env("CH_DATABASE").unwrap_or_else(|| "prices".to_string());

    let read = |p: &str| {
        std::fs::read_to_string(p).unwrap_or_else(|e| panic!("failed to read PEM at `{p}`: {e}"))
    };
    let bundle = MtlsBundle {
        cert_pem: read(&cert_path),
        key_pem: read(&key_path),
        ca_pem: read(&ca_path),
    };
    Some((domain, bundle, database))
}

#[tokio::test]
#[ignore = "live mTLS round-trip — set CH_DOMAIN + MTLS_{CERT,KEY,CA}_PATH and run with --ignored"]
async fn mtls_round_trip_select_one_and_lists_tables() {
    let Some((domain, bundle, database)) = load_target() else {
        eprintln!(
            "SKIP mtls_round_trip: set CH_DOMAIN, MTLS_CERT_PATH, MTLS_KEY_PATH, MTLS_CA_PATH \
             (and optionally CH_DATABASE) to run the live round-trip"
        );
        return;
    };

    let client = client_with_mtls(&domain, &bundle, &database)
        .expect("client_with_mtls should build a TLS client from the on-disk bundle");

    // 1) Handshake + auth + query: a trivial scalar exercises the whole mTLS
    //    path — client cert presented, Caddy CN-mapped us onto a CH user, and the
    //    query executed. `SELECT 1` is a UInt8 in ClickHouse.
    let one: u8 = client
        .query("SELECT 1")
        .fetch_one::<u8>()
        .await
        .expect("SELECT 1 over mTLS should succeed (handshake + CN mapping + query)");
    assert_eq!(one, 1, "SELECT 1 returned an unexpected value");

    // 2) Identity + grants: the CN-mapped user can see the target database. Count
    //    (not an exact table list) keeps the assertion stable as the schema grows.
    let table_count: u64 = client
        .query("SELECT count() FROM system.tables WHERE database = ?")
        .bind(&database)
        .fetch_one::<u64>()
        .await
        .expect("listing tables in the target database should succeed");

    eprintln!(
        "mTLS round-trip OK against `{domain}`: SELECT 1 -> 1; `{database}` has {table_count} tables"
    );
    assert!(
        table_count > 0,
        "the `{database}` database reports zero tables — is the schema applied (task 0051)?"
    );
}
