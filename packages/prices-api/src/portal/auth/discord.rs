//! The three calls this service makes to Discord, and nothing else
//! (tasks 0186 + 0189).
//!
//! `POST /oauth2/token` to turn an authorization code into an access token,
//! `GET /users/@me/guilds/{guild}/member` on an issue round-trip to ask whether
//! that user is a member of the Stellar guild, then `GET /users/@me` to read
//! who the token belongs to. The token is dropped at the end of the callback —
//! see [`AccessToken`].
//!
//! # Scope is exactly `identify` + `guilds.members.read`
//!
//! Requested in [`super::authorize_url`] and **verified in the token response**
//! by [`TokenResponse::granted_scopes`]. Verifying is not paranoia about
//! Discord: the requested scope set is also declared in the Developer Portal, so
//! the authorize URL and the registration can disagree, and the response is the
//! only place the actual grant is observable. The comparison is **set
//! equality** over whitespace-separated tokens — RFC 6749 §3.3 makes scope an
//! unordered set, so `guilds.members.read identify` is the same grant — and a
//! set compare still refuses anything wider *or* narrower. `guilds` and
//! `email` are refused outright by ADR 0010, the first for returning every
//! server a user belongs to and the second for collecting data we have decided
//! not to hold; `guilds.members.read` returns one membership in one guild the
//! user consented to reveal, which is the narrowest surface that can answer
//! the question at all.

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

/// The scopes requested, sent to the authorize endpoint and checked on the way
/// back — as a set, see [`scopes_match`]. ADR 0010: never `guilds`, never
/// `email`. The same pair must be declared in the Developer Portal
/// registration (deploy-prep runbook §1 step 3); a registration that drifts
/// narrower is refused by Discord at the authorize step, one that drifts wider
/// is refused here on the token response.
pub const SCOPE: &str = "identify guilds.members.read";

/// Discord's OAuth2 authorize page — where the visitor is sent, not an API call.
pub const DEFAULT_AUTHORIZE_URL: &str = "https://discord.com/oauth2/authorize";

/// Base of Discord's REST API. `/oauth2/token` and `/users/@me` hang off it.
pub const DEFAULT_API_BASE: &str = "https://discord.com/api";

/// Timeout for each Discord call.
///
/// The api-handler's own Lambda timeout is 15s (`production.json`) and API
/// Gateway's ceiling is 29s, so untimed calls could burn the whole budget and
/// return nothing. A Discord that is slower than this is a Discord that is
/// down — which the visitor is better told about than left waiting for.
///
/// ⚠️ **Four seconds, not five, since 2026-08-27.** The sign-in callback now
/// makes THREE calls where it made two — the token exchange, the membership
/// read added on 2026-08-26, and the identity read — and they are serial by
/// design (the token is borrowed by the first and consumed by the last). At
/// five seconds each plus the parameter reads, the slow-but-not-failing case
/// summed past the 15s invocation timeout: the Lambda was killed and the
/// browser got a bare API Gateway 502 instead of any of the designed screens.
/// The arithmetic that has to keep holding, worst case:
///
/// ```text
/// exchange 4s + parameters 2s + membership 4s + identity 4s = 14s < 15s
/// ```
///
/// `PARAMETER_TIMEOUT` in `portal/eligibility.rs` is the 2s term. Raising
/// either constant, or adding a fourth call, needs this sum redone and
/// `timeoutSeconds` in `infra/envs/production.json` checked against it —
/// `issue::tests::budget_arithmetic_fits_the_lambda` does the sum.
pub(super) const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);

