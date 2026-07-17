use std::process::ExitCode;

use clap::Parser;
use tracing::error;
use tracing_subscriber::EnvFilter;

use events_backfill::cli::Cli;
use events_backfill::run;

fn init_tracing(verbose: bool) {
    let default = if verbose { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!("events_backfill={default},prices_ingest_core=info"))
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match run::execute(&cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!(error = %e, "events-backfill failed");
            ExitCode::FAILURE
        }
    }
}
