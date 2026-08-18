//! Asset-identifier parsing per overview §4.1.
//!
//! The public API addresses assets by their **natural Stellar identity**, in one
//! of three textual forms:
//!   - `native`                       → the native XLM asset
//!   - `{code}:{issuer}`              → a classic credit asset (issuer is a G-strkey)
//!   - `{contract_address}`          → a Soroban token / SAC (a C-strkey)
//!
//! This module only *parses + validates the shape* (real strkey CRC validation
//! via `stellar-strkey`). Resolving a parsed identity to the internal
//! `prices.assets.asset_id` surrogate is a ClickHouse lookup that lands with the
//! first data route (Phase 2); the resolver signature will live here too.

use stellar_strkey::Contract;
use stellar_strkey::ed25519::PublicKey;

/// Maximum length of a classic asset code (SEP-11 `alphanum12`).
const MAX_CODE_LEN: usize = 12;

/// A parsed, shape-validated asset identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetIdentifier {
    /// The native asset (XLM).
    Native,
    /// A classic credit asset: `code` (1–12 chars) issued by `issuer` (G-strkey).
    Classic { code: String, issuer: String },
    /// A Soroban token or SAC, addressed by its contract (C-strkey).
    Contract(String),
}

/// Why an identifier failed to parse.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdentifierError {
    #[error("empty asset identifier")]
    Empty,
    #[error("invalid asset code (expected 1-{MAX_CODE_LEN} chars)")]
    InvalidCode,
    #[error("invalid issuer (expected a G... strkey)")]
    InvalidIssuer,
    #[error("unrecognized asset identifier (expected `native`, `CODE:ISSUER`, or a C... contract)")]
    Unrecognized,
}

impl AssetIdentifier {
    /// Parse and shape-validate a textual asset identifier.
    pub fn parse(raw: &str) -> Result<Self, IdentifierError> {
        let s = raw.trim();
        if s.is_empty() {
            return Err(IdentifierError::Empty);
        }
        if s.eq_ignore_ascii_case("native") {
            return Ok(Self::Native);
        }
        // `CODE:ISSUER` — the colon disambiguates classic from contract.
        if let Some((code, issuer)) = s.split_once(':') {
            if code.is_empty() || code.len() > MAX_CODE_LEN {
                return Err(IdentifierError::InvalidCode);
            }
            PublicKey::from_string(issuer).map_err(|_| IdentifierError::InvalidIssuer)?;
            return Ok(Self::Classic {
                code: code.to_string(),
                issuer: issuer.to_string(),
            });
        }
        // Otherwise the only valid form is a contract (C-strkey).
        if Contract::from_string(s).is_ok() {
            return Ok(Self::Contract(s.to_string()));
        }
        Err(IdentifierError::Unrecognized)
    }

    /// Render the canonical textual form (the inverse of [`parse`]). Echoed back
    /// in responses as the `asset` field.
    ///
    /// [`parse`]: AssetIdentifier::parse
    pub fn to_canonical(&self) -> String {
        match self {
            Self::Native => "native".to_string(),
            Self::Classic { code, issuer } => format!("{code}:{issuer}"),
            Self::Contract(c) => c.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valid G-strkey, built via round-trip so the test never hardcodes a
    /// possibly-stale literal.
    fn sample_issuer() -> String {
        PublicKey([7u8; 32]).to_string()
    }

    /// A valid C-strkey, built the same way.
    fn sample_contract() -> String {
        Contract([9u8; 32]).to_string()
    }

    #[test]
    fn parses_native_case_insensitively() {
        assert_eq!(
            AssetIdentifier::parse("native"),
            Ok(AssetIdentifier::Native)
        );
        assert_eq!(
            AssetIdentifier::parse("NATIVE"),
            Ok(AssetIdentifier::Native)
        );
        assert_eq!(
            AssetIdentifier::parse("  native  "),
            Ok(AssetIdentifier::Native)
        );
    }

    #[test]
    fn parses_classic_code_issuer() {
        let issuer = sample_issuer();
        let parsed = AssetIdentifier::parse(&format!("USDC:{issuer}")).unwrap();
        assert_eq!(
            parsed,
            AssetIdentifier::Classic {
                code: "USDC".to_string(),
                issuer,
            }
        );
    }

    #[test]
    fn rejects_classic_with_bad_issuer() {
        assert_eq!(
            AssetIdentifier::parse("USDC:not-a-strkey"),
            Err(IdentifierError::InvalidIssuer)
        );
    }

    #[test]
    fn rejects_classic_with_too_long_code() {
        let issuer = sample_issuer();
        assert_eq!(
            AssetIdentifier::parse(&format!("THIRTEENCHARS:{issuer}")),
            Err(IdentifierError::InvalidCode)
        );
    }

    #[test]
    fn parses_contract() {
        let contract = sample_contract();
        assert_eq!(
            AssetIdentifier::parse(&contract),
            Ok(AssetIdentifier::Contract(contract))
        );
    }

    #[test]
    fn rejects_garbage_and_empty() {
        assert_eq!(AssetIdentifier::parse(""), Err(IdentifierError::Empty));
        assert_eq!(AssetIdentifier::parse("   "), Err(IdentifierError::Empty));
        assert_eq!(
            AssetIdentifier::parse("definitely-not-an-asset"),
            Err(IdentifierError::Unrecognized)
        );
    }
}
