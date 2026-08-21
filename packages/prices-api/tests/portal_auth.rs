//! The Discord sign-in round-trip, over HTTP, end to end (task 0186).
//!
//! The unit tests next to each module cover the pieces — signatures, cookies,
//! the state pair, the session. This file covers the thing none of them can: the
//! four routes wired into the real [`prices_api::app`] router, exchanging a real
//! authorization code against a **mock Discord** bound to loopback, using the
//! same `reqwest` client production uses.
//!
//! # Why a mock server rather than a trait
//!
//! Injecting a `DiscordClient` trait would let the tests skip the HTTP layer
//! entirely — and the HTTP layer is where three of the requirements live: that
//! the client secret goes in the form body and not the URL, that the PKCE
//! verifier sent is the one the challenge was derived from, and that the scope
//! Discord reports back is checked. A fake that satisfies a trait proves none of
//! them. The mock records what it actually received, and the assertions are
//! against that.
//!
//! The mock is [`MockDiscord`] (`tests/common/mock_discord.rs`, shared with
//! the issue round-trip suite): an axum app on an ephemeral port, serving
//! `/oauth2/token`, `/users/@me` and the member route.

use axum::Router;
use axum::http::{HeaderMap, StatusCode, header};
use prices_api::portal::auth::{
    CALLBACK_PATH, LOGIN_PATH, LOGOUT_PATH, ME_PATH, cookies, crypto, discord::Endpoints,
    secret::OauthSecret, session::Session, state_token,
};
use prices_api::{AppConfig, AppState, app};
use serde_json::{Value, json};
use tower::ServiceExt;

#[path = "common/mock_discord.rs"]
mod mock_discord;
use mock_discord::{GRANTED_SCOPE, MockDiscord};

// ---------------------------------------------------------------------------
// Router under test
// ---------------------------------------------------------------------------

const SIGNING_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const REDIRECT_URI: &str = "https://portal.example/api-tokens/api/auth/callback";

fn oauth_secret() -> OauthSecret {
    OauthSecret::parse(
        &json!({
            "client_id": "a-client-id",
            "client_secret": "the-client-secret",
            "redirect_uri": REDIRECT_URI,
            "session_signing_key": SIGNING_KEY,
        })
        .to_string(),
    )
    .expect("the test bundle must be valid")
}

/// A router with the portal open and sign-in configured, pointed at Discord's
/// real endpoints.
///
/// Only for tests that never reach the token exchange — anything that completes
/// a round-trip wants [`app_against`].
fn signed_in_app(portal_enabled: bool) -> Router {
    build_app(portal_enabled, Endpoints::default())
}

/// A router pointed at `mock`.
///
/// **No environment mutation, and no lock.** An earlier version set
/// `DISCORD_API_BASE` under a `Mutex` and relied on `Endpoints::from_env`
/// running once per router — but that read happened inside `app()`, on every
/// construction, including the fifteen in this file that never took the lock.
/// libtest runs these on parallel threads, so a reader could be walking
/// `environ` while `setenv` reallocated it: the undefined behaviour that makes
/// `set_var` `unsafe` in edition 2024, and in practice an intermittent segfault
/// that takes the whole binary down rather than failing one test.
///
/// The endpoints now travel on `AppConfig`, so each router is handed its own
/// and nothing is shared. That also closes the quieter half of the same
/// problem: a router built by `signed_in_app` used to capture whatever base URL
/// another test had last written.
fn app_against(mock: &MockDiscord) -> Router {
    build_app(
        true,
        Endpoints {
            api_base: mock.base.clone(),
            ..Endpoints::default()
        },
    )
}

fn build_app(portal_enabled: bool, endpoints: Endpoints) -> Router {
    let config = AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![],
        portal_enabled,
        portal_oauth: portal_enabled.then(oauth_secret),
        portal_endpoints: endpoints,
        // Task 0187: the control-plane client for self-service keys. `None`
        // is what every non-portal test wants — with no client in the
        // config there is no code path here that can reach API Gateway.
        portal_keys: None,
        portal_eligibility: None,
    };
    app(&config, AppState::without_ch())
}

// ---------------------------------------------------------------------------
// Request helpers
// ---------------------------------------------------------------------------

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn location(&self) -> String {
        self.headers
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default()
    }

    fn set_cookies(&self) -> Vec<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect()
    }

    /// The value a browser would store for `name`, or `None` if this response
    /// clears it (`Max-Age=0`) or never set it.
    fn cookie(&self, name: &str) -> Option<String> {
        self.set_cookies()
            .into_iter()
            .filter(|c| c.starts_with(&format!("{name}=")))
            .filter(|c| !c.contains("Max-Age=0"))
            .map(|c| {
                c.split_once('=')
                    .unwrap()
                    .1
                    .split(';')
                    .next()
                    .unwrap()
                    .to_string()
            })
            .next()
    }

    fn clears(&self, name: &str) -> bool {
        self.set_cookies()
            .iter()
            .any(|c| c.starts_with(&format!("{name}=")) && c.contains("Max-Age=0"))
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("the body should be JSON")
    }
}

