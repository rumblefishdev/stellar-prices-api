//! CLI driver for the enrichment-worker prototype (task 0026).

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use enrichment_worker::candidates::JsonlCandidateSource;
use enrichment_worker::oracle::InMemoryOracleLookup;
use enrichment_worker::pass::run_pass;
use enrichment_worker::sink::{SqlFileSink, StdoutJsonSink};
use tracing::info;

#[derive(Parser, Debug)]
#[command(
    name = "enrichment-cli",
    about = "Local CLI driver for the volume_quote_usd enrichment prototype (task 0026)"
)]
struct Args {
    /// JSONL fixture of candidate price_ohlcv rows.
    #[arg(long, default_value = "fixtures/candidates.jsonl")]
    candidates: PathBuf,

    /// JSONL fixture of oracle_prices entries.
    #[arg(long, default_value = "fixtures/oracle_prices.jsonl")]
    oracle: PathBuf,

    /// Oracle source name to use (matches `oracle_prices.oracle_name`).
    #[arg(long, default_value = "reflector")]
    oracle_name: String,

    /// Forward-fill window in seconds — max staleness of an oracle bar.
    #[arg(long, default_value_t = 300)]
    window_s: u32,

    /// Candidates read per inner-loop batch.
    #[arg(long, default_value_t = 10_000)]
    batch_size: usize,

    /// Cap on inner-loop iterations per invocation.
    #[arg(long, default_value_t = 20)]
    max_batches: usize,

    /// Sink selection.
    #[arg(long, value_enum, default_value_t = SinkKind::Stdout)]
    sink: SinkKind,

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

    let oracle = InMemoryOracleLookup::load_jsonl(&args.oracle).await?;
    let mut candidates = JsonlCandidateSource::open(&args.candidates).await?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let stats = match args.sink {
        SinkKind::Stdout => {
            run_pass(
                &mut candidates,
                &oracle,
                &StdoutJsonSink,
                &args.oracle_name,
                args.window_s,
                args.batch_size,
                args.max_batches,
                now,
            )
            .await?
        }
        SinkKind::SqlFile => {
            run_pass(
                &mut candidates,
                &oracle,
                &SqlFileSink::new(&args.out_dir),
                &args.oracle_name,
                args.window_s,
                args.batch_size,
                args.max_batches,
                now,
            )
            .await?
        }
    };

    info!(
        batches = stats.batches,
        candidates_seen = stats.candidates_seen,
        enriched = stats.rows_enriched,
        misses = stats.oracle_misses,
        "enrichment pass complete"
    );

    Ok(())
}
