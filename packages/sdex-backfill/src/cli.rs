use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "sdex-backfill", version)]
pub struct Cli {
    /// First ledger to index (inclusive).
    #[arg(long)]
    pub start: u32,

    /// Last ledger to index (inclusive).
    #[arg(long)]
    pub end: u32,

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
