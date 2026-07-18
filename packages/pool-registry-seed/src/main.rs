//! One-off CLI: seed `prices.pool_registry` from the Soroswap `/pools` API
//! (task 0079). Fetches every AMM venue, maps + normalises, and writes the
//! registry via the shared `OhlcvWriter::write_pool_registry` (idempotent RMT).
//!
//!     # dry run — fetch + map, print what would be written, no ClickHouse:
//!     SOROSWAP_API_KEY=sk_… cargo run -p pool-registry-seed -- --dry-run
//!
//!     # local Docker CH:
//!     SOROSWAP_API_KEY=sk_… cargo run -p pool-registry-seed -- --ch-url http://localhost:8123
//!
//!     # Hetzner prod over mTLS:
//!     SOROSWAP_API_KEY=sk_… cargo run -p pool-registry-seed -- \
//!       --ch-domain ch.example --mtls-cert-path cert.pem \
//!       --mtls-key-path key.pem --mtls-ca-path ca.pem
//!
//! The API key is read from `SOROSWAP_API_KEY` (never a CLI flag value in shell
//! history) and never logged.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::Parser;
use pool_registry_seed::{SeedError, build_registry, fetch_all_venues};
use prices_ingest_core::OhlcvWriter;

#[derive(Parser, Debug)]
#[command(about = "Seed prices.pool_registry from the Soroswap /pools API (task 0079)")]
struct Cli {
    /// Soroswap API base URL.
    #[arg(long, default_value = "https://api.soroswap.finance")]
    base_url: String,
    /// Network to seed (the API accepts testnet | mainnet).
    #[arg(long, default_value = "mainnet")]
    network: String,
    /// Fetch + map + report only; do not connect to or write ClickHouse.
    #[arg(long)]
    dry_run: bool,
    /// ClickHouse database (writes are fully qualified `prices.*` regardless).
    #[arg(long, default_value = "prices")]
    database: String,

    // --- transport: plaintext local (default) OR Hetzner mTLS (--ch-domain) ---
    /// Plaintext CH URL (local/dev). Used unless `--ch-domain` is set.
    #[arg(long, default_value = "http://localhost:8123")]
    ch_url: String,
    /// Hetzner CH domain — when set, writes over mTLS using the bundle paths below.
    #[arg(long)]
    ch_domain: Option<String>,
    #[arg(long, env = "MTLS_CERT_PATH")]
    mtls_cert_path: Option<PathBuf>,
    #[arg(long, env = "MTLS_KEY_PATH")]
    mtls_key_path: Option<PathBuf>,
    #[arg(long, env = "MTLS_CA_PATH")]
    mtls_ca_path: Option<PathBuf>,
}

fn build_writer(cli: &Cli) -> Result<OhlcvWriter, SeedError> {
    let Some(domain) = &cli.ch_domain else {
        return Ok(OhlcvWriter::plaintext(&cli.ch_url));
    };
    // Require the three bundle paths (nice per-flag error), then delegate the
    // PEM-read + client build to the shared helper (same path sdex-backfill uses).
    fn require<'a>(label: &str, path: &'a Option<PathBuf>) -> Result<&'a Path, SeedError> {
        path.as_deref()
            .ok_or_else(|| SeedError::Config(format!("--ch-domain set but {label} is missing")))
    }
    let client = prices_clickhouse::mtls::client_with_mtls_from_paths(
        domain,
        require("--mtls-cert-path / MTLS_CERT_PATH", &cli.mtls_cert_path)?,
        require("--mtls-key-path / MTLS_KEY_PATH", &cli.mtls_key_path)?,
        require("--mtls-ca-path / MTLS_CA_PATH", &cli.mtls_ca_path)?,
        &cli.database,
    )
    .map_err(|e| SeedError::Mtls(e.to_string()))?;
    Ok(OhlcvWriter::new(client))
}

#[tokio::main]
async fn main() -> Result<(), SeedError> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cli = Cli::parse();

    // Env-only credential — deliberately NOT a CLI flag (would land in shell
    // history / `ps`) and kept out of any Debug-printable struct.
    let api_key = std::env::var("SOROSWAP_API_KEY")
        .map_err(|_| SeedError::Config("SOROSWAP_API_KEY env var is required".into()))?;

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("stellar-prices-pool-registry-seed")
        .build()?;
    let (rows, stats) = fetch_all_venues(&http, &cli.base_url, &api_key, &cli.network).await?;
    if rows.is_empty() {
        return Err(SeedError::Config(
            "fetched 0 seedable pools — check --network and the SOROSWAP_API_KEY credential".into(),
        ));
    }
    tracing::info!(
        kept = stats.kept,
        dropped_venue = stats.dropped_venue,
        dropped_pool_type = stats.dropped_pool_type,
        "mapped Soroswap API pools → registry rows"
    );
    let reg = build_registry(&rows);
    let out = reg.to_pool_rows();

    // Per-venue summary (what actually lands in the table).
    let mut by_venue: BTreeMap<String, usize> = BTreeMap::new();
    for r in &out {
        *by_venue.entry(r.venue.clone()).or_insert(0) += 1;
    }

    if cli.dry_run {
        println!("DRY RUN — would write {} pool_registry rows:", out.len());
        for (venue, count) in &by_venue {
            println!("  {venue}: {count}");
        }
        if stats.dropped_pool_type > 0 {
            println!(
                "  ({} pool(s) skipped for unknown poolType)",
                stats.dropped_pool_type
            );
        }
        return Ok(());
    }

    let writer = build_writer(&cli)?;
    writer.preflight().await?;
    writer.write_pool_registry(&reg).await?;
    tracing::info!(rows = out.len(), ?by_venue, "seeded prices.pool_registry");
    println!(
        "Seeded {} rows into prices.pool_registry: {by_venue:?}",
        out.len()
    );
    Ok(())
}