async fn send(router: &Router, method: &str, uri: &str, cookies: &[(&str, &str)]) -> Reply {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if !cookies.is_empty() {
        let header = cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        builder = builder.header(header::COOKIE, header);
    }
    let response = router
        .clone()
        .oneshot(builder.body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Reply {
        status,
        headers,
        body,
    }
}

async fn fetch(router: &Router, uri: &str, cookies: &[(&str, &str)]) -> Reply {
    send(router, "GET", uri, cookies).await
}

/// What one `/auth/login` handed out: the `state` Discord will echo back, the
/// cookie the browser now holds, and the PKCE challenge that went to Discord.
struct StartedLogin {
    state: String,
    pending: String,
    challenge: String,
}

/// Drive `/auth/login` and pull those three values out of the response.
async fn start_login(router: &Router) -> StartedLogin {
    let reply = fetch(router, LOGIN_PATH, &[]).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "login must redirect");
    let pending = reply
        .cookie(cookies::PENDING_COOKIE)
        .expect("login must set the pending-login cookie");
    let query = reply.location().split_once('?').unwrap().1.to_string();
    let params: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    let param = |name: &str| {
        params
            .iter()
            .find(|(k, _)| k == name)
            .unwrap_or_else(|| panic!("the authorize URL must carry `{name}`"))
            .1
            .clone()
    };
    StartedLogin {
        state: param("state"),
        pending,
        challenge: param("code_challenge"),
    }
}

// ---------------------------------------------------------------------------
// The gate — this slice's routes must be invisible while the portal is closed
// ---------------------------------------------------------------------------

/// The first acceptance criterion. All four routes, both verbs, byte-identical
/// to a path that was never deployed — task 0183's gate covers them by prefix,
/// and this is the assertion that it really does.
#[tokio::test]
async fn every_sign_in_route_is_an_empty_404_while_the_portal_is_closed() {
    let closed = signed_in_app(false);
    let (absent_status, absent_body) = {
        let reply = fetch(&closed, "/no-such-route-anywhere", &[]).await;
        (reply.status, reply.body)
    };

    for (method, path) in [
        ("GET", LOGIN_PATH),
        ("GET", CALLBACK_PATH),
        ("GET", ME_PATH),
        ("POST", LOGOUT_PATH),
    ] {
        let reply = send(&closed, method, path, &[]).await;
        assert_eq!(reply.status, absent_status, "{method} {path}");
        assert_eq!(reply.body, absent_body, "{method} {path}");
        assert!(reply.body.is_empty(), "{method} {path} carried a body");
        assert!(
            reply.set_cookies().is_empty(),
            "{method} {path} set a cookie while the portal was closed"
        );
    }
}

/// **A rejection that happens before the handler must not outrank the gate.**
///
/// `ValidatedQuery` refuses a query string axum cannot deserialize, and it does
/// so in an extractor — which runs after routing. Task 0183's gate is a layer,
/// so it runs first, and that ordering is the only reason a malformed query on
/// a closed portal still answers an empty `404` rather than a `400` naming
/// `invalid_query`. The latter would say "this route exists and parses input",
/// which is exactly the disclosure the gate is built to prevent.
///
/// Nothing asserted this until the extractor was adopted, and nothing about
/// `ValidatedQuery`'s own contract guarantees it — it is a property of how the
/// two are composed in `portal::apply`.
#[tokio::test]
async fn a_malformed_query_does_not_out_rank_the_closed_portal_gate() {
    let closed = signed_in_app(false);
    let absent = fetch(&closed, "/no-such-route-anywhere", &[]).await;

    for uri in [
        format!("{CALLBACK_PATH}?code=a&code=b"),
        format!("{LOGIN_PATH}?action=a&action=b"),
        format!("{CALLBACK_PATH}?state=a&state=b"),
    ] {
        let reply = fetch(&closed, &uri, &[]).await;
        assert_eq!(reply.status, absent.status, "{uri}");
        assert_eq!(reply.body, absent.body, "{uri}");
        assert!(reply.body.is_empty(), "{uri} carried a body");
    }
}

/// A closed portal must not even hint at the flow by answering a callback
/// differently depending on its query string.
#[tokio::test]
async fn a_closed_portal_answers_a_well_formed_callback_the_same_as_a_bare_one() {
    let closed = signed_in_app(false);
    let bare = fetch(&closed, CALLBACK_PATH, &[]).await;
    let dressed = fetch(&closed, &format!("{CALLBACK_PATH}?code=abc&state=def"), &[]).await;
    assert_eq!(bare.status, dressed.status);
    assert_eq!(bare.body, dressed.body);
}

// ---------------------------------------------------------------------------
// /auth/login
// ---------------------------------------------------------------------------

#[tokio::test]
async fn login_redirects_to_discord_asking_for_the_two_scopes_and_nothing_else() {
    let open = signed_in_app(true);
    let reply = fetch(&open, LOGIN_PATH, &[]).await;

    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    let location = reply.location();
    assert!(
        location.starts_with("https://discord.com/oauth2/authorize?"),
        "{location}"
    );

    let query: Vec<(String, String)> =
        form_urlencoded::parse(location.split_once('?').unwrap().1.as_bytes())
            .into_owned()
            .collect();
    let field = |name: &str| {
        query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };

    // The pair, verbatim (task 0189) — and never `guilds` or `email`, which
    // ADR 0010 refuses outright.
    assert_eq!(field("scope"), "identify guilds.members.read");
    assert_eq!(field("response_type"), "code");
    assert_eq!(field("code_challenge_method"), "S256");
    // And NO `prompt` on a sign-in (task 0189). Suppressing the consent
    // screen is for the re-authorisation round-trips, which by construction
    // follow an authorisation that already exists; sign-in is where the FIRST
    // one happens — including the re-consent that grants 0189's new
    // `guilds.members.read` to an account that authorised under 0186's
    // narrower scope. See `authorize_url`.
    assert!(
        !query.iter().any(|(k, _)| k == "prompt"),
        "sign-in must not suppress the consent screen: {location}"
    );
    assert_eq!(field("redirect_uri"), REDIRECT_URI);
    assert!(!field("state").is_empty());
    assert!(!field("code_challenge").is_empty());

    // Neither secret may appear in a URL the visitor's browser will hold, log,
    // and send as a `Referer`.
    assert!(!location.contains("the-client-secret"), "{location}");
    assert!(!location.contains("code_verifier"), "{location}");
    assert!(!location.contains(SIGNING_KEY), "{location}");
}

