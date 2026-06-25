mod cli;
mod error;
mod ingest;
mod obs;
mod partition;
mod run;
mod sink;
mod sync;

use clap::Parser;

use cli::Cli;
use sink::Sink;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    obs::init(cli.verbose);

    let sink = Sink::new(&cli.clickhouse_url);

    if let Err(err) = run::execute(
        &sink,
        &cli.temp_dir,
        cli.start,
        cli.end,
        cli.keep_partitions,
    )
    .await
    {
        eprintln!("fatal: {err}");
        std::process::exit(1);
    }
}
