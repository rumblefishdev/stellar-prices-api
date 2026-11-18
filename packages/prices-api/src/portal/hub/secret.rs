//! The secret the hub's tokens are signed with (task 0348; the shared sign-in
//! spec's decision 12).
//!
//! # What it is
//!
//! One AWS Secrets Manager secret, `sorobanscan-hub/production/prices-token`,
//! created by the operator (never by CDK, for the reason `auth::secret` gives:
//! a CloudFormation-managed value is restored by the next deploy):
//!
//! ```json
//! { "current": "<base64 of 32 bytes>", "previous": "<base64 of 32 bytes>" }
//! ```
//!
//! The hub signs with `current` only. Prices accepts a token under `current`
//! or `previous`, so the secret is changed without a gap: the old value moves
//! to `previous`, a new one goes in `current`, and `previous` is dropped once
//! every reader has the new pair. Each value is exactly what
//! `openssl rand -base64 32` prints: standard base64, padded, 32 bytes — the
//! same check the hub makes (`bff/src/product-secret.ts`), so a value one side
//! takes is never one the other refuses.
//!
//! # Where it is read from
//!
//! | source | selected by | available |
//! | --- | --- | --- |
//! | Secrets Manager, via the extension | `PORTAL_HUB_SECRET_NAME` | `aws-mtls` builds (i.e. the Lambda) |
//! | the JSON itself | `PORTAL_HUB_SECRET` | **non-`lambda` builds only** |
//!
//! Selected in `crate::config::hub_secret_from_env`. The inline JSON is a local
//! seam compiled out of the Lambda, so the deployed build has no code path
//! that reads this secret's value from the environment.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;

/// The length of each key, in bytes: the width of the HMAC-SHA256 it keys,
/// and what the hub requires (`SECRET_BYTES` in `bff/src/token.ts`).
pub const KEY_BYTES: usize = 32;

/// The parsed secret: the key the hub signs with now, and the one it signed
/// with before, while the secret is being changed.
#[derive(Clone)]
pub struct HubSecret {
    current: [u8; KEY_BYTES],
    previous: Option<[u8; KEY_BYTES]>,
}

/// Redacts both keys: there is nothing in this type a log line may print.
impl std::fmt::Debug for HubSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubSecret")
            .field("current", &"<redacted>")
            .field("previous", &"<redacted>")
            .finish()
    }
}

/// Why the secret could not be had. No variant carries a key or any part of
/// one: each names the field and what is wrong with it.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("fetching the hub token secret `{name}` failed: {message}")]
    Fetch { name: String, message: String },
    #[error("the hub token secret is not the expected JSON object {{current, previous?}}: {0}")]
    Malformed(String),
    #[error(
        "`{field}` in the hub token secret is not standard base64 with its padding \
         (generate one with `openssl rand -base64 32`)"
    )]
    NotBase64 { field: &'static str },
    #[error(
        "`{field}` in the hub token secret is {actual} bytes; exactly {KEY_BYTES} are required \
         (generate one with `openssl rand -base64 32`)"
    )]
    WrongLength { field: &'static str, actual: usize },
}

/// The JSON as it sits in Secrets Manager.
#[derive(Deserialize)]
struct SecretJson {
    current: String,
    #[serde(default)]
    previous: Option<String>,
}

impl HubSecret {
    /// Parse and validate the secret's JSON: `current` required, `previous`
    /// optional (`null` is the same as absent), each exactly [`KEY_BYTES`]
    /// in standard padded base64. A field the secret should not have is
    /// ignored, as the hub's reader ignores it.
    pub fn parse(json: &str) -> Result<HubSecret, SecretError> {
        let parsed: SecretJson = serde_json::from_str(json).map_err(|e| {
            SecretError::Malformed(crate::portal::auth::secret::describe_malformed(&e))
        })?;
        Ok(HubSecret {
            current: decode_key("current", &parsed.current)?,
            previous: parsed
                .previous
                .as_deref()
                .map(|value| decode_key("previous", value))
                .transpose()?,
        })
    }

