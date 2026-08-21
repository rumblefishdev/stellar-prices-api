//! The mock API Gateway control plane, and the helpers the key tests drive it
//! with (task 0187).
//!
//! A module rather than the top of one test file, because two test **binaries**
//! need it: `tests/portal_keys.rs` covers the behaviour of the two routes, and
//! `tests/portal_keys_logs.rs` covers the one assertion that has to run alone in
//! a process of its own — that file explains why, and the explanation is the
//! reason this split exists.
//!
//! # Why a mock service rather than a trait
//!
//! Task 0186 made this argument for Discord and it is stronger here. A
//! `Gateway` trait would let these tests skip the wire, and the wire is where
//! three of this slice's requirements live:
//!
//! - **`nameQuery` is a prefix match.** A hand-written fake would be tempted to
//!   implement it as equality — which is exactly the bug the exact client-side
//!   filter exists to survive. [`Store::list`] below matches by prefix, like the
//!   service does, so the prefix-collision tests are testing something real.
//! - **Listing paginates.** The `position` cursor is a property of the
//!   protocol; a fake that returns a `Vec` cannot have the bug where ranking
//!   happens off page one.
//! - **`include_values` is a request parameter.** "Never pull a value we do not
//!   need" is only observable in what was *sent*, and the mock records it.
//!
//! The mock is a small, stateful in-memory control plane: keys can be created,
//! listed by prefix across pages, read and deleted, and every call is counted so
//! the assertions can be about what the reconciler *did*, not only about what it
//! returned.
//!
//! Every key it mints carries a `CANARY` substring in its value, which is what
//! `portal_keys_logs.rs` greps the log stream for.
// Each binary uses a subset of these helpers, so "unused" here is per-binary
// noise rather than dead code — the same reason `tests/common/mod.rs` says so.
#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use prices_api::portal::auth::{cookies, secret::OauthSecret, session::Session};
use prices_api::portal::keys::{KEY_PATH, gateway::Gateway};
use prices_api::{AppConfig, AppState, app};
use serde_json::{Value, json};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Mock API Gateway control plane
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct StoredKey {
    pub id: String,
    pub name: String,
    pub value: String,
    pub created_at: u64,
    pub tags: HashMap<String, String>,
    pub enabled: bool,
}

#[derive(Default)]
pub struct Store {
    pub keys: Vec<StoredKey>,
    /// `(usage_plan_id, key_id)` pairs, in the order they were attached.
    pub plan_keys: Vec<(String, String)>,
    /// How many keys one `GetApiKeys` page holds. Small numbers force the
    /// pagination path the reconciler must walk to exhaustion.
    pub page_size: usize,
    pub list_calls: usize,
    pub create_calls: usize,
    pub deleted: Vec<String>,
    /// How many `CreateUsagePlanKey` calls were made, so a test can show that
    /// attaching on every request is one call and not a growing list of plan
    /// memberships.
    pub attach_calls: usize,
    /// Every `includeValues` value the listing was called with.
    pub include_values_seen: Vec<String>,
    /// Make **every** `GetApiKeys` fail with a 500, as a control plane having a
    /// bad day would.
    ///
    /// Sticky rather than one-shot, and that is a finding rather than a
    /// convenience: the SDK retries a 500, so a single failed attempt is not
    /// observable from a handler at all. Only a control plane that stays down
    /// produces the `502` this slice promises.
    pub fail_list: bool,
    /// Delete the key the next `GetApiKey` asks for, *then* answer `404` — the
    /// console-deletes-it-mid-flight race, made deterministic.
    pub vanish_on_next_read: bool,
    /// Answer every `GetApiKey` with a 500.
    ///
    /// A different failure from `read_always_404` and it must land differently:
    /// a `404` is a state this slice has an answer for, a 500 is the control
    /// plane being unwell. The SDK's `into_service_error` maps an unmodelled
    /// failure to an `Unhandled` variant rather than panicking, and this is what
    /// proves the not-found check does not swallow it.
    pub fail_reads: bool,
    /// Delete the key the next `CreateUsagePlanKey` names, *then* answer `404`
    /// — the same console-deletes-it-mid-flight race as
    /// [`Store::vanish_on_next_read`], but in the window the attach opened by
    /// running before the read. One-shot, so the retry that follows can succeed.
    pub vanish_on_next_attach: bool,
    /// Answer every `CreateUsagePlanKey` with `404` **without** removing
    /// anything — what a stale or mistyped usage-plan id looks like, since API
    /// Gateway reports a missing plan and a missing key with the same error.
    pub attach_always_404: bool,
    /// Answer the next `GetApiKeys` with no items at all, whatever the store
    /// holds. An eventually-consistent listing that has not caught up.
    pub next_list_is_empty: bool,
    /// Omit the most recently created key from the next `GetApiKeys`. The other
    /// half of the same lag: the listing is not empty, it just does not have
    /// OUR key in it yet.
    pub next_list_omits_newest: bool,
    /// Hold every `GetApiKeys` for this long before answering, so a test can
    /// drive the reconciler's wall-clock deadline without waiting for it.
    pub list_delay_ms: u64,
    /// Answer every `DeleteApiKey` with a 500.
    ///
    /// Sticky, for the reason `fail_list` is: the SDK retries a 500, so a
    /// one-shot knob would prove nothing about a control plane that is actually
    /// refusing.
    pub fail_deletes: bool,
    /// Answer every `GetApiKey` with `404` **without** removing anything, so the
    /// listing keeps returning a key the read will not produce.
    ///
    /// A stale listing, which is a real control-plane behaviour and the only way
    /// into the bounded-retry branch: an attempt that *creates* the winner
    /// already holds its value and never reads it back, so no amount of deleting
    /// reaches that code. Worth knowing — it means the branch guards a listing
    /// that disagrees with the reader, not a busy deleter.
    pub read_always_404: bool,
    pub next_id: usize,

