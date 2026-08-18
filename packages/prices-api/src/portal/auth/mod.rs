//! Sign in with Discord — identity only (task 0186).
//!
//! Four routes under [`PORTAL_API_PREFIX`](super::PORTAL_API_PREFIX), so they
//! inherit [0183]'s gate without it knowing they exist: with `PORTAL_ENABLED`
//! false every one of them is an empty `404`, byte-identical to a path that was
//! never deployed.
//!
//! | route | does |
//! | --- | --- |
//! | `GET /auth/login` | mints `state` + PKCE, redirects to Discord |
//! | `GET /auth/callback` | verifies `state`, exchanges the code, issues a session |
//! | `GET /auth/me` | reports who the caller is, or that they are nobody |
//! | `POST /auth/logout` | clears the session |
//!
//! **Not `crate::auth`.** That module is the partner API's `X-API-Key` gate.
//! This one is the portal's own sign-in, and the two are opposites by design:
//! these routes are exempt from that gate, because a visitor signing in to get
//! a key does not have one yet (`crate::auth::is_exempt`).
//!
//! # What this slice deliberately does not do
//!
//! Everything that turns an identity into an entitlement is [0189]: the
//! `guilds.members.read` scope, the guild membership call, `pending`, the
//! snowflake account-age minimum. Issuing a key is [0187]. Nothing here reads or
//! writes any store — there is no registry yet ([0190] decides whether there
//! ever is one) and no Discord token is kept (see [`session`]).
//!
//! # Why the routes are keyless, and what stands in for a key
//!
//! `apiKeyRequired: false` at the gateway, matching `crate::auth::is_exempt`,
//! because requiring a key to obtain a key is a closed loop. What replaces the
//! usage plan's two limits is a **method-level throttle** declared in
//! `infra/src/lib/stacks/api-gateway-stack.ts` — `PORTAL_THROTTLE`, outside the
//! `cacheEnabled` branch, which is the trap this task's acceptance criteria call
//! out and [0194] audits.

pub mod cookies;
pub mod crypto;
pub mod discord;
pub mod secret;
pub mod session;
pub mod state_token;

