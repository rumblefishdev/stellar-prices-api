//! OHLCV sink trait — the production-swap seam where prototype-mode
//! stdout / SQL-file writes become a `clickhouse::Client` insert against
//! `prices.price_ohlcv` (per ADRs 0003, 0004, 0007).

use std::future::Future;

use crate::bucket::OhlcvRow;

pub mod sql_file;
pub mod stdout;

pub use sql_file::SqlFileSink;
pub use stdout::StdoutJsonSink;

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    #[error("sink write failed: {0}")]
    Write(String),
}

pub trait OhlcvSink {
    fn write(&self, rows: &[OhlcvRow]) -> impl Future<Output = Result<(), SinkError>> + Send;
}