#[tokio::test]
async fn login_sets_a_short_lived_httponly_secure_lax_pending_cookie() {
    let open = signed_in_app(true);
    let reply = fetch(&open, LOGIN_PATH, &[]).await;
    let cookie = reply
        .set_cookies()
        .into_iter()
        .find(|c| c.starts_with(cookies::PENDING_COOKIE))
        .expect("login must set a pending cookie");

    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(cookie.contains("Secure"), "{cookie}");
    assert!(cookie.contains("SameSite=Lax"), "{cookie}");
    assert!(
        cookie.contains(&format!("Max-Age={}", state_token::PENDING_TTL_SECS)),
        "{cookie}"
    );
    assert!(cookie.contains("Path=/api-tokens/api/auth/"), "{cookie}");
    assert_eq!(
        reply.headers.get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
}

#[tokio::test]
async fn two_logins_produce_two_different_states() {
    let open = signed_in_app(true);
    let first = start_login(&open).await;
    let second = start_login(&open).await;
    assert_ne!(
        first.state, second.state,
        "the state must not be predictable"
    );
    assert_ne!(first.pending, second.pending);
    assert_ne!(
        first.challenge, second.challenge,
        "each login needs its own PKCE verifier"
    );
}

/// The action slot is verified at the door as well as in the signature, so an
/// action this build does not implement — 0191's `rework`, arriving early —
/// cannot start a round-trip that the callback would then have to decide what
/// to do with.
#[tokio::test]
async fn login_refuses_an_action_it_does_not_implement() {
    let open = signed_in_app(true);
    assert_eq!(
        fetch(&open, &format!("{LOGIN_PATH}?action=signin"), &[])
            .await
            .status,
        StatusCode::SEE_OTHER
    );
    let refused = fetch(&open, &format!("{LOGIN_PATH}?action=rework"), &[]).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(refused.set_cookies().is_empty());
}

/// `action=issue` IS implemented (task 0189) — but on a deployment with no
/// control plane or eligibility parameters wired it is refused before the
/// visitor is sent to Discord: a round-trip that can only ever end in
/// "failed" must not start.
///
/// **Refused with a landing, not with a `503` envelope.** This route is
/// reached by a top-level navigation from a link on the dashboard, so a JSON
/// body strands the visitor on an API URL with nothing to read and nothing to
/// press. `?issue=failed` is the state that already means "our key service,
/// not your eligibility" — and no round-trip is started, so no pending cookie
/// is minted either.
#[tokio::test]
async fn login_lands_an_unwired_issue_round_trip_on_failed() {
    let open = signed_in_app(true);
    let refused = fetch(&open, &format!("{LOGIN_PATH}?action=issue"), &[]).await;
    assert_eq!(refused.status, StatusCode::SEE_OTHER);
    assert_eq!(refused.location(), "/api-tokens/?issue=failed");
    assert!(refused.set_cookies().is_empty());
    // And not a body the browser would render as text.
    assert!(refused.body.is_empty(), "{:?}", refused.body);
}

/// The same door on a deployment with **no OAuth credentials at all**: a
/// sign-in still gets the `503` envelope its caller can read, while an issue
/// navigation gets a landing. The two answers differ because the two callers
/// do — one is `fetch` from the page, the other is the browser itself.
#[tokio::test]
async fn an_issue_round_trip_with_no_credentials_lands_rather_than_503ing() {
    let config = AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![],
        portal_enabled: true,
        portal_oauth: None,
        portal_endpoints: Endpoints::default(),
        portal_keys: None,
        portal_eligibility: None,
    };
    let router = app(&config, AppState::without_ch());

    let signin = fetch(&router, LOGIN_PATH, &[]).await;
    assert_eq!(signin.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(signin.json()["code"], "sign_in_unconfigured");

    let issue = fetch(&router, &format!("{LOGIN_PATH}?action=issue"), &[]).await;
    assert_eq!(issue.status, StatusCode::SEE_OTHER);
    assert_eq!(issue.location(), "/api-tokens/?issue=failed");
}

// ---------------------------------------------------------------------------
// /auth/callback — the happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_complete_round_trip_signs_the_visitor_in() {
    let mock = MockDiscord::start(GRANTED_SCOPE, None).await;
    let open = app_against(&mock);

    let started = start_login(&open).await;
    let (state, pending) = (&started.state, &started.pending);
    let reply = fetch(
        &open,
        &format!("{CALLBACK_PATH}?code=an-auth-code&state={state}"),
        &[(cookies::PENDING_COOKIE, pending)],
    )
    .await;

    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(
        reply.location(),
        "/api-tokens/",
        "the callback must land on the portal, and on a literal"
    );

    // The session cookie, with all four attributes the criteria name.
    let session_header = reply
        .set_cookies()
        .into_iter()
        .find(|c| c.starts_with(cookies::SESSION_COOKIE))
        .expect("the callback must set a session");
    assert!(session_header.contains("HttpOnly"), "{session_header}");
    assert!(session_header.contains("Secure"), "{session_header}");
    assert!(session_header.contains("SameSite=Lax"), "{session_header}");
    assert!(
        session_header.contains(&format!(
            "Max-Age={}",
            prices_api::portal::auth::session::SESSION_TTL_SECS
        )),
        "{session_header}"
    );
    assert!(
        session_header.contains("Path=/api-tokens/"),
        "{session_header}"
    );

    // And the pending cookie is gone — the replay defence, on the wire.
    assert!(reply.clears(cookies::PENDING_COOKIE));

    // Sign-in checks identity only — the membership route is the ISSUE
    // round-trip's, and a sign-in that consulted it would be re-inventing the
    // session-carried eligibility ADR 0010 §8 forbids.
    assert_eq!(mock.member_calls(), 0);

    // The exchange really happened, with the client secret in the BODY and the
    // PKCE verifier that matches the challenge sent at login.
    assert_eq!(mock.exchanges(), 1);
    assert_eq!(
        mock.form_field("client_secret").as_deref(),
        Some("the-client-secret")
    );
    assert_eq!(mock.form_field("code").as_deref(), Some("an-auth-code"));
    assert_eq!(
        mock.form_field("grant_type").as_deref(),
        Some("authorization_code")
    );
    assert_eq!(
        mock.form_field("redirect_uri").as_deref(),
        Some(REDIRECT_URI)
    );
    // PKCE, closed loop: the verifier sent to the token endpoint is the
    // pre-image of the challenge that went to the authorize endpoint. Without
    // this the two halves could drift and the exchange would still succeed
    // against a server that does not enforce it — which, per task 0156, may well
    // describe Discord.
    let verifier = mock.form_field("code_verifier").expect("PKCE verifier");
    assert_eq!(crypto::pkce_challenge(&verifier), started.challenge);
    assert_ne!(verifier, started.challenge);
}