    /// The production path: the secret named `name` through the Parameters
    /// and Secrets extension, retried on a transient failure, as the OAuth
    /// secret is read (`crate::portal::extension::secret_string`).
    #[cfg(feature = "aws-mtls")]
    pub(crate) async fn from_secrets_manager(name: &str) -> Result<HubSecret, SecretError> {
        let json = crate::portal::extension::secret_string(name)
            .await
            .map_err(|e| SecretError::Fetch {
                name: name.to_string(),
                message: e.to_string(),
            })?;
        HubSecret::parse(&json)
    }

    #[cfg(not(feature = "aws-mtls"))]
    pub(crate) async fn from_secrets_manager(name: &str) -> Result<HubSecret, SecretError> {
        Err(SecretError::Fetch {
            name: name.to_string(),
            message: "this build has no Secrets Manager client (build with `--features lambda`, \
                      or set PORTAL_HUB_SECRET to the secret's JSON for a local run)"
                .into(),
        })
    }

    /// The keys a token may be signed with: `current`, then `previous`.
    pub(crate) fn keys(&self) -> impl Iterator<Item = &[u8]> {
        std::iter::once(&self.current[..]).chain(self.previous.as_ref().map(|k| &k[..]))
    }
}

/// One key from its base64. `STANDARD` decodes canonically: the padding is
/// required and the unused bits of the last character must be zero, so the
/// only text accepted for 32 bytes is the one `openssl rand -base64 32`
/// prints — the hub checks the same by writing the key back.
fn decode_key(field: &'static str, value: &str) -> Result<[u8; KEY_BYTES], SecretError> {
    let bytes = STANDARD
        .decode(value)
        .map_err(|_| SecretError::NotBase64 { field })?;
    let actual = bytes.len();
    bytes
        .try_into()
        .map_err(|_| SecretError::WrongLength { field, actual })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec's test vector's secret: the bytes 1 to 32.
    const VECTOR: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA=";
    /// Another valid key: 32 bytes of 0xAA.
    const OTHER: &str = "qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqo=";

    fn vector_bytes() -> Vec<u8> {
        (1..=32).collect()
    }

    #[test]
    fn current_alone_parses() {
        let secret = HubSecret::parse(&format!(r#"{{"current":"{VECTOR}"}}"#)).unwrap();
        let keys: Vec<&[u8]> = secret.keys().collect();
        assert_eq!(keys, vec![&vector_bytes()[..]]);
    }

    #[test]
    fn previous_is_optional_and_comes_after_current() {
        let secret =
            HubSecret::parse(&format!(r#"{{"current":"{OTHER}","previous":"{VECTOR}"}}"#)).unwrap();
        let keys: Vec<&[u8]> = secret.keys().collect();
        assert_eq!(keys, vec![&[0xAA; 32][..], &vector_bytes()[..]]);

        // `null` is the same as leaving it out.
        let secret =
            HubSecret::parse(&format!(r#"{{"current":"{VECTOR}","previous":null}}"#)).unwrap();
        assert_eq!(secret.keys().count(), 1);
    }

    #[test]
    fn a_missing_current_is_named() {
        for json in ["{}".to_string(), format!(r#"{{"previous":"{VECTOR}"}}"#)] {
            let err = HubSecret::parse(&json).unwrap_err();
            assert!(matches!(err, SecretError::Malformed(_)), "{json}: {err:?}");
            assert!(
                err.to_string().contains("missing field `current`"),
                "{json}: {err}"
            );
        }
    }

    #[test]
    fn a_key_that_is_not_32_bytes_is_refused_and_named() {
        // 31 and 33 bytes, each as `openssl rand -base64` would print it.
        let short = STANDARD.encode([7u8; 31]);
        let long = STANDARD.encode([7u8; 33]);
        for (json, field, actual) in [
            (format!(r#"{{"current":"{short}"}}"#), "current", 31),
            (format!(r#"{{"current":"{long}"}}"#), "current", 33),
            (r#"{"current":""}"#.to_string(), "current", 0),
            (
                format!(r#"{{"current":"{VECTOR}","previous":"{short}"}}"#),
                "previous",
                31,
            ),
        ] {
            match HubSecret::parse(&json) {
                Err(SecretError::WrongLength {
                    field: f,
                    actual: a,
                }) => assert_eq!((f, a), (field, actual), "{json}"),
                other => panic!("{json}: {other:?}"),
            }
        }
    }

    /// Exactly what `openssl rand -base64 32` prints, as the hub requires:
    /// another spelling of the same 32 bytes is a value the hub would refuse,
    /// so it is refused here too.
    #[test]
    fn only_standard_padded_base64_is_a_key() {
        let url_safe = "-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_-=";
        let unpadded = VECTOR.trim_end_matches('=');
        let with_newline = format!("{VECTOR}\\n");
        let spaced = format!(" {VECTOR}");
        // The vector with its last character's two unused bits set: the same
        // 32 bytes to a lenient decoder, not canonical base64.
        let trailing_bits = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyB=";
        for (value, field) in [
            (url_safe.to_string(), "current"),
            (unpadded.to_string(), "current"),
            (with_newline, "current"),
            (spaced, "current"),
            (trailing_bits.to_string(), "current"),
            ("not base64 at all!".to_string(), "current"),
        ] {
            let json = format!(r#"{{"current":"{value}"}}"#);
            match HubSecret::parse(&json) {
                Err(SecretError::NotBase64 { field: f }) => assert_eq!(f, field, "{json}"),
                other => panic!("{json}: {other:?}"),
            }
        }
        let json = format!(r#"{{"current":"{VECTOR}","previous":"{unpadded}"}}"#);
        assert!(matches!(
            HubSecret::parse(&json),
            Err(SecretError::NotBase64 { field: "previous" })
        ));
    }

    #[test]
    fn garbage_is_a_named_error_and_not_a_panic() {
        for json in ["", "not json", "[]", "null", r#"{"current":32}"#] {
            assert!(
                matches!(HubSecret::parse(json), Err(SecretError::Malformed(_))),
                "{json:?}"
            );
        }
    }

    /// A malformed secret's error reaches an ERROR line, so it must not echo
    /// a value — the same rule as the OAuth secret's (review IN-02 there).
    #[test]
    fn an_error_never_echoes_a_value() {
        for json in [
            r#"{"current": 987654321987}"#.to_string(),
            r#"{"current": ["sekrit-in-a-list"]}"#.to_string(),
            r#"{"current": "sekrit-truncated"#.to_string(),
            r#"["sekrit-in-a-list"]"#.to_string(),
            r#"{"current": "sekrit!!"}"#.to_string(),
            format!(
                r#"{{"current": "{}"}}"#,
                STANDARD.encode(b"sekrit-thirty-one-bytes-long-xx")
            ),
        ] {
            let text = HubSecret::parse(&json).unwrap_err().to_string();
            for leaked in ["987654321987", "sekrit", "c2Vr"] {
                assert!(!text.contains(leaked), "{text:?} echoes {leaked}");
            }
        }
    }

    /// Nothing sensitive may reach a log line, a panic message or a trace.
    #[test]
    fn debug_redacts_both_keys() {
        let secret =
            HubSecret::parse(&format!(r#"{{"current":"{OTHER}","previous":"{VECTOR}"}}"#)).unwrap();
        let printed = format!("{secret:?}");
        assert!(printed.contains("<redacted>"), "{printed}");
        for leaked in ["qqqq", "AQID", "170", "[1, 2"] {
            assert!(!printed.contains(leaked), "{printed} shows {leaked}");
        }
    }
}
