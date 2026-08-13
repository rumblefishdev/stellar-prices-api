//! Operator entrypoint for the coarse-table USD repair driver (task 0114).
//!
//! Re-runs the enrichment tiers directly against a coarse OHLCV table
//! (`price_ohlcv_1h` … `_1M`), one monthly partition at a time, to fill the
//! `close_usd` / `volume_quote_usd` the rollup path froze at zero. Partition-
//! bounded (task 0111 option 1) and additive-only — it FREEZE-snapshots each
//! partition and never truncates, because for 2025-02 → 2026-02 the 1m source is
//! gone and the coarse tables are the sole surviving copy.
//!
//! Requires `--features aws-mtls` for `--transport hetzner` (the prod path). The
//! `local` transport (plain HTTP) is for testing against a local ClickHouse.
//!
//!   # PREVIEW first (writes nothing):
//!   cargo run -p enrichment-worker --features aws-mtls --bin coarse-repair -- \
//!     --transport hetzner --table price_ohlcv_1h \
//!     --start-month 202402 --end-month 202602 --dry-run
//!
//!   # then the real run (per-partition FREEZE snapshot is ON by default):
//!   cargo run -p enrichment-worker --features aws-mtls --bin coarse-repair -- \
//!     --transport hetzner --table price_ohlcv_1h \
//!     --start-month 202402 --end-month 202602
//!
//! mTLS env (same as sdex-backfill): CH_DOMAIN, MTLS_CERT_PATH, MTLS_KEY_PATH,
//! MTLS_CA_PATH. Revert a partition from its FREEZE snapshot with
//! `SYSTEM UNFREEZE WITH NAME '<snapshot_name>'` + `ALTER TABLE … ATTACH`.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use enrichment_worker::ch_enrich::{ChEnrichConfig, UsdResetSpec};
use enrichment_worker::repair::{CoarseRepairConfig, CoarseRepairDriver};
use tracing::{info, warn};

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Transport {
    /// Plain HTTP to a local ClickHouse (testing).
    Local,
    /// Direct mTLS to the Hetzner prod CH (needs `--features aws-mtls`).
    Hetzner,
}

#[derive(Parser, Debug)]
#[command(
    name = "coarse-repair",
    about = "Partition-bounded USD repair over the coarse OHLCV tables (task 0114)"
)]
struct Args {
    /// `local` (plain HTTP) or `hetzner` (mTLS direct-write).
    #[arg(long, value_enum, default_value_t = Transport::Local)]
    transport: Transport,

    /// ClickHouse HTTP URL for `--transport local`.
    #[arg(long, env = "CLICKHOUSE_URL", default_value = "http://localhost:8123")]
    clickhouse_url: String,

    /// Caddy host fronting Hetzner CH, for `--transport hetzner`.
    #[arg(long, env = "CH_DOMAIN")]
    ch_domain: Option<String>,

    /// Target CH database.
    #[arg(long, env = "CH_DATABASE", default_value = "prices")]
    database: String,

    #[arg(long, env = "MTLS_CERT_PATH")]
    mtls_cert_path: Option<PathBuf>,
    #[arg(long, env = "MTLS_KEY_PATH")]
    mtls_key_path: Option<PathBuf>,
    #[arg(long, env = "MTLS_CA_PATH")]
    mtls_ca_path: Option<PathBuf>,

    /// Coarse table to repair (e.g. `price_ohlcv_1h`). One table per run.
    #[arg(long)]
    table: String,

    /// First month to repair, inclusive, as `YYYYMM` (e.g. 202402).
    #[arg(long)]
    start_month: u32,

    /// Last month to repair, inclusive, as `YYYYMM` (e.g. 202602).
    #[arg(long)]
    end_month: u32,

    /// Preview: enumerate months-with-zeros and their counts, write nothing.
    #[arg(long)]
    dry_run: bool,

