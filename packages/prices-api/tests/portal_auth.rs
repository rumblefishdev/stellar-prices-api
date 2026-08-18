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
//! The mock is [`MockDiscord`]: an axum app on an ephemeral port, serving
//! `/oauth2/token` and `/users/@me`, pointed at by `DISCORD_API_BASE`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use prices_api::portal::auth::{
    CALLBACK_PATH, LOGIN_PATH, LOGOUT_PATH, ME_PATH, cookies, crypto, secret::OauthSecret,
    session::Session, state_token,
};
use prices_api::{AppConfig, AppState, app};
use serde_json::{Value, json};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Mock Discord
// ---------------------------------------------------------------------------

/// What the mock saw, so the tests can assert on the request rather than only on
/// the response.
#[derive(Default)]
struct Recorded {
    /// The form body of the last `POST /oauth2/token`, decoded.
    token_form: Vec<(String, String)>,
    /// The `Authorization` header of the last `GET /users/@me`.
    bearer: Option<String>,
    /// How many code exchanges were attempted.
    exchanges: usize,
}

#[derive(Clone)]
struct MockState {
    recorded: Arc<Mutex<Recorded>>,
    /// Scope the mock claims to have granted. Overridden by one test.
    granted_scope: String,
    /// When set, `/oauth2/token` answers with this status instead of a token.
    token_status: Option<StatusCode>,
}

struct MockDiscord {
    base: String,
    recorded: Arc<Mutex<Recorded>>,
}

impl MockDiscord {
    async fn start(granted_scope: &str, token_status: Option<StatusCode>) -> Self {
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let state = MockState {
            recorded: recorded.clone(),
            granted_scope: granted_scope.to_string(),
            token_status,
        };

        let router = Router::new()
            .route("/oauth2/token", post(token))
            .route("/users/@me", get(current_user))
            .with_state(state);

        // Port 0: the OS picks a free one, so the suite can run in parallel with
        // itself and with anything else on the machine.
        let listener = tokio::net::TcpListener::bind::<SocketAddr>(([127, 0, 0, 1], 0).into())
            .await
            .expect("the mock must bind to loopback");
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        Self { base, recorded }
    }

    fn form_field(&self, name: &str) -> Option<String> {
        self.recorded
            .lock()
            .unwrap()
            .token_form
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }

    fn exchanges(&self) -> usize {
        self.recorded.lock().unwrap().exchanges
    }
}

async fn token(State(state): State<MockState>, body: String) -> axum::response::Response {
    {
        let mut recorded = state.recorded.lock().unwrap();
        recorded.token_form = form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect();
        recorded.exchanges += 1;
    }
    if let Some(status) = state.token_status {
        return (status, "upstream said no").into_response();
    }
    Json(json!({
        "access_token": "an-access-token",
        "token_type": "Bearer",
        "expires_in": 604800,
        "refresh_token": "a-refresh-token",
        "scope": state.granted_scope,
    }))
    .into_response()
}

async fn current_user(State(state): State<MockState>, headers: HeaderMap) -> Json<Value> {
    state.recorded.lock().unwrap().bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    Json(json!({
        "id": "308994132968210433",
        "username": "adam",
        "discriminator": "0",
        "global_name": "Adam",
        // Fields this service must ignore rather than carry anywhere.
        "email": "someone@example.com",
        "verified": true,
    }))
}

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

/// A router with the portal open and sign-in configured.
///
/// `DISCORD_API_BASE` is set for the process, which is why every test that needs
/// a mock takes [`ENV_LOCK`] — `std::env::set_var` is process-global and the
/// suite runs its tests on one runtime.
fn signed_in_app(portal_enabled: bool) -> Router {
    let config = AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![],
        portal_enabled,
        portal_oauth: portal_enabled.then(oauth_secret),
    };
    app(&config, AppState::without_ch())
}

/// `Endpoints::from_env` is read when the router is built, and the environment
/// is process-wide. Serialize the tests that depend on it.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Build a router pointed at `mock`, holding the lock only while the router is
/// being constructed.
///
/// The guard is deliberately NOT returned to the caller. `Endpoints::from_env`
/// runs once, at construction, so the returned router has already captured
/// `mock.base` and a later test swapping the variable cannot reach it — and
/// holding a `std::sync::Mutex` guard across the `.await`s of a test body is a
/// deadlock waiting for a multi-threaded runtime.
fn app_against(mock: &MockDiscord) -> Router {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the lock makes this the only thread touching the variable, and
    // every router that reads it is built before the lock is released.
    unsafe { std::env::set_var("DISCORD_API_BASE", &mock.base) };
    signed_in_app(true)
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
async fn login_redirects_to_discord_asking_for_identify_and_nothing_else() {
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

    assert_eq!(field("scope"), "identify");
    assert_eq!(field("response_type"), "code");
    assert_eq!(field("code_challenge_method"), "S256");
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
/// action this build does not implement cannot start a round-trip that the
/// callback would then have to decide what to do with.
#[tokio::test]
async fn login_refuses_an_action_it_does_not_implement() {
    let open = signed_in_app(true);
    assert_eq!(
        fetch(&open, &format!("{LOGIN_PATH}?action=signin"), &[])
            .await
            .status,
        StatusCode::SEE_OTHER
    );
    let refused = fetch(&open, &format!("{LOGIN_PATH}?action=issue"), &[]).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(refused.set_cookies().is_empty());
}

// ---------------------------------------------------------------------------
// /auth/callback — the happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_complete_round_trip_signs_the_visitor_in() {
    let mock = MockDiscord::start("identify", None).await;
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
    let mock = MockDiscord::start("identify", None).await;
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
    let mock = MockDiscord::start("identify", None).await;
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
    let mock = MockDiscord::start("identify", None).await;
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
    let mock = MockDiscord::start("identify", None).await;
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
    let mock = MockDiscord::start("identify", None).await;
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
    let mock = MockDiscord::start("identify", None).await;
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
    let mock = MockDiscord::start("identify", None).await;
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
    let mock = MockDiscord::start("identify", None).await;
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
    let mock = MockDiscord::start("identify", None).await;
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
    let broken = MockDiscord::start("identify", Some(StatusCode::UNAUTHORIZED)).await;
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

/// The Developer Portal registration and this code can disagree about scope,
/// and the token response is the only place the real grant is visible. A
/// broader grant than `identify` is refused rather than quietly accepted.
#[tokio::test]
async fn a_grant_wider_than_identify_is_refused() {
    let mock = MockDiscord::start("identify guilds", None).await;
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
    assert_eq!(reply.json()["code"], "discord_unavailable");
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_none());
}

/// Discord having an incident must not read as a bug here, and must not leak
/// Discord's response body to the visitor.
#[tokio::test]
async fn a_failed_token_exchange_is_a_502_with_no_upstream_detail() {
    let mock = MockDiscord::start("identify", Some(StatusCode::UNAUTHORIZED)).await;
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
    let mock = MockDiscord::start("identify", None).await;
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
    };
    let router = app(&config, AppState::without_ch());

    assert_eq!(
        fetch(&router, LOGIN_PATH, &[]).await.status,
        StatusCode::SEE_OTHER
    );
    assert_eq!(fetch(&router, ME_PATH, &[]).await.status, StatusCode::OK);
}