use axum::Json;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::header::{LOCATION, SET_COOKIE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use crate::common::{cache_control, errors};

use secret::OauthSecret;
use session::Session;
use state_token::{Action, StateError};

/// Where the visitor starts.
pub const LOGIN_PATH: &str = "/api-tokens/api/auth/login";
/// Where Discord sends them back. **This exact suffix is what is registered in
/// the Discord Developer Portal**, and `secret.rs` refuses a `redirect_uri` that
/// does not end in it.
pub const CALLBACK_PATH: &str = "/api-tokens/api/auth/callback";
/// Who am I?
pub const ME_PATH: &str = "/api-tokens/api/auth/me";
/// Sign out.
pub const LOGOUT_PATH: &str = "/api-tokens/api/auth/logout";

/// Where the callback sends the browser when it is done, in every outcome.
///
/// A **literal**, never anything derived from the request. A `redirect_to`
/// parameter is the standard first link in an open-redirect chain, and this
/// origin is the one that handles OAuth — the same reasoning that keeps
/// `DirectoryIndexFn`'s redirect targets a fixed list in
/// `infra/src/lib/stacks/portal-hosting-stack.ts`. When the portal grows a
/// second page, the page it lands on decides where to go next; this handler
/// still will not.
const PORTAL_HOME: &str = "/api-tokens/";

/// Appended to [`PORTAL_HOME`] when the visitor declined at Discord's consent
/// screen, so [0185]'s page can say "sign-in cancelled" instead of silently
/// looking signed out.
const CANCELLED_QUERY: &str = "?signin=cancelled";

/// Appended to [`PORTAL_HOME`] when Discord refused the request for a reason
/// that is **not** the visitor declining.
///
/// A separate landing state, and a literal like its sibling: the `error` value
/// itself never reaches the URL. Telling the two apart matters because they
/// point at different people — `access_denied` is the visitor's own choice and
/// needs no action from us, while `invalid_scope` or `server_error` is ours or
/// Discord's, and a page that calls it "cancelled" hides it from everyone.
const FAILED_QUERY: &str = "?signin=failed";

/// The one OAuth error code that means the visitor said no (RFC 6749 §4.1.2.1).
///
/// Every other value in that section — `invalid_request`,
/// `unauthorized_client`, `unsupported_response_type`, `invalid_scope`,
/// `server_error`, `temporarily_unavailable` — is a failure somebody has to
/// know about.
const ERROR_ACCESS_DENIED: &str = "access_denied";

/// How much of Discord's `error` value reaches a log line.
///
/// It arrives in a query string on a public, keyless route, so it is
/// attacker-controlled: bounded here rather than trusted to be one of the seven
/// documented codes. Sanitised as well as truncated — see `sanitise_error`.
const ERROR_LOG_MAX: usize = 64;

/// Error code for every `state` rejection.
///
/// One code for all five [`StateError`] variants, on purpose — see the type's
/// own documentation. The variants exist so the tests can tell which check
/// fired; the caller gets the same answer either way.
const INVALID_STATE: &str = "invalid_state";
/// Error code for a Discord round-trip that did not complete.
const DISCORD_UNAVAILABLE: &str = "discord_unavailable";
/// Error code for reaching these routes on a deployment that has no credentials.
const SIGN_IN_UNCONFIGURED: &str = "sign_in_unconfigured";
/// Error code for a deployment whose sign-in configuration cannot produce a
/// valid response — today, only a `Location` that is not a header value.
const SIGN_IN_MISCONFIGURED: &str = "sign_in_misconfigured";

/// Everything the four handlers share, cloned per request.
///
/// `oauth` is `Option` because the api-handler must boot without it. Production
/// runs with `PORTAL_ENABLED=false` for the whole of the build, and a cold start
/// that insisted on reading a secret nobody has created yet would fail Lambda
/// init and take out `/v1` — the data API — to protect a portal that answers
/// `404` regardless. [`crate::AppConfig::load_portal_oauth`] therefore only
/// loads when the portal is open, and only then is a missing secret fatal.
#[derive(Clone)]
pub struct AuthState {
    oauth: Option<std::sync::Arc<OauthSecret>>,
    endpoints: std::sync::Arc<discord::Endpoints>,
    http: reqwest::Client,
}

impl AuthState {
    pub fn new(oauth: Option<OauthSecret>, endpoints: discord::Endpoints) -> Self {
        Self {
            oauth: oauth.map(std::sync::Arc::new),
            endpoints: std::sync::Arc::new(endpoints),
            http: discord::build_client(),
        }
    }
}

/// The four routes, as a `Router` carrying its own state.
///
/// Merged by [`super::apply`] rather than added with `.route()` for the reason
/// stated there: by that point the data routes have consumed `AppState` and the
/// router is `Router<()>`, so a route needing [`AuthState`] has to resolve its
/// own before it joins.
pub fn routes(state: AuthState) -> Router {
    Router::new()
        .route(LOGIN_PATH, get(login))
        .route(CALLBACK_PATH, get(callback))
        .route(ME_PATH, get(me))
        .route(LOGOUT_PATH, post(logout))
        .with_state(state)
}

/// `GET /auth/login` query.
#[derive(Debug, Deserialize)]
struct LoginQuery {
    /// Which action the round-trip authorizes. Absent means sign-in; [0189]
    /// sends `issue` and `rework`. An unknown value is a `400` rather than a
    /// default — see [`Action::parse`].
    action: Option<String>,
}

/// Start a sign-in: mint the pair, set the cookie, redirect to Discord.
async fn login(State(state): State<AuthState>, Query(query): Query<LoginQuery>) -> Response {
    let Some(oauth) = state.oauth.as_ref() else {
        return unconfigured();
    };
    let action = match query.action.as_deref() {
        None => Action::SignIn,
        Some(raw) => match Action::parse(raw) {
            Some(action) => action,
            None => {
                return no_store(errors::bad_request(
                    errors::INVALID_QUERY,
                    "unknown `action`",
                ));
            }
        },
    };

    let started = state_token::start(&oauth.signing_key, action, state_token::now_secs());
    let location = authorize_url(&state.endpoints, oauth, &started);

    // 303, not 302. The visitor arrived by GET so the distinction does not bite
    // here, but 303 states "go and GET this instead" unambiguously, and it is
    // the status the whole flow uses so that no redirect in it can ever be
    // replayed as the method that produced it.
    redirect(
        &location,
        vec![cookies::set(
            cookies::PENDING_COOKIE,
            &started.pending_cookie,
            cookies::PENDING_PATH,
            state_token::PENDING_TTL_SECS,
        )],
    )
}

/// Build the authorize URL the visitor is sent to.
///
/// Every value is percent-encoded by `form_urlencoded` rather than interpolated:
/// `redirect_uri` and the base64url `state` both contain characters that are
/// meaningful in a query string, and a hand-built URL is how a `+` in a `state`
/// becomes a space by the time Discord echoes it back.
fn authorize_url(
    endpoints: &discord::Endpoints,
    oauth: &OauthSecret,
    started: &state_token::StartedLogin,
) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", &oauth.client_id)
        .append_pair("redirect_uri", &oauth.redirect_uri)
        // Exactly `identify`. ADR 0010; also declared in the Developer Portal,
        // and verified again on the token response (`discord::exchange_code`).
        .append_pair("scope", discord::SCOPE)
        .append_pair("state", &started.state_param)
        .append_pair("code_challenge", &started.code_challenge)
        .append_pair("code_challenge_method", "S256")
        .finish();
    format!("{}?{query}", endpoints.authorize_url)
}