    /// DANGEROUS: skip the per-partition FREEZE snapshot. Only for a span whose
    /// 1m source still exists and can rebuild the coarse table. For 2025-02 →
    /// 2026-02 the coarse tables are the sole copy — never skip there.
    #[arg(long)]
    skip_snapshot: bool,

    /// Candidate rows per enrichment batch.
    #[arg(long, env = "BATCH_SIZE", default_value_t = 10_000)]
    batch_size: u64,

    /// Oracle source name (matches `oracle_prices.oracle_name`).
    #[arg(long, env = "ORACLE_NAME", default_value = "reflector")]
    oracle_name: String,

    /// Oracle forward-fill staleness window (seconds).
    #[arg(long, env = "FORWARD_FILL_WINDOW_S", default_value_t = 300)]
    window_s: u32,

    /// XLM/USDC pivot staleness window (seconds).
    ///
    /// ⚠️ Raise this for the coarse tiers. The default is one day, but a `_1w`
    /// or `_1M` candle whose reference bucket is the previous week/month sits
    /// further back than that, and the pivot's `ASOF` drops anything staler —
    /// silently, as `zeros_after`.
    #[arg(long, env = "PIVOT_WINDOW_S", default_value_t = 86_400)]
    pivot_window_s: u32,

    /// Re-open already-written USD values for this quote `asset_id` so the
    /// corrected tiers recompute them (task 0182). Requires
    /// `--reset-not-before`.
    ///
    /// ⚠️ This is the only flag here that DISCARDS a computed value. Everything
    /// else in this tool is purely additive — it fills zeros. Use it only to
    /// correct a *pricing* defect, where the stored number is wrong rather than
    /// missing, and only against a FREEZE-snapshotted partition.
    #[arg(long, requires = "reset_not_before")]
    reset_quote_asset_id: Option<u32>,

    /// Epoch (unix seconds) below which stored USD values are left alone.
    ///
    /// This is a correctness bound, not a convenience: below the date the pivot's
    /// reference market begins, there is nothing to recompute from, so a reset
    /// row stays at `close_usd = 0` permanently. For canonical USDT that date is
    /// **2021-02-07** (`1612656000`) — the start of its USDC market. Task 0172
    /// also measured it at genuine par before the June 2022 depeg, so the value
    /// already on disk for that window is *correct* and this flag protects it.
    #[arg(long, requires = "reset_quote_asset_id")]
    reset_not_before: Option<u32>,
}

