use std::path::PathBuf;

use clap::Parser;

use crate::ingest::{ExtractMode, SOROBAN_ACTIVATION_LEDGER};

/// Where the backfill writes its `prices.*` rows.
///
/// Direct-write to Hetzner (`Hetzner`) is the real-run model per ADR 0009 — no
/// local mirror, no separate push CLI; `/backfill/status` updates in real time.
/// `Local` is the plaintext Docker-CH path for tests and dry runs against a
/// stand-in.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Transport {
    /// Plaintext HTTP to a local / Docker ClickHouse (`--clickhouse-url`).
    #[default]
    Local,
    /// HTTPS + mTLS to the Hetzner `prices.*` cluster via Caddy (the task-0052
    /// client). Requires a build with `--features aws-mtls` and the
    /// `--ch-domain` / `--mtls-*-path` bundle args.
    Hetzner,
}

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

    /// Soroban activation ledger (Protocol 20, 2024-02-20). Splits the
    /// `sdex-only` pre-Soroban tail `[1, activation)` from the `combined`
    /// Soroban era `[activation, tip]`, and sanity-checks `--mode` against the
    /// requested range (warns on an obvious mismatch). Pinned value located by
    /// the BE team; see `lore/3-wiki/project/stellar-pubnet-ledger-archive.md`.
    #[arg(long, default_value_t = SOROBAN_ACTIVATION_LEDGER)]
    pub activation_ledger: u32,

    /// Chain tip — the `backfill_progress.target_ledger` denominator that makes
    /// `progress_pct` meaningful. The `sdex_archive` stream always targets the
    /// live tip; in `combined` mode `--end` *is* the tip, so this defaults to
    /// `--end`. For the `sdex-only` run over `[1, activation)` pass the current
    /// live tip explicitly so the archive's progress is measured against the
    /// whole chain, not just the pre-Soroban range.
    #[arg(long)]
    pub tip: Option<u32>,

    /// Where to write `prices.*` rows: `local` (plaintext Docker CH) or
    /// `hetzner` (direct-write over mTLS, ADR 0009). `hetzner` needs a build
    /// with `--features aws-mtls`.
    #[arg(long, value_enum, default_value_t = Transport::Local)]
    pub transport: Transport,

    /// ClickHouse HTTP URL for `--transport local` (e.g. http://localhost:8123).
    #[arg(long, env = "CLICKHOUSE_URL", default_value = "http://localhost:8123")]
    pub clickhouse_url: String,

    /// Caddy host fronting the Hetzner CH, for `--transport hetzner` (e.g.
    /// ch.sorobanscan.rumblefish.dev). The client connects to `https://{domain}`.
    #[arg(long, env = "CH_DOMAIN")]
    pub ch_domain: Option<String>,

    /// Target CH database for `--transport hetzner`.
    #[arg(long, env = "CH_DATABASE", default_value = "prices")]
    pub ch_database: String,

    /// Path to the PEM client certificate, for `--transport hetzner`.
    #[arg(long, env = "MTLS_CERT_PATH")]
    pub mtls_cert_path: Option<PathBuf>,

    /// Path to the PEM client private key, for `--transport hetzner`. Read
    /// straight into rustls; never logged.
    #[arg(long, env = "MTLS_KEY_PATH")]
    pub mtls_key_path: Option<PathBuf>,

    /// Path to the PEM CA bundle (signer of the client cert), for
    /// `--transport hetzner`.
    #[arg(long, env = "MTLS_CA_PATH")]
    pub mtls_ca_path: Option<PathBuf>,

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