    // -----------------------------------------------------------------------
    // GetUsage (task 0188)
    // -----------------------------------------------------------------------
    /// Daily `[used, remaining]` pairs per key id, in day order — what
    /// `GetUsage` reports under `items`. A key with no entry here has no row,
    /// which is exactly what a freshly attached key looks like while the
    /// reporting lags. Raw `Vec<Vec<i64>>` rather than tuples, so a test can
    /// also serve a MALFORMED row (`[121]`, `[]`) and prove the handler skips
    /// it instead of defaulting the missing element.
    pub usage: HashMap<String, Vec<Vec<i64>>>,
    /// How many `GetUsage` HTTP calls arrived — what the cache assertions
    /// count.
    pub usage_calls: usize,
    /// Every `(keyId, startDate, endDate)` triple `GetUsage` was asked for, so
    /// a test can assert WHICH key's usage was read and over which period.
    pub usage_queries: Vec<(String, String, String)>,
    /// Answer every `GetUsage` with a 500. Sticky, for the reason `fail_list`
    /// is: the SDK retries a 500, so a one-shot knob observes nothing.
    pub fail_usage: bool,
    /// Answer every `GetUsage` with `429 TooManyRequestsException` — the
    /// account-wide control-plane throttle. Sticky for the same reason, and
    /// doubly so here: the SDK's own backoff retries a 429, so only a throttle
    /// that persists across those retries reaches the handler at all.
    pub throttle_usage: bool,
    /// The same throttle on `GetApiKeys`. The account-wide budget does not
    /// care which operation is asked, so the usage flow's key LOOKUP can be
    /// the throttled call just as well as the usage read — and the two must
    /// land in the same stale-serve branch (`GatewayError::Throttled` from
    /// `list_named`), which this knob is what makes assertable.
    pub throttle_list: bool,
    /// How many daily pairs one `GetUsage` page holds. `0` means everything in
    /// one page; a small number forces the pagination path the summing must
    /// walk to exhaustion.
    pub usage_page_size: usize,
}

impl Store {
    pub fn seed(&mut self, name: &str, created_at: u64) -> String {
        let id = self.mint_id();
        self.keys.push(StoredKey {
            id: id.clone(),
            name: name.to_string(),
            value: format!("VALUE-{id}-CANARY"),
            created_at,
            tags: HashMap::new(),
            enabled: true,
        });
        id
    }

    pub fn mint_id(&mut self) -> String {
        self.next_id += 1;
        format!("key{:03}", self.next_id)
    }

    pub fn named(&self, name: &str) -> Vec<&StoredKey> {
        self.keys.iter().filter(|k| k.name == name).collect()
    }

    /// Prefix match, **case-sensitive** — what `nameQuery` actually does. The
    /// single most important line in this mock: implementing it as equality
    /// would make the exact-filter tests vacuous.
    pub fn list(&self, prefix: &str) -> Vec<&StoredKey> {
        self.keys
            .iter()
            .filter(|k| k.name.starts_with(prefix))
            .collect()
    }
}

