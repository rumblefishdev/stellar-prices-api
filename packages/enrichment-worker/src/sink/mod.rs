//! Sink trait — production-swap seam where the prototype-mode
//! stdout / SQL-file writes become a batched `INSERT INTO
//! prices.price_ohlcv` against the staging table (per G-note Part A.2).

use std::future::Future;

use crate::enrich::EnrichedRow;

pub mod sql_file;
pub mod stdout;

pub use sql_file::SqlFileSink;
pub use stdout::StdoutJsonSink;

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    #[error("sink write failed: {0}")]
    Write(String),
}

pub trait EnrichmentSink {
    fn write(&self, rows: &[EnrichedRow]) -> impl Future<Output = Result<(), SinkError>> + Send;
}