/// The criterion "the page shows their Discord username and ID", from the
/// backend's side: after the round-trip, `/auth/me` reports both.
#[tokio::test]
async fn me_reports_the_username_and_id_after_a_round_trip() {
    let mock = MockDiscord::start(GRANTED_SCOPE, None).await;
    let open = app_against(&mock);

    let started = start_login(&open).await;
    let (state, pending) = (&started.state, &started.pending);
    let callback = fetch(
        &open,
        &format!("{CALLBACK_PATH}?code=c&state={state}"),
        &[(cookies::PENDING_COOKIE, pending)],
    )
    .await;
    let session = callback.cookie(cookies::SESSION_COOKIE).unwrap();

    let me = fetch(&open, ME_PATH, &[(cookies::SESSION_COOKIE, &session)]).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(
        me.json(),
        json!({
            "authenticated": true,
            "user_id": "308994132968210433",
            "username": "adam",
        })
    );
    assert_eq!(me.headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
}

/// The Bearer token reached `/users/@me` — and then went nowhere. Asserted on
/// the session cookie's decoded contents, because that is the only thing that
/// outlives the request.
#[tokio::test]
async fn no_discord_token_survives_the_callback() {
    let mock = MockDiscord::start(GRANTED_SCOPE, None).await;
    let open = app_against(&mock);

    let started = start_login(&open).await;
    let (state, pending) = (&started.state, &started.pending);
    let callback = fetch(
        &open,
        &format!("{CALLBACK_PATH}?code=c&state={state}"),
        &[(cookies::PENDING_COOKIE, pending)],
    )
    .await;

    assert_eq!(
        mock.recorded.lock().unwrap().bearer.as_deref(),
        Some("Bearer an-access-token"),
        "the identity read must actually authenticate"
    );

    let cookie = callback.cookie(cookies::SESSION_COOKIE).unwrap();
    let session = Session::decode(SIGNING_KEY.as_bytes(), &cookie, state_token::now_secs())
        .expect("the session must verify under the configured key");
    assert_eq!(session.sub, "308994132968210433");
    assert_eq!(session.name, "adam");

    // Nothing from the token response, and nothing from the parts of the user
    // object this service does not want.
    let serialized = serde_json::to_string(&session).unwrap();
    for forbidden in [
        "an-access-token",
        "a-refresh-token",
        "someone@example.com",
        "the-client-secret",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "the session carries `{forbidden}`: {serialized}"
        );
    }
    // Nor does any header on the response.
    let headers = format!("{:?}", callback.headers);
    assert!(!headers.contains("an-access-token"), "{headers}");
    assert!(!headers.contains("the-client-secret"), "{headers}");
}

// ---------------------------------------------------------------------------
// /auth/callback — the refusals
// ---------------------------------------------------------------------------

/// The acceptance criterion, over HTTP: mismatch and replay are both rejected.
/// The unit tests in `state_token.rs` cover which check fires; this covers what
/// the caller sees, and that no session comes out of it.
#[tokio::test]
async fn a_mismatched_state_is_rejected_and_issues_no_session() {
    let mock = MockDiscord::start(GRANTED_SCOPE, None).await;
    let open = app_against(&mock);

    // Two independent logins. Present one browser's cookie with the other's
    // state — the login-CSRF shape.
    let victim_state = start_login(&open).await.state;
    let attacker_cookie = start_login(&open).await.pending;

    let reply = fetch(
        &open,
        &format!("{CALLBACK_PATH}?code=c&state={victim_state}"),
        &[(cookies::PENDING_COOKIE, &attacker_cookie)],
    )
    .await;

    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.json()["code"], "invalid_state");
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_none());
    // And it leaves the browser's pending login alone — see
    // `an_unverifiable_callback_cannot_cancel_someone_elses_sign_in`.
    assert!(!reply.clears(cookies::PENDING_COOKIE));
    assert_eq!(
        mock.exchanges(),
        0,
        "a rejected state must be refused BEFORE the code is exchanged"
    );
}