#[derive(Clone)]
pub struct MockGateway {
    pub base: String,
    pub store: Arc<Mutex<Store>>,
}

impl MockGateway {
    pub async fn start() -> Self {
        let store = Arc::new(Mutex::new(Store {
            page_size: 100,
            ..Store::default()
        }));

        let router = Router::new()
            .route("/apikeys", get(list_keys).post(create_key))
            .route("/apikeys/{id}", get(read_key).delete(delete_key))
            .route("/usageplans/{plan}/keys", post(attach_key))
            .route("/usageplans/{plan}/usage", get(read_usage))
            .with_state(store.clone());

        let listener = tokio::net::TcpListener::bind::<SocketAddr>(([127, 0, 0, 1], 0).into())
            .await
            .expect("the mock control plane must bind to loopback");
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        Self { base, store }
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut Store) -> T) -> T {
        f(&mut self.store.lock().unwrap())
    }
}

#[derive(serde::Deserialize)]
pub struct ListQuery {
    pub name: Option<String>,
    pub position: Option<String>,
    #[serde(rename = "includeValues")]
    pub include_values: Option<String>,
    #[allow(dead_code)]
    pub limit: Option<i32>,
}

pub async fn list_keys(
    State(store): State<Arc<Mutex<Store>>>,
    Query(query): Query<ListQuery>,
) -> Response {
    // Read the delay and DROP the lock before sleeping: holding a `std::sync`
    // guard across an await would block every other request to this mock, which
    // is the opposite of what a slow control plane does.
    let delay = store.lock().unwrap().list_delay_ms;
    if delay > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }

    let mut store = store.lock().unwrap();
    store.list_calls += 1;
    store
        .include_values_seen
        .push(query.include_values.clone().unwrap_or_default());

    if store.throttle_list {
        return throttled();
    }
    if store.fail_list {
        return (StatusCode::INTERNAL_SERVER_ERROR, "control plane is unwell").into_response();
    }

    if std::mem::take(&mut store.next_list_is_empty) {
        return Json(json!({ "item": [] })).into_response();
    }

    let prefix = query.name.unwrap_or_default();
    let newest = if std::mem::take(&mut store.next_list_omits_newest) {
        store.keys.last().map(|k| k.id.clone())
    } else {
        None
    };
    let matched: Vec<StoredKey> = store
        .list(&prefix)
        .into_iter()
        .filter(|k| Some(&k.id) != newest.as_ref())
        .cloned()
        .collect();
    let start: usize = query
        .position
        .as_deref()
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    let start = start.min(matched.len());
    let end = (start + store.page_size).min(matched.len());

    let mut body = json!({
        "item": matched[start..end].iter().map(api_key_json).collect::<Vec<_>>(),
    });
    if end < matched.len() {
        body["position"] = json!(end.to_string());
    }
    Json(body).into_response()
}

pub async fn create_key(
    State(store): State<Arc<Mutex<Store>>>,
    Json(body): Json<Value>,
) -> Response {
    let mut store = store.lock().unwrap();
    store.create_calls += 1;
    let id = store.mint_id();
    let created = StoredKey {
        id: id.clone(),
        name: body["name"].as_str().unwrap_or_default().to_string(),
        value: format!("VALUE-{id}-CANARY"),
        // Whole seconds, like the service — which is why the winner rule needs
        // an id tie-break.
        created_at: 1_800_000_000,
        tags: body["tags"]
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        enabled: body["enabled"].as_bool().unwrap_or(false),
    };
    store.keys.push(created.clone());
    (StatusCode::CREATED, Json(api_key_json(&created))).into_response()
}

#[derive(serde::Deserialize)]
pub struct ReadQuery {
    #[serde(rename = "includeValue")]
    pub include_value: Option<String>,
}

pub async fn read_key(
    State(store): State<Arc<Mutex<Store>>>,
    Path(id): Path<String>,
    Query(query): Query<ReadQuery>,
) -> Response {
    let mut store = store.lock().unwrap();

    if store.fail_reads {
        return (StatusCode::INTERNAL_SERVER_ERROR, "control plane is unwell").into_response();
    }
    if store.read_always_404 {
        return not_found();
    }
    if std::mem::take(&mut store.vanish_on_next_read) {
        store.keys.retain(|k| k.id != id);
        return not_found();
    }

    let Some(key) = store.keys.iter().find(|k| k.id == id).cloned() else {
        return not_found();
    };
    let mut body = api_key_json(&key);
    if query.include_value.as_deref() != Some("true") {
        body["value"] = Value::Null;
    }
    Json(body).into_response()
}

