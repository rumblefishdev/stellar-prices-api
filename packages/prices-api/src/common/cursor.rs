//! Opaque keyset-pagination cursor (overview §4.1).
//!
//! A cursor is the Base64-encoded JSON of `{ "v": <sort-value>, "id": <asset_id> }`
//! — the sort column value and asset id of the last returned row (id breaks
//! ties). The value is carried as a string (decimal columns are compared as
//! strings/floats in SQL); callers treat the whole token as opaque.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

/// Longest token we bother decoding. Our own tokens stay well under this
/// (a ≤64-byte value + a u64 id of at most 20 digits); anything longer is
/// foreign.
const MAX_TOKEN_LEN: usize = 256;

/// Decoded cursor payload. `deny_unknown_fields` so a foreign token with extra
/// keys is a 400, not a silently accepted lookalike.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    /// Sort-column value of the last row.
    pub v: String,
    /// Asset id of the last row (tiebreaker).
    ///
    /// u64 since task 0139: ids are `xxh3` of the identity. A token issued
    /// while ids were u32 still decodes (a u32 JSON number is a valid u64).
    /// Across the 0139 migration such a token carries an old-space id, so a
    /// walk in flight may skip or repeat rows once; it never errors. No version
    /// key: rejecting `id <= u32::MAX` would refuse real hashes too.
    pub id: u64,
}

/// Longest value accepted where an `asset_code` is compared as a string: the
/// `v` payload of a `sort=code` cursor, and the `?search` prefix. Real codes
/// are ≤12 raw bytes, but the DB legitimately holds empty codes (Soroban rows)
/// and lossy-decoded on-chain garbage (up to 3 bytes per replacement char), so
/// the only safe rule is a byte-length cap — any charset restriction would 400
/// a value the API itself serves (PR #217 review).
pub const MAX_STRING_PAYLOAD_LEN: usize = 64;

impl Cursor {
    /// Whether `v` is a plausible payload for the active sort. Numeric sorts
    /// bind `v` into `toFloat64(?)` — a non-numeric value would make ClickHouse
    /// throw, turning a corrupt token into a 500, so `v` must parse to a finite
    /// f64. String sorts bind `v` into a plain string comparison (no 500 risk),
    /// so only a length cap applies.
    ///
    /// Known limitation (recorded in task 0119): the token does not carry which
    /// sort/order produced it, so switching between two same-typed sorts
    /// mid-walk yields a wrong page, not an error.
    pub fn valid_for(&self, numeric_sort: bool) -> bool {
        if numeric_sort {
            self.v.parse::<f64>().is_ok_and(f64::is_finite)
        } else {
            self.v.len() <= MAX_STRING_PAYLOAD_LEN
        }
    }
}

/// Encode a cursor to an opaque Base64 token. URL-safe alphabet, no padding
/// (PR #217 review): the STANDARD alphabet emits `+`/`=`, and a client echoing
/// `next_cursor` into `?cursor=` without percent-encoding turns `+` into a
/// space — killing pagination on a token the API itself issued.
pub fn encode(v: &str, id: u64) -> String {
    let json = serde_json::to_vec(&Cursor {
        v: v.to_string(),
        id,
    })
    .unwrap_or_default();
    URL_SAFE_NO_PAD.encode(json)
}

/// Decode an opaque token; `None` if malformed (caller maps to a 400).
/// Accepts the STANDARD alphabet too, for tokens issued before the URL-safe
/// switch that are still mid-walk.
pub fn decode(token: &str) -> Option<Cursor> {
    if token.len() > MAX_TOKEN_LEN {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(token)
        .or_else(|_| STANDARD.decode(token))
        .ok()?;
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
    fn a_token_issued_with_a_u32_id_still_decodes() {
        // Exactly what the pre-0139 encoder produced for id 4 (native XLM).
        let old = URL_SAFE_NO_PAD.encode(r#"{"v":"1523400.50","id":4}"#);
        let c = decode(&old).expect("u32-era token decodes");
        assert_eq!(c.id, 4);
        assert_eq!(c.v, "1523400.50");
        let max = URL_SAFE_NO_PAD.encode(format!(r#"{{"v":"1","id":{}}}"#, u32::MAX));
        assert_eq!(decode(&max).expect("decodes").id, u64::from(u32::MAX));
    }

    #[test]
    fn a_u64_id_above_u32_max_round_trips() {
        for id in [u64::from(u32::MAX) + 1, 0x9e37_79b9_7f4a_7c15, u64::MAX] {
            let token = encode("USDC", id);
            assert!(token.len() <= MAX_TOKEN_LEN);
            let c = decode(&token).expect("decodes");
            assert_eq!(c.id, id);
            assert_eq!(c.v, "USDC");
        }
        // The longest value a string sort accepts still fits the token cap.
        let long = encode(&"x".repeat(MAX_STRING_PAYLOAD_LEN), u64::MAX);
        assert_eq!(decode(&long).expect("decodes").id, u64::MAX);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode("!!!not-base64!!!").is_none());
        assert!(decode("YWJj").is_none()); // "abc" — valid base64, not our JSON
    }

    #[test]
    fn issued_tokens_are_url_safe_and_old_standard_tokens_still_decode() {
        // New tokens must survive a query string verbatim: no `+`, `/` or `=`.
        let token = encode("1523400.50", 42);
        assert!(
            token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "token not URL-safe: {token}"
        );
        // Tokens minted before the URL-safe switch (STANDARD alphabet) must
        // keep working mid-walk.
        let old = STANDARD.encode(r#"{"v":"1523400.50","id":42}"#);
        let c = decode(&old).expect("standard-alphabet token decodes");
        assert_eq!(c.id, 42);
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
    fn valid_for_accepts_any_short_string_payload_on_string_sorts() {
        // The DB holds empty codes (Soroban rows) and lossy-decoded garbage —
        // the API must accept back any cursor it can itself issue.
        for v in ["USDC", "", "USD ", "\u{fffd}\u{fffd}", "1523400.50"] {
            let c = decode(&encode(v, 42)).unwrap();
            assert!(c.valid_for(false), "{v:?} should be a valid code payload");
        }
        let long = decode(&encode(&"x".repeat(65), 42)).unwrap();
        assert!(!long.valid_for(false));
    }
}
