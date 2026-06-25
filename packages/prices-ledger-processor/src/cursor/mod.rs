//! Cursor trait — the production-swap seam for ledger-sequence state.
//! In prod this reads from / writes to a ClickHouse cursor table
//! (see G-note Part D.1 for the design question).

use std::future::Future;

pub mod stub_file;

pub use stub_file::StubFileCursor;

#[derive(Debug, thiserror::Error)]
pub enum CursorError {
    #[error("cursor read failed: {0}")]
    Read(String),
    #[error("cursor write failed: {0}")]
    Write(String),
    #[error("cursor value malformed: {0}")]
    Parse(String),
}

pub trait Cursor {
    fn read(&self) -> impl Future<Output = Result<u64, CursorError>> + Send;
    fn write(&self, value: u64) -> impl Future<Output = Result<(), CursorError>> + Send;
}