pub async fn delete_key(
    State(store): State<Arc<Mutex<Store>>>,
    Path(id): Path<String>,
) -> Response {
    let mut store = store.lock().unwrap();
    if store.fail_deletes {
        return (StatusCode::INTERNAL_SERVER_ERROR, "control plane is unwell").into_response();
    }
    store.deleted.push(id.clone());
    let existed = store.keys.iter().any(|k| k.id == id);
    store.keys.retain(|k| k.id != id);
    if existed {
        StatusCode::ACCEPTED.into_response()
    } else {
        not_found()
    }
}

pub async fn attach_key(
    State(store): State<Arc<Mutex<Store>>>,
    Path(plan): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let mut store = store.lock().unwrap();
    store.attach_calls += 1;
    let key_id = body["keyId"].as_str().unwrap_or_default().to_string();
    if store.attach_always_404 {
        return not_found();
    }
    if std::mem::take(&mut store.vanish_on_next_attach) {
        store.keys.retain(|k| k.id != key_id);
        return not_found();
    }
    if !store.keys.iter().any(|k| k.id == key_id) {
        return not_found();
    }
    // Already on this plan → `409 ConflictException`, exactly as the service
    // answers. The reconciler attaches every key it is about to hand out, so
    // this is the ordinary case rather than an edge one, and a mock that
    // silently accepted a re-attach would let a handler that treats the
    // conflict as a failure pass every test here.
    if store
        .plan_keys
        .iter()
        .any(|(p, k)| p == &plan && k == &key_id)
    {
        return conflict();
    }
    store.plan_keys.push((plan, key_id.clone()));
    (
        StatusCode::CREATED,
        Json(json!({ "id": key_id, "type": "API_KEY" })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
pub struct UsageQuery {
    #[serde(rename = "keyId")]
    pub key_id: Option<String>,
    #[serde(rename = "startDate")]
    pub start_date: Option<String>,
    #[serde(rename = "endDate")]
    pub end_date: Option<String>,
    pub position: Option<String>,
    #[allow(dead_code)]
    pub limit: Option<i32>,
}

/// `GET /usageplans/{plan}/usage` — task 0188's one new control-plane call.
///
/// The wire field for the per-key daily pairs is **`values`**, not `items`:
/// the SDK's member is named `items` but its deserializer matches `"values"`
/// (checked against `aws-sdk-apigateway`'s `shape_get_usage.rs`). A mock that
/// answered with `items` would hand every test an empty map and the
/// no-rows path would cover everything, vacuously.
pub async fn read_usage(
    State(store): State<Arc<Mutex<Store>>>,
    Path(_plan): Path<String>,
    Query(query): Query<UsageQuery>,
) -> Response {
    let mut store = store.lock().unwrap();
    store.usage_calls += 1;
    let key_id = query.key_id.clone().unwrap_or_default();
    store.usage_queries.push((
        key_id.clone(),
        query.start_date.clone().unwrap_or_default(),
        query.end_date.clone().unwrap_or_default(),
    ));

    if store.throttle_usage {
        return throttled();
    }
    if store.fail_usage {
        return (StatusCode::INTERNAL_SERVER_ERROR, "control plane is unwell").into_response();
    }

    let days = store.usage.get(&key_id).cloned().unwrap_or_default();
    let start: usize = query
        .position
        .as_deref()
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    let start = start.min(days.len());
    let page = if store.usage_page_size == 0 {
        days.len()
    } else {
        store.usage_page_size
    };
    let end = (start + page).min(days.len());

    let mut body = json!({
        "usagePlanId": "freeplan1",
        "startDate": query.start_date,
        "endDate": query.end_date,
    });
    // No entry at all for a key with no rows — what a freshly attached key
    // looks like while the reporting lags.
    if !days.is_empty() {
        body["values"] = json!({
            key_id: days[start..end].iter().map(|day| json!(day)).collect::<Vec<_>>(),
        });
    }
    if end < days.len() {
        body["position"] = json!(end.to_string());
    }
    Json(body).into_response()
}

/// The `429` shape the SDK maps to `TooManyRequestsException` — the
/// account-wide control-plane throttle. Sibling of [`not_found`], and the
/// header is again what restJson1 matches on: without it the SDK reports a
/// generic error and the handler's throttle branch — the one that serves the
/// last good answer instead of an error page — is never reached.
pub fn throttled() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [("x-amzn-errortype", "TooManyRequestsException")],
        Json(json!({ "message": "Too Many Requests" })),
    )
        .into_response()
}

/// The `404` shape the SDK maps to `NotFoundException`.
///
/// The header is what restJson1 matches on; the body is what a human reads. Get
/// the header wrong and the SDK reports a generic error, at which point
/// `Gateway::value_of` would answer `Err` instead of `Ok(None)` and the
/// deleted-by-hand path would surface as a `502`. That is the failure this
/// helper exists to keep honest.
pub fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [("x-amzn-errortype", "NotFoundException")],
        Json(json!({ "message": "Invalid API Key identifier specified" })),
    )
        .into_response()
}

