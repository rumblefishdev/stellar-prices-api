//! sdex-backfill — the combined single-pass historical backfill engine
//! (SDEX + Soroban AMM + oracle) behind the `sdex-backfill` binary.
//!
//! Exposed as a library (not just a `[[bin]]`) so integration tests
//! (`tests/*_it.rs`) can drive the sink and engine against a local Docker
//! ClickHouse, matching how the workspace's other crates are tested. `main.rs`
//! is a thin shim over these modules.

pub mod cli;
pub mod error;
pub mod ingest;
pub mod obs;
pub mod partition;
pub mod progress;
pub mod run;
pub mod sink;
pub mod sync;