/// Endpoints, separated from the credentials so tests can point them at a
/// loopback mock while using the same code path production does.
///
/// # The overrides are compiled out of the Lambda
///
/// `DISCORD_AUTHORIZE_URL` and `DISCORD_API_BASE` are read by
/// [`Endpoints::from_env`] **only in builds without the `lambda` feature** —
/// the default build, the test build, and `local-server`. A Lambda build has no
/// code path that reads them at all.
///
/// This used to be a comment claiming "there is no deployed configuration in
/// which these are attacker- or operator-reachable", and that claim was simply
/// false: `AppConfig::from_env` called this unconditionally, so a Lambda
/// environment variable changed where the OAuth flow pointed. Anyone holding
/// `lambda:UpdateFunctionConfiguration` — a permission distinct from
/// `UpdateFunctionCode`, and one an ops role plausibly has on its own — could
/// aim [`exchange_code`] at a host of their choosing. That request carries
/// `client_secret` and the authorization `code` in its form body, so it
/// exfiltrates the client secret **without a single `GetSecretValue` of the
/// attacker's own appearing in CloudTrail**: the Lambda performs its usual,
/// legitimate read and then posts the result out.
///
/// Malice is not required either. A typo in a future `compute-stack.ts` edit,
/// or a debugging override nobody removed, redirects sign-in just as
/// effectively and far more quietly.
///
/// So the boundary is now enforced by the compiler rather than asserted by a
/// comment. `compute-stack.ts` still sets neither variable; that is now a
/// second line of defence instead of the only one.
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
    ///
    /// Non-`lambda` builds only — see the type's documentation for why.
    #[cfg(not(feature = "lambda"))]
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

    /// Discord's real endpoints, unconditionally.
    ///
    /// The `lambda` build reads no environment variable here, so there is
    /// nothing for a `lambda:UpdateFunctionConfiguration` holder — or a
    /// mistaken CDK edit — to point elsewhere. Same signature as the build
    /// above so `AppConfig::from_env` needs no `cfg` of its own.
    #[cfg(feature = "lambda")]
    pub fn from_env() -> Self {
        Self::default()
    }

    fn token_url(&self) -> String {
        format!("{}/oauth2/token", self.api_base.trim_end_matches('/'))
    }

    fn current_user_url(&self) -> String {
        format!("{}/users/@me", self.api_base.trim_end_matches('/'))
    }

    /// The membership route for one guild (task 0189).
    ///
    /// The guild id is operator-seeded configuration (SSM), not code — it is
    /// validated to be a bare snowflake before it becomes a path segment, so a
    /// mis-seeded value cannot smuggle `../` or a query string into the URL.
    /// Validation failure is the caller's `Unknown` outcome, not a panic.
    fn member_url(&self, guild_id: &str) -> Option<String> {
        if !is_snowflake(guild_id) {
            return None;
        }
        Some(format!(
            "{}/users/@me/guilds/{guild_id}/member",
            self.api_base.trim_end_matches('/')
        ))
    }
}

/// Compare a granted scope string against [`SCOPE`] as a **set**.
///
/// RFC 6749 §3.3: scope is a space-delimited, unordered set. String equality
/// would refuse `guilds.members.read identify` — the exact grant we asked for,
/// echoed in an order we do not control. A set compare is order-independent
/// and still refuses a grant that is wider (registration drifted to include
/// `guilds` or `email`) or narrower (the member scope missing, which would
/// turn every membership check into a `401`-shaped `Unknown`).
fn scopes_match(granted: &str) -> bool {
    use std::collections::BTreeSet;
    let granted: BTreeSet<&str> = granted.split_whitespace().collect();
    let requested: BTreeSet<&str> = SCOPE.split_whitespace().collect();
    granted == requested
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

    // Compared as a set, not as "contains identify". A grant of
    // `identify guilds guilds.members.read` contains both requested scopes and
    // is exactly what must be refused: it would mean the Developer Portal
    // registration had drifted from this code, and ADR 0010 rejects `guilds`
    // on privacy grounds. See `scopes_match` for why not string equality.
    if !scopes_match(token.granted_scopes()) {
        return Err(DiscordError::UnexpectedScope {
            granted: token.granted_scopes().to_string(),
        });
    }

    Ok(AccessToken(token.access_token))
}

/// The fields of `GET /users/@me/guilds/{guild}/member` this service reads.
///
/// One, out of the dozen the endpoint returns. `flags` and `joined_at` are
/// deliberately not deserialized: nothing here stores membership data (ADR
/// 0010 — "the registry stores no membership data"), and what is not named
/// cannot leak into a log.
#[derive(Debug, Deserialize)]
pub struct GuildMember {
    /// Membership Screening: `Some(true)` means the user joined but has not
    /// cleared the screening gate. **Optional on purpose** — the docs'
    /// presence guarantee is written about gateway events, not this REST
    /// route, and 0180 item 2 (which would have settled it) is still
    /// unmeasured. Absent is a third state the caller must handle, never
    /// "cleared".
    pub pending: Option<bool>,
}

