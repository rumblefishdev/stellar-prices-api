//! Oracle-lookup trait — production-swap seam where the in-memory
//! map becomes an ASOF JOIN folded into the candidate query.

use std::future::Future;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub mod in_memory;

pub use in_memory::InMemoryOracleLookup;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct OracleEntry {
    pub asset_id: String,
    pub oracle_name: String,
    pub timestamp: i64,
    #[serde(with = "rust_decimal::serde::str")]
    pub price_usd: Decimal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OraclePrice {
    pub price_usd: Decimal,
}

#[derive(Debug, thiserror::Error)]
pub enum OracleError {
    #[error("oracle backend error: {0}")]
    Backend(String),
}

/// Forward-fill lookup: most recent oracle entry at or before
/// `at_unix_seconds`, within `window_s` seconds. Returns `None` if
/// no entry is found (treated as a miss by the caller).
pub trait OraclePriceLookup {
    fn lookup(
        &self,
        asset_id: &str,
        at_unix_seconds: i64,
        window_s: u32,
    ) -> impl Future<Output = Result<Option<OraclePrice>, OracleError>> + Send;
}