/// `GET /auth/callback` query — everything Discord may send back.
#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    /// Present instead of `code` when the visitor declined, per RFC 6749 §4.1.2.1.
    error: Option<String>,
}

/// Finish a sign-in.
async fn callback(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let Some(oauth) = state.oauth.as_ref() else {
        return unconfigured();
    };

    // `state` FIRST, before the outcome is even read.
    //
    // Every branch below — including the "user pressed Cancel" one — runs only
    // once this callback has been shown to belong to the browser that started
    // the flow. RFC 6749 §4.1.2.1 requires the authorization server to echo
    // `state` on the error response too, so an error callback that carries none
    // did not come from Discord finishing our round-trip.
    let Some(state_param) = query.state.as_deref() else {
        return refuse_state(StateError::BadSignature);
    };
    let pending = cookies::read(&headers, cookies::PENDING_COOKIE);
    let accepted = match state_token::accept(
        &oauth.signing_key,
        state_param,
        pending.as_deref(),
        state_token::now_secs(),
    ) {
        Ok(accepted) => accepted,
        Err(error) => return refuse_state(error),
    };

    // Only NOW is the pending cookie dropped, and that ordering is the whole
    // point of this arrangement.
    //
    // It still makes a callback single-use — every path from here on carries
    // this header, so a second presentation of the same `code`+`state` finds no
    // cookie and is refused by the first check in `state_token::accept`.
    //
    // What it no longer does is let a stranger cancel someone else's sign-in.
    // Clearing before verification meant ANY unverifiable callback wiped an
    // in-flight login: `?error=x`, `?state=<garbage>`, or no parameters at all.
    // `SameSite=Lax` sends this cookie on a top-level GET navigation, which is
    // exactly what a third-party page can cause, so a victim part-way through
    // signing in could be knocked back to `invalid_state` by any page they
    // visited. Nothing was disclosed and no session could be issued — it was a
    // denial of login rather than a break — but it was free to perform and
    // invisible to the victim, who would see only that signing in "did not
    // work". A refusal now leaves the cookie alone: it is short-lived, it is
    // useless without the matching `state`, and the legitimate flow it belongs
    // to can still complete.
    let drop_pending = cookies::clear(cookies::PENDING_COOKIE, cookies::PENDING_PATH);

    // Discord refused. WHICH refusal decides both what the visitor is told and
    // whether anyone is told at all.
    if let Some(error) = query.error.as_deref() {
        if error == ERROR_ACCESS_DENIED {
            // The visitor said no at the consent screen. Their choice, not a
            // fault: plain text on the page, the button still there, nothing
            // logged. A `warn` per cancellation would be noise proportional to
            // how many people change their mind.
            return redirect(
                &format!("{PORTAL_HOME}{CANCELLED_QUERY}"),
                vec![drop_pending],
            );
        }

        // Everything else is a failure somebody has to know about, and the one
        // that matters most is `invalid_scope`: it is what Discord returns when
        // the Developer Portal registration has drifted from `discord::SCOPE` —
        // the very drift the token-response scope check exists to catch, taking
        // a door that check never sees. Reported as "cancelled" and unlogged,
        // it presents as every visitor changing their mind forever, with
        // nothing in CloudWatch to contradict that reading.
        return refuse_oauth_error(error, drop_pending);
    }

    let Some(code) = query.code.as_deref() else {
        // Verified state, but neither `code` nor `error`.
        //
        // A `400`, not the `502` this used to answer. The distinction is not
        // pedantry about status codes: these routes are keyless and throttled
        // at 10 req/s, so anyone can call `/auth/login`, take the `state` it
        // hands them, and replay it here to manufacture 5xx at will. That
        // pollutes the alarm surface [0204] is building and makes a real
        // Discord outage indistinguishable from a script. Discord is not
        // unavailable here; the caller sent a malformed request.
        return refuse_query("callback carried neither `code` nor `error`", drop_pending);
    };

    // One action exists, and it is still matched rather than assumed. When
    // [0189] adds `issue`, the arm it needs is a new `match` line and not a
    // restructuring of this handler.
    match accepted.action {
        Action::SignIn => {}
        // Compiled only into the test build, and unreachable even there:
        // `Action::parse` never yields `TestOther`, so `/auth/login` cannot mint
        // a round-trip for it. Refused rather than `unreachable!()` — a panic in
        // a public handler is a worse answer than a `400`, whatever the
        // reasoning says about how it got here.
        // `state` has already verified by this point, so the pending cookie is
        // spent and goes with the refusal — the same rule every branch below
        // follows. A `400` for the same reason the missing-`code` branch is
        // one: nothing upstream failed.
        #[cfg(test)]
        Action::TestOther => {
            return refuse_query("action not implemented by this build", drop_pending);
        }
    }

    let token = match discord::exchange_code(
        &state.http,
        &state.endpoints,
        oauth,
        code,
        &accepted.verifier,
    )
    .await
    {
        Ok(token) => token,
        Err(error) => return refuse_discord("token exchange", error, drop_pending),
    };

    // `token` is moved here, so from this line on the handler cannot reach it.
    let user = match discord::current_user(&state.http, &state.endpoints, token).await {
        Ok(user) => user,
        Err(error) => return refuse_discord("identity read", error, drop_pending),
    };

    let session = Session::issue(&user.id, &user.username, state_token::now_secs());
    redirect(
        PORTAL_HOME,
        vec![
            drop_pending,
            cookies::set(
                cookies::SESSION_COOKIE,
                &session.encode(&oauth.signing_key),
                cookies::SESSION_PATH,
                session::SESSION_TTL_SECS,
            ),
        ],
    )
}

