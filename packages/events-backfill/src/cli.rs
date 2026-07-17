use clap::Parser;

/// Events-sourced AMM backfill: reprice historical AMM candles by reading BE's
/// ClickHouse `default.soroban_events` and running the events through the shared
/// live extraction pipeline — a CH-to-CH reprice, no ledger archive re-download
/// (task 0097).
#[derive(Parser, Debug)]
#[command(name = "events-backfill", version, about)]
pub struct Cli {
    /// First ledger to reprice (inclusive).
    #[arg(long)]
    pub start: u32,

    /// Last ledger to reprice (inclusive). Keep this below the SDEX live floor
    /// to avoid same-source minute overlap with live ingestion (which silently
    /// undercounts under ReplacingMergeTree) — AMM sources are independent of
    /// SDEX, but align to the operator's disjoint-range rule regardless.
    #[arg(long)]
    pub end: u32,

    /// Ledgers per read/flush chunk. Each chunk is read, classified, and its
    /// candles flushed+written before the next — bounds peak memory on a
    /// multi-million-ledger range.
    #[arg(long, default_value_t = 320_000)]
    pub chunk_size: u32,

    /// ClickHouse URL. The single client reads `default.*` (BE tables) AND
    /// writes `prices.*`, so run it as a user with access to both databases —
    /// on `ch-prod-01` that is the `default` user (the prices mTLS user cannot
    /// read `default.*`).
    #[arg(long, env = "CLICKHOUSE_URL", default_value = "http://localhost:8123")]
    pub clickhouse_url: String,

    /// ClickHouse user. Defaults to `default` (the ch-prod-01 account that can
    /// both read `default.*` and write `prices.*`).
    ///
    /// Must always be sent: the client sets `X-ClickHouse-User` and
    /// `X-ClickHouse-Key` as independent headers, so a password with no user is
    /// a key with nobody to authenticate — ClickHouse rejects it. Leaving this
    /// unset while passing `CLICKHOUSE_PASSWORD` produced exactly that.
    #[arg(long, env = "CLICKHOUSE_USER", default_value = "default")]
    pub clickhouse_user: String,

    /// ClickHouse password (optional). Read from env to avoid shell history.
    #[arg(long, env = "CLICKHOUSE_PASSWORD")]
    pub clickhouse_password: Option<String>,

    /// Read + classify but write nothing. Prints per-source tick counts so the
    /// operator can sanity-check coverage against `soroban_events` swap counts
    /// before committing writes.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// Verbose (DEBUG) logging.
    #[arg(long, default_value_t = false)]
    pub verbose: bool,
}
