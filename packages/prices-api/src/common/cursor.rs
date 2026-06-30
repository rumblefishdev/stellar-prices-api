//! Opaque keyset-pagination cursor (overview §4.1).
//!
//! A cursor is the Base64-encoded JSON of `{ "v": <sort-value>, "id": <asset_id> }`
//! — the sort column value and asset id of the last returned row (id breaks
//! ties). The value is carried as a string (decimal columns are compared as
//! strings/floats in SQL); callers treat the whole token as opaque.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

/// Decoded cursor payload.
#[derive(Debug, Serialize, Deserialize)]
pub struct Cursor {
    /// Sort-column value of the last row.
    pub v: String,
    /// Asset id of the last row (tiebreaker).
    pub id: u32,
}

/// Encode a cursor to an opaque Base64 token.
pub fn encode(v: &str, id: u32) -> String {
    let json = serde_json::to_vec(&Cursor {
        v: v.to_string(),
        id,
    })
    .unwrap_or_default();
    STANDARD.encode(json)
}

/// Decode an opaque token; `None` if malformed (caller maps to a 400).
pub fn decode(token: &str) -> Option<Cursor> {
    let bytes = STANDARD.decode(token).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let token = encode("1523400.50", 42);
        let c = decode(&token).expect("decodes");
        assert_eq!(c.v, "1523400.50");
        assert_eq!(c.id, 42);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode("!!!not-base64!!!").is_none());
        assert!(decode("YWJj").is_none()); // "abc" — valid base64, not our JSON
    }
}