/// The `409` shape the SDK maps to `ConflictException`.
///
/// Sibling of [`not_found`] and load-bearing for the same reason: the header is
/// what restJson1 matches on, and `Gateway::attach_to_free_plan` reads that
/// mapping to decide that a key already on the plan is the desired state rather
/// than a failure.
pub fn conflict() -> Response {
    (
        StatusCode::CONFLICT,
        [("x-amzn-errortype", "ConflictException")],
        Json(json!({ "message": "API Key already exists in the usage plan" })),
    )
        .into_response()
}

pub fn api_key_json(key: &StoredKey) -> Value {
    json!({
        "id": key.id,
        "name": key.name,
        "value": key.value,
        "enabled": key.enabled,
        "createdDate": key.created_at,
        "tags": key.tags,
    })
}

// ---------------------------------------------------------------------------
// Router under test
// ---------------------------------------------------------------------------

pub const SIGNING_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub const PLAN_ID: &str = "freeplan1";
pub const USER_ID: &str = "308994132968210433";
pub const USER_NAME: &str = "adam";

pub fn oauth_secret() -> OauthSecret {
    OauthSecret::parse(
        &json!({
            "client_id": "a-client-id",
            "client_secret": "the-client-secret",
            "redirect_uri": "https://portal.example/api-tokens/api/auth/callback",
            "session_signing_key": SIGNING_KEY,
        })
        .to_string(),
    )
    .expect("the test bundle must be valid")
}

pub fn build_app(portal_enabled: bool, gateway: Option<Gateway>) -> Router {
    build_app_with(portal_enabled, gateway, Default::default(), None)
}

/// [`build_app`], plus where Discord is and where the eligibility knobs come
/// from — what the issue round-trip suite (`tests/portal_issue.rs`) needs:
/// the `action=issue` callback talks to a mock Discord AND the mock control
/// plane in one request.
pub fn build_app_with(
    portal_enabled: bool,
    gateway: Option<Gateway>,
    endpoints: prices_api::portal::auth::discord::Endpoints,
    eligibility: Option<prices_api::portal::eligibility::EligibilitySettings>,
) -> Router {
    let config = AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![],
        portal_enabled,
        portal_oauth: portal_enabled.then(oauth_secret),
        portal_endpoints: endpoints,
        portal_keys: gateway,
        portal_eligibility: eligibility,
        portal_rate_limit: None,
    };
    app(&config, AppState::without_ch())
}

/// Eligibility knobs for tests: direct values, no SSM. The guild id is a
/// snowflake because the code validates that before building the member URL.
pub fn eligibility(
    guild_id: &str,
    min_age_minutes: &str,
) -> prices_api::portal::eligibility::EligibilitySettings {
    use prices_api::portal::eligibility::{EligibilitySettings, ParamSource};
    EligibilitySettings {
        guild_id: ParamSource::Direct(guild_id.to_string()),
        min_account_age: ParamSource::Direct(min_age_minutes.to_string()),
    }
}

/// The guild the issue suite gates on — any syntactically valid snowflake.
pub const GUILD_ID: &str = "897514728459468821";

/// A router with the portal open, sign-in configured, and the control plane
/// pointed at `mock`.
pub fn app_against(mock: &MockGateway) -> Router {
    build_app(
        true,
        Some(Gateway::against(&mock.base, PLAN_ID.to_string())),
    )
}

