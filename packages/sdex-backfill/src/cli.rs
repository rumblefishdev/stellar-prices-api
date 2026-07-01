use std::path::PathBuf;

use clap::Parser;

use crate::ingest::ExtractMode;

#[derive(Parser, Debug)]
#[command(name = "sdex-backfill", version)]
pub struct Cli {
    /// First ledger to index (inclusive).
    #[arg(long)]
    pub start: u32,

    /// Last ledger to index (inclusive).
    #[arg(long)]
    pub end: u32,

    /// What to extract per ledger. `combined` = SDEX trades + Soroban AMM
    /// swaps + oracle samples (for the Soroban era, [activation, tip]);
    /// `sdex-only` = classic SDEX trades only (for the pre-Soroban tail,
    /// [1, activation)).
    #[arg(long, value_enum, default_value_t = ExtractMode::Combined)]
    pub mode: ExtractMode,

    /// Soroban activation ledger — used only to sanity-check `--mode` against
    /// the requested range (warns on an obvious mismatch).
    #[arg(long, default_value_t = 48_500_000)]
    pub activation_ledger: u32,

    /// ClickHouse HTTP URL (e.g. http://localhost:8123).
    #[arg(long, env = "CLICKHOUSE_URL", default_value = "http://localhost:8123")]
    pub clickhouse_url: String,

    /// Local scratch directory for downloaded partitions.
    #[arg(long, env = "BACKFILL_TEMP_DIR", default_value = ".temp/sdex-backfill")]
    pub temp_dir: PathBuf,

    /// Keep partition folders after indexing (for debugging).
    #[arg(long)]
    pub keep_partitions: bool,

    /// Enable per-ledger and per-partition progress logs.
    #[arg(long, short)]
    pub verbose: bool,
}
