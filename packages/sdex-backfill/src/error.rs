use std::io;

#[derive(Debug, thiserror::Error)]
pub enum BackfillError {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("aws s3 sync failed for partition {partition} (exit {exit_code}): {stderr}")]
    AwsSyncFailed {
        partition: u32,
        exit_code: i32,
        stderr: String,
    },

    #[error("partition {partition_start} sync incomplete: local={local}, s3={s3}, need={need}")]
    PartitionSyncFailed {
        partition_start: u32,
        local: usize,
        s3: usize,
        need: usize,
    },

    #[error("s3 ls failed for partition {partition_start}: {source}")]
    S3LsFailed {
        partition_start: u32,
        source: io::Error,
    },

    #[error("ledger file missing: partition={partition} seq={seq} path={path}")]
    LedgerFileMissing {
        partition: u32,
        seq: u32,
        path: String,
    },

    #[error("xdr parse: {0}")]
    Parse(#[from] xdr_parser::ParseError),

    #[error("clickhouse: {0}")]
    Clickhouse(#[from] clickhouse::error::Error),

    #[error("ingest: {0}")]
    Ingest(#[from] prices_ingest_core::IngestError),

    #[error(
        "{0} AMM pool(s) had swaps dropped for lack of registration — \
         see prices.unresolved_pools WHERE still_unresolved = 1"
    )]
    UnresolvedPools(usize),

    #[cfg(feature = "aws-mtls")]
    #[error("--transport hetzner requires these arg(s)/env var(s): {0}")]
    MissingMtlsArg(String),

    #[error("mtls sink init failed: {0}")]
    Mtls(String),
}