/// What the membership route answered. Three outcomes, not two, and
/// deliberately infallible: a Discord that is down is an ordinary answer here,
/// not an error for the callback to 502 on, because "could not verify" must
/// never be rendered as "not a member" (task 0189).
#[derive(Debug)]
pub enum MemberLookup {
    /// A member object came back. Whether it *counts* is eligibility's call.
    Member(GuildMember),
    /// A `404` whose JSON `code` is 10007 (Unknown Member) or 10004 (Unknown
    /// Guild) — the only shapes read as "not a member". 10004 usually means
    /// the *guild id* is wrong, which is our configuration and not the user;
    /// the caller logs it loudly for exactly that reason.
    NotMember { code: u64 },
    /// Anything else: `401`/`403`/`429`/`5xx`, a `404` whose body does not
    /// carry a recognised code, a transport fault, an unparseable body. Do not
    /// issue, and do not accuse.
    Unknown {
        status: Option<reqwest::StatusCode>,
        detail: String,
    },
}

/// JSON error codes on a `404` that mean "no such membership".
///
/// 10007 "Unknown Member", 10004 "Unknown Guild" — the only two documented
/// shapes. The exact live behaviour is 0180 item 1, still unmeasured; until it
/// is, an unlisted code on a 404 lands in `Unknown`, which fails safe in both
/// directions (no key issued, no accusation rendered).
const NOT_MEMBER_CODES: [u64; 2] = [10_007, 10_004];

/// Whether `value` has the shape of a Discord snowflake.
///
/// Shared by [`Endpoints::member_url`], which builds a **URL path segment**
/// out of a guild id, and by `eligibility::EligibilitySettings::guild_id`,
/// which is what validates the operator's seed. Those two were allowed to
/// disagree, and the disagreement had a cost: `guild_id` checked only for
/// emptiness, so `stellar_test` — the value the task's own parameter table
/// named for the build period — passed the cold-start probe, deployed green,
/// and then answered "we could not verify your Discord membership" to every
/// visitor forever, because the check ran here instead and produced
/// [`MemberLookup::Unknown`] once per request. One predicate, so the seed
/// cannot be accepted in a shape the caller cannot use.
///
/// Deliberately **shape only**: digits, non-empty, and inside `u64`. There is
/// no minimum length, because a well-formed id for the wrong guild is not this
/// function's problem — Discord answers `10004` for it and `complete_issue`
/// logs that loudly against the parameter name. Inventing a length floor here
/// would risk refusing a legitimate id to catch a case that is already caught.
pub(crate) fn is_snowflake(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) && value.parse::<u64>().is_ok()
}

/// Ask whether the token's owner is a member of `guild_id` (task 0189).
///
/// Called with the **user's own** consented token — no bot in the guild, no
/// admin rights — and **by reference**: it runs before [`current_user`]
/// consumes the token, so the consumed-at-end property of the callback is
/// untouched.
pub async fn guild_member(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    token: &AccessToken,
    guild_id: &str,
) -> MemberLookup {
    let Some(url) = endpoints.member_url(guild_id) else {
        return MemberLookup::Unknown {
            status: None,
            detail: "guild id is not a snowflake — check the SSM parameter".into(),
        };
    };

    let response = match client.get(&url).bearer_auth(&token.0).send().await {
        Ok(response) => response,
        Err(e) => {
            return MemberLookup::Unknown {
                status: None,
                detail: e.to_string(),
            };
        }
    };

    let status = response.status();
    // The body is read for classification only (the JSON `code` on a 404) and
    // is never echoed to the visitor or logged verbatim — same caution as the
    // token exchange above.
    let body = response.bytes().await.unwrap_or_default();
    classify_member_response(status, &body)
}