#[tokio::test]
async fn a_callback_with_no_pending_cookie_is_rejected() {
    let mock = MockDiscord::start(GRANTED_SCOPE, None).await;
    let open = app_against(&mock);

    let state = start_login(&open).await.state;
    let reply = fetch(&open, &format!("{CALLBACK_PATH}?code=c&state={state}"), &[]).await;

    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_none());
    assert_eq!(mock.exchanges(), 0);
}

/// Replay, on the wire. The first callback succeeds and clears the cookie; a
/// browser that obeys `Set-Cookie` cannot send it again, and neither can anyone
/// who captured the URL — the `code` and `state` alone are not enough.
#[tokio::test]
async fn replaying_a_callback_url_after_the_cookie_is_cleared_is_rejected() {
    let mock = MockDiscord::start(GRANTED_SCOPE, None).await;
    let open = app_against(&mock);

    let started = start_login(&open).await;
    let (state, pending) = (&started.state, &started.pending);
    let uri = format!("{CALLBACK_PATH}?code=c&state={state}");

    let first = fetch(&open, &uri, &[(cookies::PENDING_COOKIE, pending)]).await;
    assert_eq!(first.status, StatusCode::SEE_OTHER);
    assert!(first.clears(cookies::PENDING_COOKIE));

    // The replay: same URL, and the browser no longer holds the cookie.
    let second = fetch(&open, &uri, &[]).await;
    assert_eq!(second.status, StatusCode::BAD_REQUEST);
    assert!(second.cookie(cookies::SESSION_COOKIE).is_none());
    assert_eq!(
        mock.exchanges(),
        1,
        "the replay must not reach Discord at all"
    );
}

#[tokio::test]
async fn a_forged_state_signed_with_another_key_is_rejected() {
    let mock = MockDiscord::start(GRANTED_SCOPE, None).await;
    let open = app_against(&mock);

    let pending = start_login(&open).await.pending;
    let forged = state_token::start(
        b"a-key-the-attacker-chose-themselves-0000",
        state_token::Action::SignIn,
        state_token::now_secs(),
    );

    let reply = fetch(
        &open,
        &format!("{CALLBACK_PATH}?code=c&state={}", forged.state_param),
        &[(cookies::PENDING_COOKIE, &pending)],
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(mock.exchanges(), 0);
}

/// A `state` the attacker minted *and* the matching cookie — i.e. they ran the
/// whole flow — still cannot forge a session, because the cookie they hold was
/// signed with their key and not ours.
#[tokio::test]
async fn a_self_signed_pair_is_rejected() {
    let mock = MockDiscord::start(GRANTED_SCOPE, None).await;
    let open = app_against(&mock);

    let forged = state_token::start(
        b"a-key-the-attacker-chose-themselves-0000",
        state_token::Action::SignIn,
        state_token::now_secs(),
    );
    let reply = fetch(
        &open,
        &format!("{CALLBACK_PATH}?code=c&state={}", forged.state_param),
        &[(cookies::PENDING_COOKIE, &forged.pending_cookie)],
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(mock.exchanges(), 0);
}

/// The visitor pressed "Cancel". Not an error — plain text on the page, per the
/// task, and the pending cookie is dropped so the abandoned flow cannot be
/// resumed.
///
/// The `state` is required here as it is everywhere else. RFC 6749 §4.1.2.1
/// makes the authorization server echo it on the error response too, so a
/// cancellation that carries none did not come from Discord finishing our
/// round-trip — see `an_unverifiable_callback_cannot_cancel_someone_elses_sign_in`.
#[tokio::test]
async fn a_cancelled_sign_in_returns_to_the_portal_saying_so() {
    let open = signed_in_app(true);
    let started = start_login(&open).await;

    let reply = fetch(
        &open,
        &format!(
            "{CALLBACK_PATH}?error=access_denied&error_description=nope&state={}",
            started.state
        ),
        &[(cookies::PENDING_COOKIE, &started.pending)],
    )
    .await;

    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location(), "/api-tokens/?signin=cancelled");
    assert!(reply.clears(cookies::PENDING_COOKIE));
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_none());
}

