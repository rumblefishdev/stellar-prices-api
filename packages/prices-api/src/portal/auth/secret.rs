//! Where the Discord client secret comes from, and where it must never come
//! from (task 0186).
//!
//! ADR 0007 set the precedent with the mTLS material and Tranche 3 AC 6 audits
//! it: **no secret VALUE is an environment variable.** The environment carries a
//! secret *name*; the value is fetched at runtime through the AWS Parameters and
//! Secrets Lambda Extension the api-handler already loads for its ClickHouse
//! certificate. This module is that read, and the parsing and validation of what
//! comes back.
//!
//! # One secret, four fields
//!
//! ```json
//! {
//!   "client_id":          "...",
//!   "client_secret":      "...",
//!   "redirect_uri":       "https://<host>/api-tokens/api/auth/callback",
//!   "session_signing_key": "<64 hex chars>"
//! }
//! ```
//!
//! Three of the four are not secret, and bundling them anyway is a decision:
//!
//! - They are **one operator artefact**. `client_id`, `client_secret` and
//!   `redirect_uri` are all properties of a single Discord application
//!   registration, owned by one person (ADR 0010 §6), and at the custom-domain
//!   cutover ([0195]/[0126]) the redirect URI changes in the Developer Portal
//!   and here **at the same moment**. Two homes for one artefact is two things
//!   to change and one to forget, and forgetting the second is a sign-in that
//!   breaks silently on the cutover — the failure the task asks the runbook to
//!   prevent.
//! - They must **survive a deploy**. The repo already learned this for
//!   operator-owned values: `SecretsStack` "deliberately does NOT create the
//!   secrets" and `compute-stack.ts` reads the ledger cursor from SSM rather
//!   than committing it, because a CloudFormation-managed value is restored by
//!   the next `cdk deploy`. A `redirect_uri` in `production.json` would be
//!   un-pointed by the first deploy after the cutover.
//! - Nothing about a Discord registration is then in the CDK template at all,
//!   which is the smallest surface to review.
//!
//! `session_signing_key` is the odd one out — it belongs to this service rather
//! than to the registration — and it rides along because it is provisioned by
//! the same person in the same step, and because it genuinely is secret.
//!
//! # Two ways in, and only one of them is production
//!
//! | source | selected by | available |
//! | --- | --- | --- |
//! | Secrets Manager, via the extension | `PORTAL_OAUTH_SECRET_NAME` | `aws-mtls` builds (i.e. the Lambda) |
//! | a local JSON file | `PORTAL_OAUTH_SECRET_FILE` | always |
//!
//! The file is for the local round-trip the acceptance criteria require, against
//! a Discord redirect URI pointed at `localhost`. It is a **file** and not an
//! environment variable holding JSON, so that there is no code path anywhere
//! that reads a client secret out of the environment — a path that exists "only
//! for local" is a path production can be misconfigured onto.

use serde::Deserialize;

/// The parsed bundle. Cloneable so [`crate::AppConfig`] stays `Clone`.
#[derive(Clone)]
pub struct OauthSecret {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub signing_key: Vec<u8>,
}

/// Redacts everything, including the two fields that are not secret.
///
/// `client_id` and `redirect_uri` are public values, but a `Debug` that prints
/// half a struct invites the next field to be added to the printed half. There
/// is no diagnostic here worth the risk: the failures this type has are
/// "missing" and "malformed", and both are reported by [`SecretError`] with
/// enough detail to act on.
impl std::fmt::Debug for OauthSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OauthSecret")
            .field("client_id", &"<redacted>")
            .field("client_secret", &"<redacted>")
            .field("redirect_uri", &"<redacted>")
            .field("signing_key", &"<redacted>")
            .finish()
    }
}

