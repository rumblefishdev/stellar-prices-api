//! Thin binary shim over the `sdex_backfill` library. All engine logic lives in
//! the library modules (see `lib.rs`); this file only parses the CLI, builds the
//! selected sink, and runs the engine.

use clap::Parser;

use sdex_backfill::cli::{Cli, Transport};
use sdex_backfill::error::BackfillError;
use sdex_backfill::sink::Sink;
use sdex_backfill::{obs, run};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    obs::init(cli.verbose);

    let sink = match build_sink(&cli) {
        Ok(sink) => sink,
        Err(err) => {
            eprintln!("fatal: {err}");
            std::process::exit(1);
        }
    };

    // The sdex_archive stream targets the live tip; --end is the tip in combined
    // mode, so default there. For the sdex-only tail the operator passes --tip.
    let tip = cli.tip.unwrap_or(cli.end);

    if let Err(err) = run::execute(
        &sink,
        &cli.temp_dir,
        cli.start,
        cli.end,
        cli.keep_partitions,
        cli.mode,
        cli.activation_ledger,
        tip,
    )
    .await
    {
        eprintln!("fatal: {err}");
        std::process::exit(1);
    }
}

/// Build the sink for the selected transport. `local` is always available;
/// `hetzner` (ADR 0009 direct-write) needs the `--ch-domain` / `--mtls-*-path`
/// bundle args and a build with `--features aws-mtls`.
fn build_sink(cli: &Cli) -> Result<Sink, BackfillError> {
    match cli.transport {
        Transport::Local => Ok(Sink::new(&cli.clickhouse_url)),
        Transport::Hetzner => build_hetzner_sink(cli),
    }
}

#[cfg(feature = "aws-mtls")]
fn build_hetzner_sink(cli: &Cli) -> Result<Sink, BackfillError> {
    // Collect every missing arg so the operator fixes them in one pass rather
    // than one round-trip per unset value.
    let mut missing = Vec::new();
    if cli.ch_domain.is_none() {
        missing.push("--ch-domain / CH_DOMAIN");
    }
    if cli.mtls_cert_path.is_none() {
        missing.push("--mtls-cert-path / MTLS_CERT_PATH");
    }
    if cli.mtls_key_path.is_none() {
        missing.push("--mtls-key-path / MTLS_KEY_PATH");
    }
    if cli.mtls_ca_path.is_none() {
        missing.push("--mtls-ca-path / MTLS_CA_PATH");
    }
    if !missing.is_empty() {
        return Err(BackfillError::MissingMtlsArg(missing.join(", ")));
    }

    Sink::mtls(
        cli.ch_domain.as_deref().unwrap(),
        cli.mtls_cert_path.as_deref().unwrap(),
        cli.mtls_key_path.as_deref().unwrap(),
        cli.mtls_ca_path.as_deref().unwrap(),
        &cli.ch_database,
    )
}

#[cfg(not(feature = "aws-mtls"))]
fn build_hetzner_sink(_cli: &Cli) -> Result<Sink, BackfillError> {
    Err(BackfillError::Mtls(
        "this binary was built without the `aws-mtls` feature — rebuild with \
         `cargo build -p sdex-backfill --features aws-mtls` to use `--transport hetzner`"
            .to_string(),
    ))
}