/// **A stranger must not be able to cancel a sign-in they cannot prove is
/// theirs.**
///
/// `SameSite=Lax` sends the pending cookie on a top-level GET navigation, which
/// is exactly what any third-party page can cause. While the handler cleared
/// that cookie before verifying `state`, every shape below knocked a victim
/// part-way through signing in back to `invalid_state` — for free, and
/// invisibly: they would see only that signing in "did not work". No session
/// could be issued and nothing was disclosed, so this was a denial of login
/// rather than a break, but it needed no more than a link.
///
/// The fix is an ordering one: nothing touches the cookie until the callback
/// has been shown to belong to the browser that sent it.
#[tokio::test]
async fn an_unverifiable_callback_cannot_cancel_someone_elses_sign_in() {
    // Needs a working Discord: the point is that the victim's own callback
    // still completes, which means it has to reach the token exchange.
    let mock = MockDiscord::start(GRANTED_SCOPE, None).await;
    let open = app_against(&mock);

    for attack in [
        "error=access_denied",
        "error=access_denied&state=not-a-real-state",
        "code=x&state=not-a-real-state",
        "code=x",
        "state=not-a-real-state",
        "",
    ] {
        let victim = start_login(&open).await;

        // The victim's browser is navigated to the callback by a third party.
        let attacked = fetch(
            &open,
            &format!("{CALLBACK_PATH}?{attack}"),
            &[(cookies::PENDING_COOKIE, &victim.pending)],
        )
        .await;
        assert!(
            !attacked.clears(cookies::PENDING_COOKIE),
            "`?{attack}` cleared a pending login it could not verify"
        );
        assert!(attacked.cookie(cookies::SESSION_COOKIE).is_none());

        // …and the victim's own callback still completes.
        let genuine = fetch(
            &open,
            &format!("{CALLBACK_PATH}?code=c&state={}", victim.state),
            &[(cookies::PENDING_COOKIE, &victim.pending)],
        )
        .await;
        assert_eq!(
            genuine.status,
            StatusCode::SEE_OTHER,
            "`?{attack}` broke the victim's genuine callback"
        );
        assert!(genuine.cookie(cookies::SESSION_COOKIE).is_some());
    }
}

/// The other half of the same rule: once `state` HAS verified, the cookie is
/// spent and goes, whatever the outcome. Without this the replay defence would
/// be gone.
#[tokio::test]
async fn a_verified_callback_always_drops_the_pending_cookie() {
    let mock = MockDiscord::start(GRANTED_SCOPE, None).await;
    let open = app_against(&mock);

    // Success.
    let ok = start_login(&open).await;
    let success = fetch(
        &open,
        &format!("{CALLBACK_PATH}?code=c&state={}", ok.state),
        &[(cookies::PENDING_COOKIE, &ok.pending)],
    )
    .await;
    assert!(success.clears(cookies::PENDING_COOKIE));

    // Verified cancel.
    let cancelled = start_login(&open).await;
    let cancel = fetch(
        &open,
        &format!(
            "{CALLBACK_PATH}?error=access_denied&state={}",
            cancelled.state
        ),
        &[(cookies::PENDING_COOKIE, &cancelled.pending)],
    )
    .await;
    assert!(cancel.clears(cookies::PENDING_COOKIE));

    // Verified, but Discord did not complete.
    let broken = MockDiscord::start(GRANTED_SCOPE, Some(StatusCode::UNAUTHORIZED)).await;
    let broken_app = app_against(&broken);
    let failed = start_login(&broken_app).await;
    let upstream = fetch(
        &broken_app,
        &format!("{CALLBACK_PATH}?code=c&state={}", failed.state),
        &[(cookies::PENDING_COOKIE, &failed.pending)],
    )
    .await;
    assert_eq!(upstream.status, StatusCode::BAD_GATEWAY);
    assert!(upstream.clears(cookies::PENDING_COOKIE));
}

/// A query string the extractor cannot deserialize is answered in the same
/// `ErrorEnvelope` voice as everything else (task 0119).
///
/// Axum's own `Query` rejection is a `text/plain` body, and on these routes
/// that is answered by [0185]'s bundle — whose `getJson` reports a non-JSON
/// body as "the portal backend is unreachable". A caller's own malformed
/// request would present as an outage, which is the reading task 0119's
/// wrappers exist to prevent.
#[tokio::test]
async fn a_malformed_query_is_rejected_in_the_error_envelope_voice() {
    let open = signed_in_app(true);

    for uri in [
        // Duplicate key: `Option<String>` cannot take two values.
        format!("{CALLBACK_PATH}?code=a&code=b"),
        format!("{CALLBACK_PATH}?state=a&state=b"),
        format!("{LOGIN_PATH}?action=a&action=b"),
    ] {
        let reply = fetch(&open, &uri, &[]).await;

        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(
            reply
                .headers
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.starts_with("application/json")),
            Some(true),
            "{uri} answered a non-JSON body, which the bundle reads as an outage"
        );
        assert_eq!(reply.json()["code"], "invalid_query", "{uri}");
        // A rejection before the handler must not touch either cookie.
        assert!(reply.set_cookies().is_empty(), "{uri}");
    }
}

/// **Only `access_denied` is a cancellation.** Every other OAuth error code is a
/// failure, and reporting them all as "the visitor changed their mind" is what
/// hid a drifted scope registration behind a page that looked normal.
///
/// `invalid_scope` is the one that matters: it is Discord's answer when the
/// Developer Portal registration no longer matches `discord::SCOPE`, which is
/// the same drift the token-response check catches — arriving by the one door
/// that check never sees.
#[tokio::test]
async fn only_access_denied_is_reported_as_a_cancellation() {
    let open = signed_in_app(true);

    for (error, expected) in [
        ("access_denied", "/api-tokens/?signin=cancelled"),
        ("invalid_scope", "/api-tokens/?signin=failed"),
        ("server_error", "/api-tokens/?signin=failed"),
        ("temporarily_unavailable", "/api-tokens/?signin=failed"),
        ("invalid_request", "/api-tokens/?signin=failed"),
        ("unauthorized_client", "/api-tokens/?signin=failed"),
        // Not in RFC 6749 §4.1.2.1 at all. Anything unrecognised is a failure,
        // not a cancellation — defaulting the other way is how a new Discord
        // error code would silently become "cancelled".
        (
            "something_discord_invented_later",
            "/api-tokens/?signin=failed",
        ),
    ] {
        let started = start_login(&open).await;
        let reply = fetch(
            &open,
            &format!("{CALLBACK_PATH}?error={error}&state={}", started.state),
            &[(cookies::PENDING_COOKIE, &started.pending)],
        )
        .await;

        assert_eq!(reply.status, StatusCode::SEE_OTHER, "error={error}");
        assert_eq!(reply.location(), expected, "error={error}");
        // Either way it is a landing state, not a session, and the spent
        // cookie goes.
        assert!(
            reply.cookie(cookies::SESSION_COOKIE).is_none(),
            "error={error}"
        );
        assert!(reply.clears(cookies::PENDING_COOKIE), "error={error}");
    }
}