/// What `/auth/me` answers.
///
/// Hand-written rather than derived from the OpenAPI document, because the
/// portal's routes are deliberately absent from it —
/// `tools/scripts/verify-openapi-routes.mjs` fails CI if one appears. The
/// counterpart type is `PortalSession` in `web/portal/src/api/portal.ts`.
#[derive(Debug, Serialize)]
struct MeResponse {
    /// Whether a valid, unexpired session cookie was presented.
    authenticated: bool,
    /// The Discord user ID. Absent when signed out.
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
    /// The Discord username, for display. Absent when signed out.
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
}

/// Report the caller's session.
///
/// `200` with `authenticated: false` rather than `401`, deliberately. This is
/// the question "am I signed in?", and refusing to answer it while signed out is
/// as circular as [0183]'s `/config` refusing to say the portal is closed. The
/// page asks it on every load and renders plain text either way.
async fn me(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let signed_out = MeResponse {
        authenticated: false,
        user_id: None,
        username: None,
    };

    let Some(oauth) = state.oauth.as_ref() else {
        // Not `unconfigured()`: a deployment with no credentials has no
        // sessions, which is a truthful answer to this question and lets the
        // page render rather than showing an error it can do nothing about.
        return no_store(Json(signed_out).into_response());
    };

    let session = cookies::read(&headers, cookies::SESSION_COOKIE)
        .and_then(|cookie| Session::decode(&oauth.signing_key, &cookie, state_token::now_secs()));

    let body = match session {
        Some(session) => MeResponse {
            authenticated: true,
            user_id: Some(session.sub),
            username: Some(session.name),
        },
        None => signed_out,
    };
    no_store(Json(body).into_response())
}

