//! Opaque keyset-pagination cursor (overview §4.1).
//!
//! A cursor is the Base64-encoded JSON of `{ "v": <sort-value>, "id": <asset_id> }`
//! — the sort column value and asset id of the last returned row (id breaks
//! ties). The value is carried as a string (decimal columns are compared as
//! strings/floats in SQL); callers treat the whole token as opaque.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

/// Longest token we bother decoding. Our own tokens are well under 100 chars
/// (a decimal string + a u32); anything longer is foreign.
const MAX_TOKEN_LEN: usize = 256;

/// Decoded cursor payload. `deny_unknown_fields` so a foreign token with extra
/// keys is a 400, not a silently accepted lookalike.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    /// Sort-column value of the last row.
    pub v: String,
    /// Asset id of the last row (tiebreaker).
    pub id: u32,
}

impl Cursor {
    /// Whether `v` is a plausible payload for the active sort. Numeric sorts
    /// bind `v` into `toFloat64(?)` — a non-numeric value would make ClickHouse
    /// throw, turning a corrupt token into a 500. `code` sorts compare against
    /// `asset_code`, so `v` must look like one (1–12 ASCII alphanumerics; a
    /// numeric-cursor replay fails this via `.`/sign chars).
    ///
    /// Known limitation (recorded in task 0119): the token does not carry which
    /// sort/order produced it, so switching between two same-typed sorts
    /// mid-walk yields a wrong page, not an error.
    pub fn valid_for(&self, numeric_sort: bool) -> bool {
        if numeric_sort {
            self.v.parse::<f64>().is_ok_and(f64::is_finite)
        } else {
            !self.v.is_empty()
                && self.v.len() <= 12
                && self.v.bytes().all(|b| b.is_ascii_alphanumeric())
        }
    }
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
    if token.len() > MAX_TOKEN_LEN {
        return None;
    }
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

    #[test]
    fn rejects_foreign_and_oversized_tokens() {
        // Extra keys → deny_unknown_fields.
        let foreign = STANDARD.encode(r#"{"v":"1.0","id":1,"extra":true}"#);
        assert!(decode(&foreign).is_none());
        // Over the length cap.
        let long = "A".repeat(MAX_TOKEN_LEN + 1);
        assert!(decode(&long).is_none());
    }

    #[test]
    fn valid_for_checks_numeric_payloads() {
        let ok = decode(&encode("1523400.50", 42)).unwrap();
        assert!(ok.valid_for(true));
        let bad = decode(&encode("notanumber", 42)).unwrap();
        assert!(!bad.valid_for(true));
        let inf = decode(&encode("1e999", 42)).unwrap();
        assert!(!inf.valid_for(true)); // parses to +inf — toFloat64 territory
    }

    #[test]
    fn valid_for_checks_code_payloads() {
        let ok = decode(&encode("USDC", 42)).unwrap();
        assert!(ok.valid_for(false));
        let numeric_replayed = decode(&encode("1523400.50", 42)).unwrap();
        assert!(!numeric_replayed.valid_for(false)); // '.' fails the charset
        let empty = decode(&encode("", 42)).unwrap();
        assert!(!empty.valid_for(false));
    }
}
