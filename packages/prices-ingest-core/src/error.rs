//! Shared error type for the ingestion core.

/// Errors raised while decoding ledgers or reading/writing ClickHouse. The
/// SDEX backfill wraps this in its own `BackfillError` (which adds the
/// S3-partition-sync variants); the Lambda surfaces it through its reconcile
/// error. Keeping the shared variants here means both binaries classify
/// transient ClickHouse failures the same way.
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("xdr parse: {0}")]
    Parse(#[from] xdr_parser::ParseError),

    #[error("clickhouse: {0}")]
    Clickhouse(#[from] clickhouse::error::Error),
}
