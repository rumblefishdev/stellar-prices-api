use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use extractors_core::VenueRegistry;
use phoenix_extractor::PhoenixPoolRegistry;
use prices_ledger_processor::{
    cursor::{Cursor, StubFileCursor},
    decode::XdrLedgerDecoder,
    object_fetcher::LocalDiskFetcher,
    reconcile::Reconciler,
    sink::{SqlFileSink, StdoutJsonSink},
};
use tracing::info;

#[derive(Parser, Debug)]
#[command(
    name = "prices-cli",
    about = "Local CLI driver for the Prices Ledger Processor prototype (task 0038)"
)]
struct Args {
    /// Initial cursor value (ledger sequence the run starts AFTER).
    /// Always overwrites the cursor file before the run.
    #[arg(long)]
    cursor: u64,

    /// Maximum reconcile iterations per invocation.
    #[arg(long, default_value_t = 16)]
    max_iterations: usize,

    /// Sink selection.
    #[arg(long, value_enum, default_value_t = SinkKind::Stdout)]
    sink: SinkKind,

    /// Local fixture root — keys derived by `ledger_s3_key` are joined onto this.
    #[arg(long, default_value = "fixtures/ledgers")]
    fixtures_dir: PathBuf,

    /// Where the cursor file lives.
    #[arg(long, default_value = "out/cursor.txt")]
    cursor_file: PathBuf,

    /// Where SQL-file sink output lands.
    #[arg(long, default_value = "out")]
    out_dir: PathBuf,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum SinkKind {
    Stdout,
    SqlFile,
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

    let stats = match args.sink {
        SinkKind::Stdout => {
            let reconciler = Reconciler {
                fetcher,
                cursor,
                sink: StdoutJsonSink,
                decoder: XdrLedgerDecoder,
                venue_registry: VenueRegistry::new(),
                phoenix_registry: PhoenixPoolRegistry::default(),
            };
            reconciler.run(args.max_iterations).await?
        }
        SinkKind::SqlFile => {
            let reconciler = Reconciler {
                fetcher,
                cursor,
                sink: SqlFileSink::new(&args.out_dir),
                decoder: XdrLedgerDecoder,
                venue_registry: VenueRegistry::new(),
                phoenix_registry: PhoenixPoolRegistry::default(),
            };
            reconciler.run(args.max_iterations).await?
        }
    };

    info!(
        start = stats.start_cursor,
        end = stats.end_cursor,
        persisted = stats.ledgers_persisted,
        rows = stats.rows_emitted,
        "reconcile complete"
    );

    Ok(())
}
