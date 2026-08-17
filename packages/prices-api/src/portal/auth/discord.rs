//! The two calls this service makes to Discord, and nothing else (task 0186).
//!
//! `POST /oauth2/token` to turn an authorization code into an access token, then
//! `GET /users/@me` to read who it belongs to. The token is dropped at the end of
//! the callback — see [`AccessToken`].
//!
//! # Scope is exactly `identify`
//!
//! Requested in [`super::authorize_url`] and **verified in the token response**
//! by [`TokenResponse::granted_scopes`]. Verifying is not paranoia about
//! Discord: the requested scope set is also declared in the Developer Portal, so
//! the authorize URL and the registration can disagree, and the response is the
//! only place the actual grant is observable. `guilds.members.read` is [0189]'s
//! to add — in both places — and `guilds` and `email` are refused outright by
//! ADR 0010, the first for returning every server a user belongs to and the
//! second for collecting data we have decided not to hold.

use std::time::Duration;

use serde::Deserialize;

/// A Discord access token, wrapped so it cannot be printed by accident.
///
/// It lives for the duration of one callback: created by [`exchange_code`],
/// consumed by [`current_user`], dropped. Nothing persists it — see the module
/// docs on [`super::session`] for why that is a decision rather than an omission.
pub struct AccessToken(String);

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccessToken(<redacted>)")
    }
}

/// The fields of `GET /users/@me` this service reads.
///
/// Two of them, out of the twenty-odd the endpoint returns. `serde` ignores the
/// rest, which is the behaviour wanted: nothing that is not named here can be
/// accidentally carried into a log or a session.
#[derive(Debug, Deserialize)]
pub struct DiscordUser {
    /// The snowflake. A string in the JSON because it exceeds 2^53 — never parse
    /// it into an `f64`, and note ADR 0010's account-age derivation ([0189])
    /// needs `BigInt`-equivalent handling for the same reason.
    pub id: String,
    /// The unique handle (`adam`, not the display name).
    pub username: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscordError {
    #[error("could not reach Discord at {url}: {message}")]
    Unreachable { url: String, message: String },
    #[error("Discord answered {status} to {url}")]
    Status {
        url: String,
        status: reqwest::StatusCode,
    },
    #[error("could not read Discord's response to {url}: {message}")]
    Decode { url: String, message: String },
    #[error(
        "Discord granted scopes `{granted}`; this service requests and accepts \
         exactly `{}`",
        SCOPE
    )]
    UnexpectedScope { granted: String },
}

/// The one scope requested, sent to the authorize endpoint and checked on the
/// way back. ADR 0010: never `guilds`, never `email`.
pub const SCOPE: &str = "identify";

/// Discord's OAuth2 authorize page — where the visitor is sent, not an API call.
pub const DEFAULT_AUTHORIZE_URL: &str = "https://discord.com/oauth2/authorize";

/// Base of Discord's REST API. `/oauth2/token` and `/users/@me` hang off it.
pub const DEFAULT_API_BASE: &str = "https://discord.com/api";

/// Timeout for each of the two calls.
///
/// The api-handler's own Lambda timeout is 15s (`production.json`) and API
/// Gateway's ceiling is 29s, so two untimed calls could burn the whole budget
/// and return nothing. Five seconds each leaves room for both plus the handler's
/// own work, and a Discord that is slower than that is a Discord that is down —
/// which the visitor is better told about than left waiting for.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Endpoints, separated from the credentials so tests can point them at a
/// loopback mock while using the same code path production does.
///
/// Overridable from the environment (`DISCORD_AUTHORIZE_URL`, `DISCORD_API_BASE`)
/// for the local round-trip. Not set by `compute-stack.ts` — production always
/// takes the defaults above, which is the property that makes the override safe
/// to have: there is no deployed configuration in which these are attacker- or
/// operator-reachable.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub authorize_url: String,
    pub api_base: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            authorize_url: DEFAULT_AUTHORIZE_URL.to_string(),
            api_base: DEFAULT_API_BASE.to_string(),
        }
    }
}

impl Endpoints {
    /// Read the two overrides, falling back to Discord's real endpoints.
    pub fn from_env() -> Self {
        let read = |key: &str, default: &str| {
            std::env::var(key)
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        Self {
            authorize_url: read("DISCORD_AUTHORIZE_URL", DEFAULT_AUTHORIZE_URL),
            api_base: read("DISCORD_API_BASE", DEFAULT_API_BASE),
        }
    }

    fn token_url(&self) -> String {
        format!("{}/oauth2/token", self.api_base.trim_end_matches('/'))
    }

    fn current_user_url(&self) -> String {
        format!("{}/users/@me", self.api_base.trim_end_matches('/'))
    }
}

/// Build the HTTP client used for both calls.
///
/// One client, built once and cloned: `reqwest::Client` is `Arc`-backed, so the
/// clone shares the connection pool and a warm Lambda reuses the TLS session
/// rather than re-handshaking with discord.com on every sign-in. The same
/// reasoning `prices-clickhouse` applies to its ClickHouse pool.
pub fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        // The builder fails only on a TLS backend that cannot initialise, which
        // is a build-configuration fault rather than a runtime condition — and
        // one every request would hit. Loud at cold start beats a 500 per
        // visitor.
        .expect("the reqwest client must build; its TLS backend is compiled in")
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    /// Space-separated, per RFC 6749 §5.1. Absent from Discord's documented
    /// example never happens in practice, but `Option` keeps a missing field
    /// from being an unhelpful decode error rather than a scope refusal.
    scope: Option<String>,
}