/// The `error` value is attacker-controlled — it arrives in a query string on a
/// public, keyless route — and it must never reach the `Location` header.
/// Both landing states are literals; only the choice between them depends on
/// the input.
#[tokio::test]
async fn the_error_value_never_reaches_the_redirect_target() {
    let open = signed_in_app(true);
    let started = start_login(&open).await;

    let reply = fetch(
        &open,
        &format!(
            "{CALLBACK_PATH}?error=evil%0d%0aX-Injected:%20yes&state={}",
            started.state
        ),
        &[(cookies::PENDING_COOKIE, &started.pending)],
    )
    .await;

    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location(), "/api-tokens/?signin=failed");
    assert!(!reply.location().contains("evil"));
    assert!(reply.headers.get("x-injected").is_none());
}

/// **A malformed callback is a `400`, not a `502`.**
///
/// These routes are keyless and throttled at 10 req/s, so anyone can call
/// `/auth/login`, take the `state` it hands them and replay it here. Answering
/// `502 discord_unavailable` let that manufacture 5xx on demand — polluting the
/// alarms [0204] is building and making a real Discord outage indistinguishable
/// from a script. Nothing upstream failed, so nothing upstream is blamed.
#[tokio::test]
async fn a_callback_with_no_code_and_no_error_is_a_client_error() {
    let open = signed_in_app(true);
    let started = start_login(&open).await;

    let reply = fetch(
        &open,
        &format!("{CALLBACK_PATH}?state={}", started.state),
        &[(cookies::PENDING_COOKIE, &started.pending)],
    )
    .await;

    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert!(
        !reply.status.is_server_error(),
        "an anonymous caller must not be able to manufacture a 5xx"
    );
    assert_eq!(reply.json()["code"], "invalid_query");
    // Not Discord's fault, so not Discord's error code.
    assert_ne!(reply.json()["code"], "discord_unavailable");
    // The cookie is still spent — `state` verified, so this callback is used up.
    assert!(reply.clears(cookies::PENDING_COOKIE));
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_none());
}

/// The Developer Portal registration and this code can disagree about scope,
/// and the token response is the only place the real grant is visible. A grant
/// wider OR narrower than the requested pair is refused rather than quietly
/// accepted — while the pair in the other order, which is the same set per
/// RFC 6749 §3.3, is not.
#[tokio::test]
async fn a_grant_that_is_not_exactly_the_two_scopes_is_refused() {
    for drifted in [
        // Wider: the registration grew `guilds` or `email` — ADR 0010's
        // named refusals.
        "identify guilds.members.read guilds",
        "identify guilds.members.read email",
        // Narrower: the member scope missing would turn every membership
        // check into a 401-shaped "unknown"; refuse at the exchange instead.
        "identify",
        "guilds.members.read",
    ] {
        let mock = MockDiscord::start(drifted, None).await;
        let open = app_against(&mock);

        let started = start_login(&open).await;
        let (state, pending) = (&started.state, &started.pending);
        let reply = fetch(
            &open,
            &format!("{CALLBACK_PATH}?code=c&state={state}"),
            &[(cookies::PENDING_COOKIE, pending)],
        )
        .await;

        assert_eq!(reply.status, StatusCode::BAD_GATEWAY, "scope={drifted}");
        assert_eq!(reply.json()["code"], "discord_unavailable", "{drifted}");
        assert!(reply.cookie(cookies::SESSION_COOKIE).is_none(), "{drifted}");
    }

    // The same set in the other order is the same grant.
    let reordered = MockDiscord::start("guilds.members.read identify", None).await;
    let open = app_against(&reordered);
    let started = start_login(&open).await;
    let reply = fetch(
        &open,
        &format!("{CALLBACK_PATH}?code=c&state={}", started.state),
        &[(cookies::PENDING_COOKIE, &started.pending)],
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_some());
}

/// Discord having an incident must not read as a bug here, and must not leak
/// Discord's response body to the visitor.
#[tokio::test]
async fn a_failed_token_exchange_is_a_502_with_no_upstream_detail() {
    let mock = MockDiscord::start(GRANTED_SCOPE, Some(StatusCode::UNAUTHORIZED)).await;
    let open = app_against(&mock);

    let started = start_login(&open).await;
    let (state, pending) = (&started.state, &started.pending);
    let reply = fetch(
        &open,
        &format!("{CALLBACK_PATH}?code=c&state={state}"),
        &[(cookies::PENDING_COOKIE, pending)],
    )
    .await;

    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    let body = String::from_utf8(reply.body.clone()).unwrap();
    assert!(!body.contains("upstream said no"), "{body}");
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_none());
    assert!(reply.clears(cookies::PENDING_COOKIE));
}