fn validate_month(label: &str, m: u32) -> Result<(), String> {
    let (y, mm) = (m / 100, m % 100);
    if !(2015..=2100).contains(&y) || !(1..=12).contains(&mm) {
        return Err(format!("{label} = {m} is not a valid YYYYMM"));
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    let args = Args::parse();

    validate_month("--start-month", args.start_month)?;
    validate_month("--end-month", args.end_month)?;
    if args.start_month > args.end_month {
        return Err(format!(
            "--start-month {} is after --end-month {}",
            args.start_month, args.end_month
        )
        .into());
    }
    if !args.table.starts_with("price_ohlcv_") || args.table == "price_ohlcv_1m" {
        // The driver targets the coarse rollups; 1m is the live base table the
        // scheduled Lambda owns and has live writers. Refuse it outright.
        return Err(format!(
            "--table {} is not a coarse OHLCV table (expected price_ohlcv_15m … _1M; \
             price_ohlcv_1m is the live base table and is off-limits here)",
            args.table
        )
        .into());
    }

    let enrich = ChEnrichConfig {
        url: args.clickhouse_url.clone(),
        database: args.database.clone(),
        table: args.table.clone(),
        oracle_name: args.oracle_name.clone(),
        window_s: args.window_s,
        pivot_window_s: args.pivot_window_s,
        // Irrelevant to the repair (recency metric only), keep the default.
        recent_window_s: 14_400,
        batch_size: args.batch_size,
        // one_shot + time_window are set per-month by the driver.
        max_batches: 20,
        one_shot: true,
        time_window: None,
        usd_reset: match (args.reset_quote_asset_id, args.reset_not_before) {
            (Some(quote_asset_id), Some(not_before)) => Some(UsdResetSpec {
                quote_asset_id,
                not_before,
            }),
            // clap's `requires` makes the mixed cases unreachable.
            _ => None,
        },
    };

    let repair_cfg = CoarseRepairConfig {
        enrich,
        start_month: args.start_month,
        end_month: args.end_month,
        snapshot: !args.skip_snapshot,
        dry_run: args.dry_run,
        // Manual historical repair: drain each month to the no_reference floor in
        // one pass. (The recurring hourly sweep sets this false — bounded.)
        one_shot: true,
        // No wall-clock budget — the operator run must drain in full, not defer.
        deadline: None,
    };

    // Build the CH client per transport, then hand it to the driver.
    let driver = match args.transport {
        Transport::Local => {
            let client = clickhouse::Client::default()
                .with_url(&args.clickhouse_url)
                .with_database(&args.database);
            CoarseRepairDriver::with_client(client, repair_cfg)
        }
        Transport::Hetzner => {
            let domain = args
                .ch_domain
                .as_deref()
                .ok_or("--transport hetzner requires --ch-domain / CH_DOMAIN")?;
            let cert = args
                .mtls_cert_path
                .as_deref()
                .ok_or("--transport hetzner requires --mtls-cert-path / MTLS_CERT_PATH")?;
            let key = args
                .mtls_key_path
                .as_deref()
                .ok_or("--transport hetzner requires --mtls-key-path / MTLS_KEY_PATH")?;
            let ca = args
                .mtls_ca_path
                .as_deref()
                .ok_or("--transport hetzner requires --mtls-ca-path / MTLS_CA_PATH")?;
            let client = prices_clickhouse::mtls::client_with_mtls_from_paths(
                domain,
                cert,
                key,
                ca,
                &args.database,
            )?;
            CoarseRepairDriver::with_client(client, repair_cfg)
        }
    };

    driver.preflight().await?;

    if args.skip_snapshot && !args.dry_run {
        warn!(
            "snapshot DISABLED (--skip-snapshot): partitions will be repaired with no FREEZE \
             backup — only safe when the 1m source can rebuild this coarse table"
        );
    }

    if let Some(quote_asset_id) = args.reset_quote_asset_id
        && !args.dry_run
    {
        warn!(
            quote_asset_id,
            not_before = args.reset_not_before,
            "USD RESET ENABLED: stored close_usd/volume_quote_usd for this quote leg will be \
             DISCARDED and recomputed. Rows the pivot cannot reach stay at 0 — compare \
             rows_reset against rows_enriched in the summary, and keep the FREEZE snapshot \
             until they agree"
        );
    }

    info!(
        transport = ?args.transport,
        database = %args.database,
        table = %args.table,
        start_month = args.start_month,
        end_month = args.end_month,
        dry_run = args.dry_run,
        snapshot = !args.skip_snapshot,
        "coarse-repair starting"
    );

    let summary = driver.run().await?;

    // Human-readable roll-up; the structured per-month lines are already logged.
    println!("\n=== coarse-repair summary ({}) ===", args.table);
    println!(
        "{:>8}  {:>12}  {:>12}  {:>12}  snapshot",
        "month", "zeros_before", "enriched", "zeros_after"
    );
    for m in &summary.months {
        println!(
            "{:>8}  {:>12}  {:>12}  {:>12}  {}",
            m.month,
            m.zeros_before,
            m.rows_enriched,
            m.zeros_after,
            m.snapshot_name.as_deref().unwrap_or("-")
        );
    }
    println!(
        "{} month(s): {} enriched, {} left at the no_reference floor{}",
        summary.months.len(),
        summary.total_enriched(),
        summary.total_remaining(),
        if args.dry_run {
            " [DRY RUN — nothing written]"
        } else {
            ""
        }
    );

    Ok(())
}