impl TokenResponse {
    fn granted_scopes(&self) -> &str {
        self.scope.as_deref().unwrap_or("")
    }
}

/// Exchange an authorization code for an access token, with PKCE.
///
/// `code_verifier` is the value that never left the browser's cookie — sending
/// it here is what proves this exchange belongs to the `/auth/login` that
/// started the flow, and is what makes an intercepted `code` useless on its own.
///
/// The client secret goes in the **form body**, which is where Discord's token
/// endpoint documents it. `client_secret_post` rather than HTTP Basic is
/// Discord's own example; both are RFC 6749-legal and the difference is not
/// security-relevant over TLS.
pub async fn exchange_code(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    secret: &super::secret::OauthSecret,
    code: &str,
    code_verifier: &str,
) -> Result<AccessToken, DiscordError> {
    let url = endpoints.token_url();
    let response = client
        .post(&url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", secret.redirect_uri.as_str()),
            ("client_id", secret.client_id.as_str()),
            ("client_secret", secret.client_secret.as_str()),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .map_err(|e| DiscordError::Unreachable {
            url: url.clone(),
            // `e.to_string()` on a reqwest error names the URL and the transport
            // fault. It cannot contain the form body, so the client secret is
            // not at risk here — but nothing below logs this string anyway.
            message: e.to_string(),
        })?;

    let status = response.status();
    if !status.is_success() {
        // The body is deliberately not read. Discord's OAuth errors are
        // `{"error": "invalid_grant"}`-shaped and useful, but this string ends
        // up in a `tracing::warn!` on a public route, and a body echoed from an
        // upstream is how a token or a code ends up in CloudWatch.
        return Err(DiscordError::Status { url, status });
    }

    let token: TokenResponse = response.json().await.map_err(|e| DiscordError::Decode {
        url: url.clone(),
        message: e.to_string(),
    })?;

    // Compared as a whole string, not as "contains identify". A grant of
    // `identify guilds` contains it and is exactly what must be refused: it
    // would mean the Developer Portal registration had drifted from this code,
    // and ADR 0010 rejects `guilds` on privacy grounds.
    if token.granted_scopes().trim() != SCOPE {
        return Err(DiscordError::UnexpectedScope {
            granted: token.granted_scopes().to_string(),
        });
    }

    Ok(AccessToken(token.access_token))
}

/// Read the identity the token belongs to.
///
/// Takes the token **by value** so it is consumed here: after this call the
/// caller no longer holds one, which is the type system stating the "do not
/// persist Discord tokens" rule rather than a comment stating it.
pub async fn current_user(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    token: AccessToken,
) -> Result<DiscordUser, DiscordError> {
    let url = endpoints.current_user_url();
    let response = client
        .get(&url)
        .bearer_auth(&token.0)
        .send()
        .await
        .map_err(|e| DiscordError::Unreachable {
            url: url.clone(),
            message: e.to_string(),
        })?;

    let status = response.status();
    if !status.is_success() {
        return Err(DiscordError::Status { url, status });
    }

    response.json().await.map_err(|e| DiscordError::Decode {
        url,
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_requested_scope_is_exactly_identify() {
        assert_eq!(SCOPE, "identify");
        // The two ADR 0010 forbids, stated as a test so a future edit that adds
        // one has to delete an assertion rather than change a string.
        assert!(!SCOPE.contains("guilds"));
        assert!(!SCOPE.contains("email"));
    }

    #[test]
    fn endpoints_default_to_discord_and_are_overridable_for_tests() {
        let endpoints = Endpoints::default();
        assert_eq!(
            endpoints.token_url(),
            "https://discord.com/api/oauth2/token"
        );
        assert_eq!(
            endpoints.current_user_url(),
            "https://discord.com/api/users/@me"
        );

        let local = Endpoints {
            api_base: "http://127.0.0.1:9/api/".into(),
            ..Endpoints::default()
        };
        // The trailing slash is absorbed rather than doubled.
        assert_eq!(local.token_url(), "http://127.0.0.1:9/api/oauth2/token");
    }

    #[test]
    fn an_access_token_cannot_be_printed() {
        let token = AccessToken("super-secret-token".into());
        assert_eq!(format!("{token:?}"), "AccessToken(<redacted>)");
    }

    #[test]
    fn a_missing_scope_field_reads_as_no_scope_rather_than_a_decode_error() {
        let response: TokenResponse = serde_json::from_str(r#"{"access_token":"t"}"#).unwrap();
        assert_eq!(response.granted_scopes(), "");
        assert_ne!(response.granted_scopes().trim(), SCOPE);
    }
}
