//! Local fixture runner for the Prices Ledger Processor (task 0038).
//!
//! Drives the *same* reconcile loop the Lambda runs, but against local-disk
//! fixtures and a local (plaintext) ClickHouse — no AWS, no mTLS. Use it to
//! exercise the full decode → extract → bucket → write pipeline end-to-end:
//!
//! ```bash
//! # write into local Docker ClickHouse (apply prices schema first)
//! CLICKHOUSE_URL=http://localhost:8123 cargo run -p prices-ledger-processor \
//!     --bin prices-cli -- --cursor 62460539 --max-iterations 16
//!
//! # parse + bucket only, no DB writes
//! cargo run -p prices-ledger-processor --bin prices-cli -- \
//!     --cursor 62460539 --dry-run
//! ```

use std::path::PathBuf;

use clap::Parser;
use prices_ingest_core::{AssetRegistry, Registries};
use prices_ledger_processor::{
    cursor::{Cursor, StubFileCursor},
    object_fetcher::LocalDiskFetcher,
    reconcile::{Reconciler, RunStats},
    sink::{ClickHouseSink, CountingSink},
};
use tracing::info;

#[derive(Parser, Debug)]
#[command(
    name = "prices-cli",
    about = "Local fixture runner for the Prices Ledger Processor (task 0038)"
)]
struct Args {
    /// Initial cursor (the run starts at this ledger + 1). Overwrites the
    /// cursor file before the run.
    #[arg(long)]
    cursor: u64,

    /// Maximum reconcile iterations (contiguous ledgers) per run.
    #[arg(long, default_value_t = 16)]
    max_iterations: usize,

    /// Local fixture root — derived Galexie keys are joined onto this.
    #[arg(long, default_value = "fixtures/ledgers")]
    fixtures_dir: PathBuf,

    /// Where the cursor file lives.
    #[arg(long, default_value = "out/cursor.txt")]
    cursor_file: PathBuf,

    /// Local ClickHouse endpoint (plaintext). Ignored with --dry-run.
    #[arg(long, env = "CLICKHOUSE_URL", default_value = "http://localhost:8123")]
    clickhouse_url: String,

    /// Parse + bucket only; do not write to ClickHouse (counts rows).
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let args = Args::parse();

    let cursor = StubFileCursor::new(&args.cursor_file);
    cursor.write(args.cursor).await?;
    let fetcher = LocalDiskFetcher::new(&args.fixtures_dir);

    let stats: RunStats = if args.dry_run {
        let reconciler = Reconciler::new(
            fetcher,
            cursor,
            CountingSink::default(),
            AssetRegistry::from_existing(Vec::new()),
            Registries::new(),
        );
        reconciler.run(args.max_iterations).await?
    } else {
        let sink = ClickHouseSink::plaintext(&args.clickhouse_url);
        sink.preflight().await?;
        let registry = sink.load_registry().await?;
        let pool_registry = sink.load_pool_registry().await?;
        let reconciler = Reconciler::new(fetcher, cursor, sink, registry, pool_registry);
        reconciler.run(args.max_iterations).await?
    };

    info!(
        start = stats.start_cursor,
        end = stats.end_cursor,
        persisted = stats.ledgers_persisted,
        rows = stats.rows_emitted,
        dry_run = args.dry_run,
        "reconcile complete"
    );

    Ok(())
}