/// Clear the session.
///
/// `POST`, not `GET`: a `GET` sign-out is triggerable by any `<img>` on any page
/// the visitor loads. `SameSite=Lax` would not stop that, because it permits
/// top-level navigation. The mapped verbs at the gateway already include `POST`
/// (`PORTAL_API_METHODS`).
///
/// No CSRF token beyond that. The worst outcome of a forged sign-out is that the
/// visitor is signed out, which they can undo with one click — the standard
/// reasoning, stated here because the next route to be added on this prefix
/// ([0187]'s key issue) must not inherit it.
async fn logout() -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        cookies::clear(cookies::SESSION_COOKIE, cookies::SESSION_PATH)
            .parse()
            .expect("a cleared cookie is a valid header value"),
    );
    no_store(response)
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

/// A `303 See Other` to `location`, carrying `cookies` and never cached.
fn redirect(location: &str, set_cookies: Vec<String>) -> Response {
    // `parse` rather than `expect`. Every `Location` this module builds *should*
    // be header-safe — literals, or `form_urlencoded` output — but one of them
    // is not built from literals alone: the authorize URL starts with
    // `Endpoints::authorize_url`. A value carrying a newline or a control byte
    // made this panic, and a panic here is worse than a 500: the task dies, no
    // response is written, and the caller sees a dropped connection (`curl`
    // reports `000`). On Lambda that is an invocation error and a `Errors`
    // metric data point, for what is a configuration fault.
    let Ok(location) = location.parse() else {
        tracing::error!("portal sign-in redirect target is not a valid header value");
        return no_store(errors::internal_error(
            SIGN_IN_MISCONFIGURED,
            "sign-in is misconfigured on this deployment",
        ));
    };

    let mut response = StatusCode::SEE_OTHER.into_response();
    let headers = response.headers_mut();
    headers.insert(LOCATION, location);
    for cookie in set_cookies {
        headers.append(
            SET_COOKIE,
            cookie
                .parse()
                .expect("cookies here are base64url and literals"),
        );
    }
    no_store(response)
}

/// `no-store` on every response this module produces.
///
/// Three of the four routes carry or set a credential and the fourth is a
/// redirect that does. CloudFront is already told not to cache this prefix
/// (`CACHING_DISABLED` on the API behaviour) and the gateway is told the same
/// (`portalSettings`), but those are two configurations that can drift; this is
/// the one the handler controls, and it is also what stops a *browser* holding
/// `/auth/me` from before a sign-out.
fn no_store(mut response: Response) -> Response {
    cache_control::attach(&mut response, cache_control::NO_STORE);
    response
}

/// `503` for a deployment that reached these routes with no credentials.
///
/// Only reachable if `PORTAL_ENABLED` is true and the secret is missing — the
/// load in `AppConfig::load_portal_oauth` fails hard on exactly that
/// combination, so this is a second line rather than the first.
fn unconfigured() -> Response {
    no_store(errors::service_unavailable(
        SIGN_IN_UNCONFIGURED,
        "sign-in is not configured on this deployment",
    ))
}

