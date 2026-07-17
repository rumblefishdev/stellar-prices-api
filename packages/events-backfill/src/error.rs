#[derive(Debug, thiserror::Error)]
pub enum EventsBackfillError {
    // Self-redacting, exactly like `IngestError::Clickhouse`: a ClickHouse
    // `BadResponse` body echoes server-side detail (and can echo offending row
    // values), so emit only the leading `Code: NNN` token. Rendering `{0}`
    // here forwarded the crate's full `Display` and bypassed the shared
    // redaction that the rest of the pipeline applies.
    #[error("clickhouse: {}", prices_ingest_core::safe_log::redact_clickhouse(.0))]
    Clickhouse(#[from] clickhouse::error::Error),

    #[error("ingest: {0}")]
    Ingest(#[from] prices_ingest_core::IngestError),

    #[error("invalid range: start ({start}) > end ({end})")]
    InvalidRange { start: u32, end: u32 },

    #[error("chunk-size must be >= 1")]
    InvalidChunkSize,

    #[error(
        "no AMM pools in prices.pool_registry — seed it first (pool-registry-seed) so the \
         events read has contracts to filter on; an empty registry reprices nothing"
    )]
    EmptyPoolRegistry,

    #[error(
        "none of the {0} registry pools resolved to a default.soroban_contracts id — nothing \
         to read (is BE's soroban_contracts populated for this cluster?)"
    )]
    NoResolvedContracts(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: this variant rendered `{0}`, forwarding the crate's whole
    /// `Display`, so a real prod failure logged the full server body —
    /// `"clickhouse: bad response: Code: 241. DB::Exception: ... /var/lib/
    /// clickhouse/store/..."` — bypassing the redaction every other error path
    /// applies.
    #[test]
    fn clickhouse_variant_redacts_to_code_only() {
        let err = EventsBackfillError::Clickhouse(clickhouse::error::Error::BadResponse(
            "Code: 241. DB::Exception: Query memory limit exceeded: would use 5.59 GiB \
             (while reading column topics_xdr) from part /var/lib/clickhouse/store/1c3/..."
                .into(),
        ));
        let rendered = err.to_string();
        assert_eq!(rendered, "clickhouse: Code: 241");
        assert!(!rendered.contains("/var/lib/clickhouse"));
        assert!(!rendered.contains("topics_xdr"));
    }

    #[test]
    fn ingest_variant_still_redacts_through_ingest_error() {
        let err = EventsBackfillError::Ingest(prices_ingest_core::IngestError::Clickhouse(
            clickhouse::error::Error::BadResponse("Code: 516. DB::Exception: secret".into()),
        ));
        assert_eq!(err.to_string(), "ingest: clickhouse: Code: 516");
    }
}