/// Shortest accepted `session_signing_key`, in bytes.
///
/// 32, matching the output width of the HMAC-SHA256 it keys. Anything shorter is
/// a key with less entropy than the tag it produces, which makes the signature
/// forgeable by brute force rather than by cryptanalysis. `openssl rand -hex 32`
/// yields 64 characters and comfortably clears it.
const MIN_SIGNING_KEY_BYTES: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error(
        "portal sign-in is configured but no source for its credentials is set; \
         set PORTAL_OAUTH_SECRET_NAME (Lambda) or PORTAL_OAUTH_SECRET_FILE (local)"
    )]
    NoSource,
    #[error("reading `{path}` failed: {source}")]
    ReadFile {
        path: String,
        source: std::io::Error,
    },
    #[error("fetching secret `{name}` failed: {message}")]
    Fetch { name: String, message: String },
    #[error(
        "the portal OAuth secret is not the expected JSON object \
         {{client_id, client_secret, redirect_uri, session_signing_key}}: {0}"
    )]
    Malformed(String),
    #[error("`{field}` in the portal OAuth secret is empty")]
    Empty { field: &'static str },
    #[error(
        "`session_signing_key` is {actual} bytes; at least {MIN_SIGNING_KEY_BYTES} are required \
         (generate one with `openssl rand -hex 32`)"
    )]
    WeakSigningKey { actual: usize },
    #[error(
        "`redirect_uri` is `{uri}`, which does not end in `{expected}` — Discord matches \
         the registered redirect URI exactly, so a callback would be refused at the \
         authorize step"
    )]
    RedirectUriMismatch { uri: String, expected: &'static str },
}

/// The JSON as it sits in Secrets Manager.
#[derive(Deserialize)]
struct SecretJson {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    session_signing_key: String,
}

impl OauthSecret {
    /// Load the bundle from whichever source is configured.
    ///
    /// `Ok(None)` means **no source is configured and none was expected** — the
    /// caller decides whether that is fatal. It is fatal exactly when the portal
    /// is open; see [`crate::AppConfig::load_portal_oauth`].
    pub async fn load() -> Result<Option<Self>, SecretError> {
        // The file is checked FIRST, so a developer with both set gets the local
        // one. On the Lambda only the name is ever set, and nothing sets both.
        if let Ok(path) = std::env::var("PORTAL_OAUTH_SECRET_FILE")
            && !path.is_empty()
        {
            let json = std::fs::read_to_string(&path)
                .map_err(|source| SecretError::ReadFile { path, source })?;
            return Self::parse(&json).map(Some);
        }
        if let Ok(name) = std::env::var("PORTAL_OAUTH_SECRET_NAME")
            && !name.is_empty()
        {
            return Self::from_secrets_manager(&name).await.map(Some);
        }
        Ok(None)
    }

    /// The production path: one HTTP round-trip to the extension on
    /// `localhost:2773`, cached in-process for the life of the execution
    /// environment (`PARAMETERS_SECRETS_EXTENSION_CACHE_ENABLED=true`, set by
    /// `compute-stack.ts`).
    ///
    /// Only compiled where the extension client is: `aws-mtls` is what pulls
    /// `reqwest` into `prices-clickhouse`, and the `lambda` feature enables it.
    /// A default build has no way to reach Secrets Manager and no business
    /// doing so.
    #[cfg(feature = "aws-mtls")]
    async fn from_secrets_manager(name: &str) -> Result<Self, SecretError> {
        let json = prices_clickhouse::mtls::fetch_secret_string(name)
            .await
            .map_err(|e| SecretError::Fetch {
                name: name.to_string(),
                message: e.to_string(),
            })?;
        Self::parse(&json)
    }

    #[cfg(not(feature = "aws-mtls"))]
    async fn from_secrets_manager(name: &str) -> Result<Self, SecretError> {
        Err(SecretError::Fetch {
            name: name.to_string(),
            message: "this build has no Secrets Manager client (build with `--features lambda`, \
                      or point PORTAL_OAUTH_SECRET_FILE at a local file)"
                .into(),
        })
    }

