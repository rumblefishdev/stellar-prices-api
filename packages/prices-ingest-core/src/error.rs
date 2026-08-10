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

    // Self-redacting: a ClickHouse `BadResponse` body can echo offending row
    // values, so the `Display` emits only the leading `Code: NNN` / status
    // token, never the raw body. Applying it on the shared error means every
    // consumer of the writer (live Lambda + SDEX backfill) is leak-safe.
    #[error("clickhouse: {}", crate::safe_log::redact_clickhouse(.0))]
    Clickhouse(#[from] clickhouse::error::Error),

    /// A data precondition made a write unsafe, so it was refused rather than
    /// performed. Distinct from a ClickHouse failure: nothing went wrong at the
    /// transport level, we declined to write. Carries only identifiers and
    /// counts, never row values, so it stays leak-safe like the variant above.
    #[error("precondition failed: {0}")]
    Precondition(String),
}