// ---------------------------------------------------------------------------
// /auth/me and /auth/logout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn me_reports_signed_out_rather_than_refusing_to_answer() {
    let open = signed_in_app(true);
    let reply = fetch(&open, ME_PATH, &[]).await;

    // 200, not 401: this is the question "am I signed in?", and the page renders
    // plain text either way.
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json!({ "authenticated": false }));
}

#[tokio::test]
async fn me_ignores_a_forged_or_expired_session() {
    let open = signed_in_app(true);

    let forged = Session::issue("999", "impostor", state_token::now_secs())
        .encode(b"a-key-that-is-not-the-configured-one-000");
    assert_eq!(
        fetch(&open, ME_PATH, &[(cookies::SESSION_COOKIE, &forged)])
            .await
            .json()["authenticated"],
        json!(false)
    );

    let expired = Session {
        sub: "1".into(),
        name: "adam".into(),
        exp: state_token::now_secs() - 1,
    }
    .encode(SIGNING_KEY.as_bytes());
    assert_eq!(
        fetch(&open, ME_PATH, &[(cookies::SESSION_COOKIE, &expired)])
            .await
            .json()["authenticated"],
        json!(false)
    );

    for garbage in ["", "not-a-cookie", "a.b", "...."] {
        assert_eq!(
            fetch(&open, ME_PATH, &[(cookies::SESSION_COOKIE, garbage)])
                .await
                .status,
            StatusCode::OK,
            "garbage `{garbage}` should be signed-out, not a 500"
        );
    }
}

#[tokio::test]
async fn logout_clears_the_session_at_the_path_that_set_it() {
    let mock = MockDiscord::start(GRANTED_SCOPE, None).await;
    let open = app_against(&mock);

    let started = start_login(&open).await;
    let (state, pending) = (&started.state, &started.pending);
    let session = fetch(
        &open,
        &format!("{CALLBACK_PATH}?code=c&state={state}"),
        &[(cookies::PENDING_COOKIE, pending)],
    )
    .await
    .cookie(cookies::SESSION_COOKIE)
    .unwrap();

    let reply = send(
        &open,
        "POST",
        LOGOUT_PATH,
        &[(cookies::SESSION_COOKIE, &session)],
    )
    .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert!(reply.clears(cookies::SESSION_COOKIE));

    // Same path as the cookie it is clearing — a mismatch leaves the original
    // in place and adds an empty one, which is a sign-out that does nothing.
    let cleared = reply
        .set_cookies()
        .into_iter()
        .find(|c| c.starts_with(cookies::SESSION_COOKIE))
        .unwrap();
    assert!(
        cleared.contains(&format!("Path={}", cookies::SESSION_PATH)),
        "{cleared}"
    );
}

/// A `GET` sign-out is triggerable by any `<img src>` on any page the visitor
/// loads, and `SameSite=Lax` permits it. The route is `POST` only.
#[tokio::test]
async fn logout_is_not_reachable_by_a_get() {
    let open = signed_in_app(true);
    let reply = fetch(&open, LOGOUT_PATH, &[]).await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(reply.set_cookies().is_empty());
}

// ---------------------------------------------------------------------------
// Misconfiguration
// ---------------------------------------------------------------------------

/// An open portal with no credentials must say so, not present a sign-in that
/// silently 404s. `AppConfig::load_portal_oauth` fails at cold start on this
/// combination, so reaching here means something bypassed it — the routes are
/// still mounted and still honest.
#[tokio::test]
async fn an_open_portal_with_no_credentials_answers_503_on_login() {
    let config = AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![],
        portal_enabled: true,
        portal_oauth: None,
        portal_endpoints: Endpoints::default(),
        // Task 0187: the control-plane client for self-service keys. `None`
        // is what every non-portal test wants — with no client in the
        // config there is no code path here that can reach API Gateway.
        portal_keys: None,
        portal_eligibility: None,
    };
    let router = app(&config, AppState::without_ch());

    let login = fetch(&router, LOGIN_PATH, &[]).await;
    assert_eq!(login.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(login.json()["code"], "sign_in_unconfigured");

    // `/auth/me` is the exception: "nobody is signed in" is true and lets the
    // page render.
    let me = fetch(&router, ME_PATH, &[]).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.json()["authenticated"], json!(false));
}

/// The routes are keyless — `crate::auth::is_exempt` exempts the whole prefix
/// while the portal is open — because a visitor signing in to get a key has
/// none. Pinned here as well as in `tests/portal.rs`, since it is one of this
/// task's acceptance criteria and the gateway side (`apiKeyRequired: false`)
/// cannot be asserted from Rust.
#[tokio::test]
async fn sign_in_needs_no_api_key_even_when_the_key_gate_is_armed() {
    let config = AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec!["a-partner-key".to_string()],
        portal_enabled: true,
        portal_oauth: Some(oauth_secret()),
        portal_endpoints: Endpoints::default(),
        // Task 0187: the control-plane client for self-service keys. `None`
        // is what every non-portal test wants — with no client in the
        // config there is no code path here that can reach API Gateway.
        portal_keys: None,
        portal_eligibility: None,
    };
    let router = app(&config, AppState::without_ch());

    assert_eq!(
        fetch(&router, LOGIN_PATH, &[]).await.status,
        StatusCode::SEE_OTHER
    );
    assert_eq!(fetch(&router, ME_PATH, &[]).await.status, StatusCode::OK);
}
