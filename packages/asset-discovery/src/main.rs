//! Asset Discovery Lambda entrypoint (task 0054).
//!
//! EventBridge `rate(1 hour)` → this binary. Each invocation:
//!   1. ensures the seed assets exist (`ensure_seed`), then
//!   2. scans a window of recent ledgers from S3 and registers any new assets
//!      seen in trades (`discover_window`), advancing `prices.discovery_state`.
//!
//! Build the deployable with:
//!
//!     cargo lambda build -p asset-discovery --release --arm64
//!
//! Requires the `lambda` feature (default `cargo build`/`cargo test` exercise
//! the lib + seed + discovery logic without the AWS runtime / mTLS / S3 stack).

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use prices_ledger_processor::object_fetcher::S3Fetcher;
    use std::sync::Arc;

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    // Default contiguous ledgers scanned per hourly run. ~1h of mainnet ≈ 720
    // ledgers (5s close); the default scans a couple hours of headroom so a
    // missed run self-heals, bounded against the Lambda timeout.
    const DEFAULT_MAX_LEDGERS: u64 = 2000;

    // Cold start: build the mTLS client (MTLS_SECRET_NAME + CH_DOMAIN), probe
    // connectivity, build the S3 fetcher, and parse the seed once. Failures
    // here surface as a CloudWatch Init error, not a per-invocation error.
    let client = prices_clickhouse::mtls::client_from_lambda_env("prices").await?;
    let writer = Arc::new(prices_ingest_core::OhlcvWriter::new(client));
    writer.preflight().await?;

    let bucket = std::env::var("BUCKET_NAME")
        .map_err(|_| lambda_runtime::Error::from("BUCKET_NAME env var is required"))?;
    let fetcher = Arc::new(S3Fetcher::from_env(bucket).await);

    let seed = Arc::new(asset_discovery::seed_identities()?);
    let max_ledgers = prices_clickhouse::env::env_parse_or("MAX_LEDGERS", DEFAULT_MAX_LEDGERS);
    // Where to begin if `discovery_state` is empty (no prior run). Operator-set,
    // like 0038's INITIAL_CURSOR; absent → seed only, no scan (logged).
    let initial_ledger: Option<u64> = std::env::var("INITIAL_DISCOVERY_LEDGER")
        .ok()
        .and_then(|s| s.parse().ok());

    tracing::info!(
        seed = seed.len(),
        max_ledgers,
        "asset-discovery cold start ready"
    );

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let writer = writer.clone();
        let fetcher = fetcher.clone();
        let seed = seed.clone();
        async move {
            // 1. Seed (idempotent).
            let seeded = asset_discovery::ensure_seed(&writer, &seed).await?;

            // 2. Discover from `cursor + 1` (or the operator seed on first run).
            let start = match asset_discovery::load_cursor(&writer).await? {
                Some(cursor) => Some(cursor + 1),
                None => initial_ledger,
            };
            let stats = match start {
                Some(start) => Some(
                    asset_discovery::discover_window(&writer, &*fetcher, start, max_ledgers)
                        .await?,
                ),
                None => {
                    tracing::warn!(
                        "no discovery_state cursor and INITIAL_DISCOVERY_LEDGER unset — \
                         seeding only, skipping ledger scan"
                    );
                    None
                }
            };

            tracing::info!(
                seeded,
                scanned = stats.map(|s| s.ledgers_scanned).unwrap_or(0),
                to_ledger = stats.map(|s| s.to_ledger).unwrap_or(0),
                assets_total = stats.map(|s| s.assets_total).unwrap_or(seeded),
                "asset-discovery run complete"
            );
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "seeded": seeded,
                "scanned": stats.map(|s| s.ledgers_scanned).unwrap_or(0),
                "to_ledger": stats.map(|s| s.to_ledger).unwrap_or(0),
                "assets_total": stats.map(|s| s.assets_total).unwrap_or(seeded),
            }))
        }
    }))
    .await
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "asset-discovery: build with `--features lambda` (or `cargo lambda build -p \
         asset-discovery --release --arm64`) for the AWS Lambda entrypoint."
    );
}
