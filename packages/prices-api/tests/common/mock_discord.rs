//! A mock Discord on loopback, shared by the sign-in suite
//! (`tests/portal_auth.rs`) and the issue round-trip suite
//! (`tests/portal_issue.rs`) via `#[path]`, the same way
//! `tests/portal_keys/harness.rs` is shared.
//!
//! Serves the three routes the service calls — `POST /oauth2/token`,
//! `GET /users/@me`, and `GET /users/@me/guilds/{guild}/member` (task 0189) —
//! and **records what it received**, so tests assert on the request the code
//! actually made: the client secret in the form body, the PKCE verifier, the
//! bearer token and guild id on the member call.
//!
//! # Why a mock server rather than a trait
//!
//! Injecting a `DiscordClient` trait would let the tests skip the HTTP layer
//! entirely — and the HTTP layer is where the requirements live: that the
//! client secret goes in the form body and not the URL, that the verifier sent
//! is the one the challenge was derived from, that the granted scope is
//! checked, and that a member `404` is only "not a member" when the JSON
//! `code` says so. A fake satisfying a trait proves none of them.
// Each binary uses a subset of these helpers, so "unused" here is per-binary
// noise rather than dead code — the same reason `portal_keys/harness.rs` says so.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

/// The scope pair production requests — what the mock grants unless a test
/// overrides it to model registration drift.
pub const GRANTED_SCOPE: &str = "identify guilds.members.read";

/// The mock user's snowflake: 2017-04-02, comfortably older than any sane
/// threshold, so age never interferes with a membership-focused test.
pub const USER_ID: &str = "308994132968210433";
pub const USER_NAME: &str = "adam";

/// How `GET /users/@me/guilds/{guild}/member` answers.
#[derive(Clone)]
pub enum MemberReply {
    /// `200` with a member object; `pending` present or absent as given.
    Member { pending: Option<bool> },
    /// `404` with Discord's error envelope carrying this JSON `code`
    /// (10007 Unknown Member, 10004 Unknown Guild — or anything else, to
    /// model an unrecognised shape).
    NotFound { code: u64 },
    /// A bare status — 429, 500, 403… — with a non-envelope body.
    Status(StatusCode),
    /// `200` whose body is not JSON at all.
    Malformed,
}

/// What the mock saw, so the tests can assert on the request rather than only
/// on the response.
#[derive(Default)]
pub struct Recorded {
    /// The form body of the last `POST /oauth2/token`, decoded.
    pub token_form: Vec<(String, String)>,
    /// The `Authorization` header of the last `GET /users/@me`.
    pub bearer: Option<String>,
    /// How many code exchanges were attempted.
    pub exchanges: usize,
    /// How many membership lookups were attempted.
    pub member_calls: usize,
    /// The `Authorization` header of the last membership lookup.
    pub member_bearer: Option<String>,
    /// The `{guild}` path segment of the last membership lookup.
    pub member_guild: Option<String>,
}

#[derive(Clone)]
struct MockState {
    recorded: Arc<Mutex<Recorded>>,
    granted_scope: String,
    token_status: Option<StatusCode>,
    member: MemberReply,
    user_id: String,
}

pub struct MockDiscord {
    pub base: String,
    pub recorded: Arc<Mutex<Recorded>>,
}

impl MockDiscord {
    /// The common case: the production scope pair granted, the member in good
    /// standing (`pending: false`), the default user.
    pub async fn start(granted_scope: &str, token_status: Option<StatusCode>) -> Self {
        Self::start_with(
            granted_scope,
            token_status,
            MemberReply::Member {
                pending: Some(false),
            },
            USER_ID,
        )
        .await
    }

    /// Full control, for the issue suite: what the member route answers, and
    /// which snowflake `/users/@me` reports (a freshly minted one drives the
    /// too-young refusal).
    pub async fn start_with(
        granted_scope: &str,
        token_status: Option<StatusCode>,
        member: MemberReply,
        user_id: &str,
    ) -> Self {
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let state = MockState {
            recorded: recorded.clone(),
            granted_scope: granted_scope.to_string(),
            token_status,
            member,
            user_id: user_id.to_string(),
        };

        let router = Router::new()
            .route("/oauth2/token", post(token))
            .route("/users/@me", get(current_user))
            .route("/users/@me/guilds/{guild}/member", get(guild_member))
            .with_state(state);

        // Port 0: the OS picks a free one, so the suite can run in parallel
        // with itself and with anything else on the machine.
        let listener = tokio::net::TcpListener::bind::<SocketAddr>(([127, 0, 0, 1], 0).into())
            .await
            .expect("the mock must bind to loopback");
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        Self { base, recorded }
    }

    pub fn form_field(&self, name: &str) -> Option<String> {
        self.recorded
            .lock()
            .unwrap()
            .token_form
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }

    pub fn exchanges(&self) -> usize {
        self.recorded.lock().unwrap().exchanges
    }

    pub fn member_calls(&self) -> usize {
        self.recorded.lock().unwrap().member_calls
    }

    pub fn member_bearer(&self) -> Option<String> {
        self.recorded.lock().unwrap().member_bearer.clone()
    }

    pub fn member_guild(&self) -> Option<String> {
        self.recorded.lock().unwrap().member_guild.clone()
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

async fn current_user(
    State(state): State<MockState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    state.recorded.lock().unwrap().bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    Json(json!({
        "id": state.user_id,
        "username": USER_NAME,
        "discriminator": "0",
        "global_name": "Adam",
        // Fields this service must ignore rather than carry anywhere.
        "email": "someone@example.com",
        "verified": true,
    }))
}

async fn guild_member(
    State(state): State<MockState>,
    Path(guild): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    {
        let mut recorded = state.recorded.lock().unwrap();
        recorded.member_calls += 1;
        recorded.member_bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        recorded.member_guild = Some(guild);
    }
    match &state.member {
        MemberReply::Member { pending } => {
            // The full-ish member object, so the code's "read only `pending`"
            // rule is exercised against fields it must ignore.
            let mut body = json!({
                "roles": ["1"],
                "joined_at": "2020-01-01T00:00:00.000000+00:00",
                "deaf": false,
                "mute": false,
                "flags": 0,
                "user": { "id": USER_ID, "username": USER_NAME },
            });
            if let Some(pending) = pending {
                body["pending"] = json!(pending);
            }
            Json(body).into_response()
        }
        MemberReply::NotFound { code } => (
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "Unknown", "code": code })),
        )
            .into_response(),
        MemberReply::Status(status) => (*status, "not an envelope").into_response(),
        MemberReply::Malformed => "<html>definitely not json</html>".into_response(),
    }
}
