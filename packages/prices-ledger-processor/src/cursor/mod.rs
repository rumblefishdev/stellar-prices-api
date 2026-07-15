//! Cursor trait — the production-swap seam for ledger-sequence state.
//! In prod this reads from / writes to a ClickHouse cursor table (task 0064).

use std::future::Future;

pub mod ch;
pub mod stub_file;

pub use ch::ClickHouseCursor;
pub use stub_file::StubFileCursor;

#[derive(Debug, thiserror::Error)]
pub enum CursorError {
    /// No checkpoint exists yet — a genuine first run. Distinct from [`Self::Read`]
    /// so callers can seed on emptiness WITHOUT also seeding on a transient read
    /// failure (which would clobber a healthy durable cursor).
    #[error("cursor is empty (no checkpoint yet)")]
    Empty,
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
