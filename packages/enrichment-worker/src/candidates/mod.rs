//! Candidate-source trait — production-swap seam where the JSONL
//! reader becomes a CH `SELECT … FROM prices.price_ohlcv FINAL
//! WHERE volume_quote_usd = 0 …` query (per G-note Part A.2).

use std::future::Future;

pub mod jsonl_file;

pub use jsonl_file::JsonlCandidateSource;

use crate::enrich::Candidate;

#[derive(Debug, thiserror::Error)]
pub enum CandidateError {
    #[error("candidate source error: {0}")]
    Backend(String),
}

pub trait CandidateSource {
    /// Read up to `limit` candidates. Returns fewer than `limit`
    /// (possibly zero) when the source is exhausted.
    fn next_batch(
        &mut self,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Candidate>, CandidateError>> + Send;
}