/// The key routes alone, with a shortened reconciliation deadline.
///
/// Bypasses [`build_app`] on purpose: the deadline lives on `KeysState`, which
/// `AppConfig` does not carry, and threading it through the whole config for one
/// test would put a production knob somewhere a deploy could reach. What this
/// skips — task 0183's prefix gate — is covered by its own tests; what it keeps
/// is the handler, the real SDK and the mock behind it.
pub fn keys_router_with_deadline(mock: &MockGateway, deadline: std::time::Duration) -> Router {
    prices_api::portal::keys::routes(
        prices_api::portal::keys::KeysState::new(
            Some(oauth_secret()),
            Some(Gateway::against(&mock.base, PLAN_ID.to_string())),
        )
        .with_deadline(deadline),
    )
}

/// A session cookie for `sub`, signed exactly as the callback signs it.
pub fn session_cookie(sub: &str) -> String {
    let session = Session::issue(sub, USER_NAME, now_secs());
    format!(
        "{}={}",
        cookies::SESSION_COOKIE,
        session.encode(SIGNING_KEY.as_bytes())
    )
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| {
            panic!(
                "non-JSON body (status {}): {}",
                self.status,
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    pub fn cache_control(&self) -> String {
        self.headers
            .get(header::CACHE_CONTROL)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default()
    }

    pub fn location(&self) -> String {
        self.headers
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default()
    }

    /// The value a browser would store for cookie `name`, or `None` if this
    /// response clears it or never set it.
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
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
}

/// The usage route alone (task 0188), with the cache TTL and the deadline both
/// shortened.
///
/// Bypasses [`build_app`] for the reason `keys_router_with_deadline` does: the
/// TTL and deadline live on `UsageState`, which `AppConfig` does not carry, and
/// threading them through the whole config would put two production knobs
/// somewhere a deploy could reach. What this skips — task 0183's prefix gate —
/// is covered by its own tests; what it keeps is the handler, the real SDK and
/// the mock behind it.
pub fn usage_router_with(
    mock: &MockGateway,
    ttl: std::time::Duration,
    deadline: std::time::Duration,
) -> Router {
    prices_api::portal::usage::routes(
        prices_api::portal::usage::UsageState::new(
            Some(oauth_secret()),
            Some(Gateway::against(&mock.base, PLAN_ID.to_string())),
        )
        .with_ttl(ttl)
        .with_deadline(deadline),
    )
}

pub async fn call(router: Router, method: &str, cookie: Option<&str>) -> Reply {
    call_path(router, method, KEY_PATH, cookie).await
}

pub async fn call_path(router: Router, method: &str, path: &str, cookie: Option<&str>) -> Reply {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let response = router
        .oneshot(request.body(Body::empty()).unwrap())
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

/// `GET /key` — the reveal. Since task 0189 the `POST` verb answers
/// identically (read-only); [`post_key`] exists so tests can pin exactly that.
pub async fn reveal(mock: &MockGateway, sub: &str) -> Reply {
    call(app_against(mock), "GET", Some(&session_cookie(sub))).await
}

/// `POST /key` — 0187's issue verb, which task 0189 made a second reveal.
pub async fn post_key(mock: &MockGateway, sub: &str) -> Reply {
    call(app_against(mock), "POST", Some(&session_cookie(sub))).await
}

/// Drive a full `action=issue` OAuth round-trip against `router` (which must
/// be built with `build_app_with`, pointed at a mock Discord): `/auth/login`
/// mints the state pair, the callback completes the action. Returns the
/// callback's reply — a `303` whose `Location` is one of the five
/// `?issue=…` landing states.
pub async fn issue_round_trip(router: &Router) -> Reply {
    let login = call_path(
        router.clone(),
        "GET",
        "/api-tokens/api/auth/login?action=issue",
        None,
    )
    .await;
    assert_eq!(
        login.status,
        StatusCode::SEE_OTHER,
        "login must redirect to Discord: {}",
        String::from_utf8_lossy(&login.body)
    );
    let pending = login
        .cookie(cookies::PENDING_COOKIE)
        .expect("login must set the pending-login cookie");
    let location = login.location();
    let query = location
        .split_once('?')
        .expect("the authorize URL carries a query")
        .1;
    let state = form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == "state")
        .expect("the authorize URL must carry `state`")
        .1
        .to_string();

    call_path(
        router.clone(),
        "GET",
        &format!("/api-tokens/api/auth/callback?code=an-auth-code&state={state}"),
        Some(&format!("{}={pending}", cookies::PENDING_COOKIE)),
    )
    .await
}
