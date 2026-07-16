#[derive(Debug, thiserror::Error)]
pub enum EventsBackfillError {
    #[error("clickhouse: {0}")]
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