/// Map one HTTP answer from the member route to a [`MemberLookup`].
///
/// Pure so the whole decision table is unit-testable without a socket. Only a
/// `404` carrying a recognised JSON `code` is `NotMember`; a `2xx` must parse
/// as a member object; everything else is `Unknown`.
fn classify_member_response(status: reqwest::StatusCode, body: &[u8]) -> MemberLookup {
    if status.is_success() {
        return match serde_json::from_slice::<GuildMember>(body) {
            Ok(member) => MemberLookup::Member(member),
            Err(e) => MemberLookup::Unknown {
                status: Some(status),
                detail: format!("member object did not parse: {e}"),
            },
        };
    }

    if status == reqwest::StatusCode::NOT_FOUND {
        #[derive(Deserialize)]
        struct ErrorBody {
            code: Option<u64>,
        }
        if let Ok(ErrorBody { code: Some(code) }) = serde_json::from_slice::<ErrorBody>(body)
            && NOT_MEMBER_CODES.contains(&code)
        {
            return MemberLookup::NotMember { code };
        }
        // A 404 with no recognised code is NOT proof of non-membership — it
        // could be a proxy, an outage page, or a shape 0180 item 1 has not
        // measured yet. Fail safe: refuse without accusing.
        return MemberLookup::Unknown {
            status: Some(status),
            detail: "404 without a recognised error code".into(),
        };
    }

    MemberLookup::Unknown {
        status: Some(status),
        detail: "membership not verifiable".into(),
    }
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
    fn the_requested_scopes_are_exactly_identify_and_members_read() {
        assert_eq!(SCOPE, "identify guilds.members.read");
        // The two ADR 0010 forbids, stated as a test so a future edit that adds
        // one has to delete an assertion rather than change a string. Compared
        // per token — `guilds.members.read` legitimately *contains* "guilds".
        for token in SCOPE.split_whitespace() {
            assert_ne!(token, "guilds");
            assert_ne!(token, "email");
        }
    }

    /// RFC 6749 §3.3: scope is an unordered set. Discord echoing the pair in
    /// the other order is the same grant; anything wider or narrower is not.
    #[test]
    fn the_granted_scope_is_compared_as_a_set_not_a_string() {
        assert!(scopes_match("identify guilds.members.read"));
        assert!(scopes_match("guilds.members.read identify"));
        assert!(scopes_match("  guilds.members.read   identify "));

        // Narrower: the member scope missing means every membership check
        // would come back 401-shaped — refuse at the exchange instead.
        assert!(!scopes_match("identify"));
        assert!(!scopes_match("guilds.members.read"));
        assert!(!scopes_match(""));
        // Wider: the registration drifted. `guilds` and `email` are the ADR's
        // named refusals.
        assert!(!scopes_match("identify guilds.members.read guilds"));
        assert!(!scopes_match("identify guilds.members.read email"));
        assert!(!scopes_match("identify guilds"));
    }

    #[test]
    fn the_member_url_is_built_only_from_a_bare_snowflake() {
        let endpoints = Endpoints::default();
        assert_eq!(
            endpoints.member_url("897514728459468821").as_deref(),
            Some("https://discord.com/api/users/@me/guilds/897514728459468821/member")
        );
        // Operator input never becomes a path segment un-validated.
        for bad in ["", "stellar_test", "123/../admin", "123?x=1", "123 456"] {
            assert_eq!(endpoints.member_url(bad), None, "accepted {bad:?}");
        }
    }

    /// The whole decision table for one membership answer, pinned pure.
    ///
    /// Only a 404 carrying JSON code 10007/10004 is "not a member"; every
    /// other refusal — including a 404 whose body is empty, non-JSON or
    /// carries an unlisted code — is `Unknown`, because 0180 item 1 (the live
    /// shape) is unmeasured and an accusation must not rest on a guess.
    #[test]
    fn only_a_recognised_404_code_reads_as_not_a_member() {
        use reqwest::StatusCode;

        let is_not_member = |status: StatusCode, body: &str| {
            matches!(
                classify_member_response(status, body.as_bytes()),
                MemberLookup::NotMember { .. }
            )
        };
        let is_unknown = |status: StatusCode, body: &str| {
            matches!(
                classify_member_response(status, body.as_bytes()),
                MemberLookup::Unknown { .. }
            )
        };

        assert!(is_not_member(
            StatusCode::NOT_FOUND,
            r#"{"message": "Unknown Member", "code": 10007}"#
        ));
        assert!(is_not_member(
            StatusCode::NOT_FOUND,
            r#"{"message": "Unknown Guild", "code": 10004}"#
        ));

        // A 404 that cannot prove what it is.
        assert!(is_unknown(StatusCode::NOT_FOUND, ""));
        assert!(is_unknown(StatusCode::NOT_FOUND, "<html>gateway</html>"));
        assert!(is_unknown(StatusCode::NOT_FOUND, r#"{"code": 0}"#));
        assert!(is_unknown(StatusCode::NOT_FOUND, r#"{"code": 10008}"#));
        assert!(is_unknown(StatusCode::NOT_FOUND, r#"{"message": "hm"}"#));

        // The statuses the task names, plus the ones it implies.
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            // Even a body that LOOKS like a not-member error does not count
            // outside a 404 — a throttled proxy echoing an error template
            // must not read as an accusation.
            assert!(is_unknown(status, r#"{"code": 10007}"#), "{status}");
        }
    }

    /// A member object parses with `pending` present (either value) or absent,
    /// and absence survives as `None` — the third state eligibility handles
    /// explicitly rather than defaulting.
    #[test]
    fn a_member_response_keeps_pending_optional() {
        let of =
            |body: &str| match classify_member_response(reqwest::StatusCode::OK, body.as_bytes()) {
                MemberLookup::Member(m) => m.pending,
                other => panic!("expected a member, got {other:?}"),
            };
        assert_eq!(of(r#"{"pending": false, "flags": 0}"#), Some(false));
        assert_eq!(of(r#"{"pending": true}"#), Some(true));
        assert_eq!(of(r#"{"joined_at": "2020-01-01T00:00:00Z"}"#), None);

        // A 200 whose body is not a member object at all is Unknown, not a
        // pass — a mock or proxy answering `{}` IS a member object (all
        // fields optional), but non-JSON is not.
        assert!(matches!(
            classify_member_response(reqwest::StatusCode::OK, b"not json"),
            MemberLookup::Unknown { .. }
        ));
    }

    /// **The `lambda` build has no code path that reads the overrides.**
    ///
    /// A compile-time assertion rather than a runtime one, and deliberately so:
    /// under `--features lambda` the reading version of `from_env` does not
    /// exist, so there is nothing to set an environment variable against. The
    /// test that *would* prove it at runtime cannot be written, because the
    /// behaviour it would probe has been compiled away — which is the whole
    /// point.
    ///
    /// What this pins is the pair: exactly one `from_env` is compiled in each
    /// configuration, and under `lambda` it is the one that ignores the
    /// environment entirely.
    #[cfg(feature = "lambda")]
    #[test]
    fn the_lambda_build_ignores_the_endpoint_overrides() {
        // SAFETY: single-threaded assertion on a variable no other test in this
        // configuration reads — the reading `from_env` is not compiled here.
        unsafe {
            std::env::set_var("DISCORD_API_BASE", "https://attacker.example/api");
            std::env::set_var("DISCORD_AUTHORIZE_URL", "https://attacker.example/auth");
        }
        let endpoints = Endpoints::from_env();
        assert_eq!(endpoints.api_base, DEFAULT_API_BASE);
        assert_eq!(endpoints.authorize_url, DEFAULT_AUTHORIZE_URL);
        assert!(!endpoints.api_base.contains("attacker"));
        unsafe {
            std::env::remove_var("DISCORD_API_BASE");
            std::env::remove_var("DISCORD_AUTHORIZE_URL");
        }
    }

    /// The mirror of the above: in a build that is NOT the Lambda, the seam
    /// still works, or the local round-trip and the integration suite lose
    /// their mock.
    #[cfg(not(feature = "lambda"))]
    #[test]
    fn a_non_lambda_build_still_honours_the_overrides() {
        // SAFETY: no other test in this file reads these variables, and this
        // one restores them.
        unsafe { std::env::set_var("DISCORD_API_BASE", "http://127.0.0.1:9/api") };
        assert_eq!(Endpoints::from_env().api_base, "http://127.0.0.1:9/api");
        unsafe { std::env::remove_var("DISCORD_API_BASE") };
        assert_eq!(Endpoints::from_env().api_base, DEFAULT_API_BASE);
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
        assert!(!scopes_match(response.granted_scopes()));
    }
}