/// Refuse a callback whose `state` did not check out.
///
/// `400` with one message for all five reasons. `tracing` records which check
/// fired, at `warn`, because a rise in one of them is the signal that someone is
/// probing — but the caller is told only that it was rejected.
///
/// **Deliberately does not touch the pending-login cookie.** A refusal here
/// means this callback could not be shown to belong to the browser that sent
/// it, so acting on that browser's state is precisely what must not happen —
/// see the ordering note in `callback`.
fn refuse_state(error: StateError) -> Response {
    tracing::warn!(reason = ?error, "portal sign-in callback rejected");
    no_store(errors::bad_request(
        INVALID_STATE,
        "this sign-in could not be verified; start again from the portal",
    ))
}

/// Land a Discord OAuth error that is **not** the visitor declining.
///
/// Split out of `callback` so the logging is reachable synchronously from a
/// test. That is not incidental: the first version of this fix logged from
/// inside the handler, and deleting the `tracing::warn!` left the whole suite
/// green — the same shape of gap that let "every error is a cancellation" ship
/// in the first place. A behaviour nothing can observe is a behaviour nothing
/// protects.
fn refuse_oauth_error(error: &str, drop_pending: String) -> Response {
    tracing::warn!(
        error = %sanitise_error(error),
        "portal sign-in refused by Discord"
    );
    redirect(&format!("{PORTAL_HOME}{FAILED_QUERY}"), vec![drop_pending])
}

/// Refuse a callback that is malformed on the caller's side.
///
/// `400` with `INVALID_QUERY`, and the pending cookie still goes — `state` has
/// verified by every call site, so the cookie is spent whatever happens next.
/// Kept distinct from [`refuse_discord`] so that a 5xx on these routes means
/// what it says: something upstream failed. See the missing-`code` branch in
/// `callback`.
fn refuse_query(reason: &str, drop_pending: String) -> Response {
    tracing::warn!(reason, "portal sign-in callback was malformed");
    let mut response = errors::bad_request(
        errors::INVALID_QUERY,
        "this sign-in could not be completed; start again from the portal",
    );
    response.headers_mut().append(
        SET_COOKIE,
        drop_pending
            .parse()
            .expect("a cleared cookie is a valid header value"),
    );
    no_store(response)
}

/// Make Discord's `error` value safe to put in a log line.
///
/// Two hazards, both because the value arrives in a query string on a public,
/// keyless route rather than from the seven codes RFC 6749 documents:
///
/// - **Length.** Truncated to [`ERROR_LOG_MAX`]. API Gateway caps a query
///   string long before this matters, but the bound belongs next to the value
///   it bounds rather than in a service quota somebody has to remember.
/// - **Control characters.** `tracing`'s JSON layer escapes them, so this is
///   defence in depth rather than the only guard — but the plain-text layer
///   `serve` uses does not, and a newline in a log line is how one event
///   becomes two.
///
/// Anything outside printable ASCII becomes `.`, which keeps the result
/// recognisable when it *is* one of the documented codes and obviously
/// mangled when it is not.
fn sanitise_error(error: &str) -> String {
    error
        .chars()
        .take(ERROR_LOG_MAX)
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '.'
            }
        })
        .collect()
}