    /// Parse and validate. Split out from both loaders so the validation is
    /// testable without a file or an AWS runtime — it is the part that decides
    /// whether a misconfiguration is caught at cold start or at a visitor's
    /// callback.
    pub fn parse(json: &str) -> Result<Self, SecretError> {
        let parsed: SecretJson =
            serde_json::from_str(json).map_err(|e| SecretError::Malformed(e.to_string()))?;

        for (field, value) in [
            ("client_id", &parsed.client_id),
            ("client_secret", &parsed.client_secret),
            ("redirect_uri", &parsed.redirect_uri),
            ("session_signing_key", &parsed.session_signing_key),
        ] {
            if value.trim().is_empty() {
                return Err(SecretError::Empty { field });
            }
        }

        let signing_key = parsed.session_signing_key.into_bytes();
        if signing_key.len() < MIN_SIGNING_KEY_BYTES {
            return Err(SecretError::WeakSigningKey {
                actual: signing_key.len(),
            });
        }

        // Checked at load rather than trusted, because the alternative failure is
        // remote and confusing: Discord refuses the *authorize* request with its
        // own error page, so the visitor never reaches this service and there is
        // nothing in our logs to explain it.
        if !parsed.redirect_uri.ends_with(super::CALLBACK_PATH) {
            return Err(SecretError::RedirectUriMismatch {
                uri: parsed.redirect_uri,
                expected: super::CALLBACK_PATH,
            });
        }

        Ok(Self {
            client_id: parsed.client_id,
            client_secret: parsed.client_secret,
            redirect_uri: parsed.redirect_uri,
            signing_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_json() -> String {
        serde_json::json!({
            "client_id": "1234567890",
            "client_secret": "a-client-secret",
            "redirect_uri": "https://example.cloudfront.net/api-tokens/api/auth/callback",
            "session_signing_key": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        })
        .to_string()
    }

    #[test]
    fn a_well_formed_bundle_parses() {
        let secret = OauthSecret::parse(&valid_json()).unwrap();
        assert_eq!(secret.client_id, "1234567890");
        assert_eq!(secret.signing_key.len(), 64);
    }

    #[test]
    fn a_short_signing_key_is_refused_rather_than_stretched() {
        let json = valid_json().replace(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "too-short",
        );
        assert!(matches!(
            OauthSecret::parse(&json),
            Err(SecretError::WeakSigningKey { actual: 9 })
        ));
    }

    #[test]
    fn an_empty_field_is_refused() {
        let json = valid_json().replace("a-client-secret", "");
        assert!(matches!(
            OauthSecret::parse(&json),
            Err(SecretError::Empty {
                field: "client_secret"
            })
        ));
    }

    /// The misconfiguration whose symptom is furthest from its cause: Discord
    /// rejects it at the authorize step, on Discord's own page, leaving nothing
    /// in our logs.
    #[test]
    fn a_redirect_uri_that_is_not_the_callback_path_is_refused_at_load() {
        let json = valid_json().replace(
            "/api-tokens/api/auth/callback",
            "/api-tokens/api/auth/callbac",
        );
        assert!(matches!(
            OauthSecret::parse(&json),
            Err(SecretError::RedirectUriMismatch { .. })
        ));
    }

    #[test]
    fn a_localhost_redirect_uri_is_accepted_for_the_local_round_trip() {
        let json = valid_json().replace("https://example.cloudfront.net", "http://localhost:4200");
        assert!(OauthSecret::parse(&json).is_ok());
    }

    #[test]
    fn garbage_is_a_named_error_and_not_a_panic() {
        assert!(matches!(
            OauthSecret::parse("not json at all"),
            Err(SecretError::Malformed(_))
        ));
        assert!(matches!(
            OauthSecret::parse("{}"),
            Err(SecretError::Malformed(_))
        ));
    }

    /// Nothing sensitive may reach a log line, a panic message or a trace.
    #[test]
    fn debug_redacts_every_field() {
        let printed = format!("{:?}", OauthSecret::parse(&valid_json()).unwrap());
        assert!(!printed.contains("a-client-secret"), "{printed}");
        assert!(!printed.contains("0123456789abcdef"), "{printed}");
        assert!(!printed.contains("1234567890"), "{printed}");
    }
}