/// Refuse a callback that Discord did not complete.
///
/// `502`, not `500`: the failure is upstream, and saying so is what stops an
/// operator looking for a bug in this handler when Discord is having an
/// incident. The visitor's own message says nothing about which call failed.
fn refuse_discord(stage: &str, error: discord::DiscordError, drop_pending: String) -> Response {
    tracing::warn!(stage, error = %error, "portal sign-in could not complete");
    let mut response = (
        StatusCode::BAD_GATEWAY,
        Json(errors::ErrorEnvelope {
            code: DISCORD_UNAVAILABLE,
            message: "could not complete sign-in with Discord; try again".into(),
            details: None,
        }),
    )
        .into_response();
    response.headers_mut().append(
        SET_COOKIE,
        drop_pending
            .parse()
            .expect("a cleared cookie is a valid header value"),
    );
    no_store(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A `tracing` writer that keeps what was written, so a test can assert on
    /// log output instead of on the presence of a macro call.
    #[derive(Clone, Default)]
    struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

    impl CapturedLogs {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).to_string()
        }
    }

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLogs {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn oauth() -> OauthSecret {
        OauthSecret::parse(
            &serde_json::json!({
                "client_id": "client-1",
                "client_secret": "shh",
                "redirect_uri": "https://portal.example/api-tokens/api/auth/callback",
                "session_signing_key": "0123456789abcdef0123456789abcdef0123456789abcdef",
            })
            .to_string(),
        )
        .unwrap()
    }

    /// The URL a visitor is sent to is the one place the scope, the PKCE method
    /// and the registered redirect URI all have to be right at once — and the
    /// only place a mistake shows up on Discord's error page rather than in our
    /// logs.
    #[test]
    fn the_authorize_url_asks_for_identify_with_s256_pkce() {
        let secret = oauth();
        let started = state_token::start(&secret.signing_key, Action::SignIn, 1_800_000_000);
        let url = authorize_url(&discord::Endpoints::default(), &secret, &started);

        assert!(url.starts_with("https://discord.com/oauth2/authorize?"));
        let query: Vec<(String, String)> =
            form_urlencoded::parse(url.split_once('?').unwrap().1.as_bytes())
                .into_owned()
                .collect();
        let get = |key: &str| {
            query
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
                .unwrap_or_default()
                .to_string()
        };

        assert_eq!(get("response_type"), "code");
        assert_eq!(get("client_id"), "client-1");
        assert_eq!(
            get("redirect_uri"),
            "https://portal.example/api-tokens/api/auth/callback"
        );
        // Exactly `identify` — not a superset, not a second scope.
        assert_eq!(get("scope"), "identify");
        assert_eq!(get("code_challenge_method"), "S256");
        assert_eq!(get("code_challenge"), started.code_challenge);
        assert_eq!(get("state"), started.state_param);

        // The verifier is the half that must NOT travel. A URL containing it
        // makes PKCE decorative and is the single worst thing this function
        // could do.
        assert!(!url.contains("code_verifier"));

        // …and the check above is on the parameter NAME, which is not enough.
        // Swapping `state_param` for `pending_cookie` in `state_token::start`
        // would put the verifier's VALUE in the query string under the name
        // `state`, and every existing assertion here would still pass: the URL
        // would carry a well-formed signed token, just the wrong one. Assert on
        // the value.
        let pending_payload = crypto::verify(
            &secret.signing_key,
            crypto::CTX_PENDING,
            &started.pending_cookie,
        )
        .expect("the pending cookie must verify under its own context");
        let verifier =
            serde_json::from_slice::<serde_json::Value>(&pending_payload).unwrap()["verifier"]
                .as_str()
                .expect("the pending cookie carries the verifier")
                .to_string();
        assert!(!verifier.is_empty());
        assert!(
            !url.contains(&verifier),
            "the PKCE verifier reached the authorize URL — PKCE is decorative"
        );
        assert!(!url.contains(&started.pending_cookie));
        // Nor may the client secret, which is the other thing on `oauth()`.
        assert!(!url.contains("shh"));
    }

    /// Percent-encoding, asserted because the failure is remote: a raw `:` or
    /// `/` in `redirect_uri` is what makes Discord answer "invalid redirect_uri"
    /// on its own page, with nothing in our logs.
    #[test]
    fn the_authorize_url_encodes_its_parameters() {
        let secret = oauth();
        let started = state_token::start(&secret.signing_key, Action::SignIn, 0);
        let url = authorize_url(&discord::Endpoints::default(), &secret, &started);
        assert!(url.contains("redirect_uri=https%3A%2F%2Fportal.example"));
    }

    /// Every response out of this module is uncacheable, and the redirect that
    /// carries the session cookie most of all.
    #[test]
    fn redirects_are_never_cached_and_carry_their_cookies() {
        let response = redirect(
            PORTAL_HOME,
            vec!["a=1; Path=/".into(), "b=2; Path=/".into()],
        );
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get(LOCATION).unwrap(), "/api-tokens/");
        assert_eq!(response.headers().get_all(SET_COOKIE).iter().count(), 2);
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    }

    /// `redirect` must not panic on a `Location` it cannot use.
    ///
    /// The authorize URL is the one target not built from literals alone — it
    /// starts with `Endpoints::authorize_url` — and a value carrying a newline
    /// used to panic the task, so the caller got a dropped connection rather
    /// than any status at all.
    #[test]
    fn an_unusable_redirect_target_is_an_error_not_a_panic() {
        let response = redirect("https://example.test/ok\r\nX-Injected: yes", vec![]);
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(response.headers().get(LOCATION).is_none());
        assert!(response.headers().get("x-injected").is_none());
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    }

    /// A Discord failure **is logged**, and a cancellation is **not**.
    ///
    /// Asserted against captured output rather than by reading the source,
    /// because "there is a `warn!` here" is exactly the kind of claim that
    /// survives its own deletion. The silence half matters too: a `warn` per
    /// cancellation would be noise proportional to how many people change their
    /// mind, and would bury the failures this exists to surface.
    #[test]
    fn a_discord_failure_is_logged_and_a_cancellation_is_not() {
        let logs = CapturedLogs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            refuse_oauth_error("invalid_scope", String::new());
        });

        let text = logs.text();
        assert!(
            text.contains("refused by Discord"),
            "a non-cancellation OAuth error must reach the log: {text:?}"
        );
        assert!(
            text.contains("invalid_scope"),
            "the log must name which error it was: {text:?}"
        );

        // And the cancellation path, which goes nowhere near this function,
        // writes nothing at all.
        let quiet = CapturedLogs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(quiet.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            redirect(&format!("{PORTAL_HOME}{CANCELLED_QUERY}"), vec![]);
        });
        assert!(quiet.text().is_empty(), "a cancellation must be silent");
    }

    /// Discord's `error` reaches a log line, and it arrives in a query string
    /// on a public keyless route — so it is bounded and stripped, not trusted
    /// to be one of RFC 6749's seven codes.
    #[test]
    fn the_logged_error_value_is_truncated_and_stripped() {
        assert_eq!(sanitise_error("invalid_scope"), "invalid_scope");

        // A newline in a plain-text log line turns one event into two.
        assert_eq!(sanitise_error("evil\r\nWARN faked"), "evil..WARN faked");
        assert!(!sanitise_error("a\nb").contains('\n'));
        assert!(!sanitise_error("a\u{0}b").contains('\u{0}'));

        let long = "x".repeat(10_000);
        assert_eq!(sanitise_error(&long).len(), ERROR_LOG_MAX);
    }

    /// The two landing states are literals and they are different. Collapsing
    /// them is the defect this pair exists to prevent.
    #[test]
    fn the_two_landing_states_are_distinct_literals() {
        assert_eq!(CANCELLED_QUERY, "?signin=cancelled");
        assert_eq!(FAILED_QUERY, "?signin=failed");
        assert_ne!(CANCELLED_QUERY, FAILED_QUERY);
        assert_eq!(ERROR_ACCESS_DENIED, "access_denied");
        for query in [CANCELLED_QUERY, FAILED_QUERY] {
            assert!(format!("{PORTAL_HOME}{query}").starts_with("/api-tokens/?"));
        }
    }

    /// The redirect target is a literal in every branch. An `assert` rather than
    /// a comment, so a later slice that adds a `redirect_to` parameter has to
    /// delete this to do it.
    #[test]
    fn the_only_redirect_targets_are_the_portal_itself() {
        assert_eq!(PORTAL_HOME, "/api-tokens/");
        assert!(PORTAL_HOME.starts_with('/'));
        assert!(!PORTAL_HOME.starts_with("//"));
        assert!(format!("{PORTAL_HOME}{CANCELLED_QUERY}").starts_with("/api-tokens/?"));
    }

    /// The registered redirect URI and the route that serves it are one string,
    /// checked in both directions: `secret.rs` refuses a URI that does not end
    /// in `CALLBACK_PATH`, and this pins `CALLBACK_PATH` under the gated prefix.
    #[test]
    fn every_route_sits_under_the_gated_portal_prefix() {
        for path in [LOGIN_PATH, CALLBACK_PATH, ME_PATH, LOGOUT_PATH] {
            assert!(
                path.starts_with(super::super::PORTAL_API_PREFIX),
                "{path} would not be gated by PORTAL_ENABLED"
            );
        }
    }
}
