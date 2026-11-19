//! The API hub's dashboard contract (task 0348): `GET /api/hub/v1/account`,
//! `POST /api/hub/v1/key/{reveal,revoke,issue}`, each authorised by a token
//! the hub signs for one call (`v1.<payload>.<mac>`, the shared sign-in spec's
//! decision 15).
//!
//! Driven through the real router (`app_with_portal`, so the gate, the
//! portal's CORS layer and the API-key gate are all in front of it) against
//! the mock control plane the key and usage suites use. Pinned here, beside
//! each route's happy path:
//!
//! - a missing, malformed or refused token is `401 invalid_token` before any
//!   control-plane call (`list_calls == 0`), on every route;
//! - Review Focus 2: a token whose `act` is not the route's — a `read` token
//!   on `key/revoke` or `key/issue` above all — is `403 action_not_allowed`,
//!   with nothing asked of AWS;
//! - no hub secret, or one that does not parse, is `503 hub_unconfigured`
//!   while `/api/config` still answers `enabled`;
//! - Review Focus 4: a failing or slow control plane answers the portal's own
//!   `502` / `503 key_unavailable`, with a code and words and nothing else;
//! - Review Focus 5: a hub revoke or issue evicts the account's cached usage
//!   in this execution environment, for `account` and for the portal's
//!   `/api/usage` alike;
//! - `Cache-Control: no-store` on every answer, a `405` included;
//! - no CORS headers, ever: no browser calls the contract;
//! - the portal's own load and the hub's secret are awaited together, inside
//!   the invocation, and the portal's load is never cut short by the hub's.
//!
//! What the logs say is in `tests/portal_hub_logs.rs`, a binary of its own
//! for the reason `tests/portal_keys_logs.rs` gives.

#[path = "portal_keys/harness.rs"]
mod harness;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use harness::*;
use prices_api::portal::auth::crypto;
use prices_api::portal::hub::secret::HubSecret;
use prices_api::portal::hub::token::{AUDIENCE, CONTEXT, ISSUER};
use prices_api::portal::hub::{
    ACCOUNT_PATH, HubState, ISSUE_PATH, REVEAL_PATH, REVOKE_PATH, routes as hub_routes,
};
use prices_api::portal::period::Period;
use prices_api::portal::sources::{HubLoader, HubSource, Loaded, Loader, PortalSources};
use prices_api::portal::usage::UsageState;
use prices_api::{AppConfig, AppState, app_with_portal};
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The spec's test vector's secret: the bytes 1 to 32.
const SECRET: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA=";
/// Another key: 32 bytes of 0xAA.
const OTHER_SECRET: &str = "qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqo=";
/// The bundle's origin: the portal's CORS layer grants it, the contract not.
const WEB_ORIGIN: &str = "https://sorobanscan.example";
/// The free plan's rate limit this deployment reports.
const RATE_LIMIT: u32 = 5;
/// The Discord username the hub puts in `name`.
const USERNAME: &str = "nelly-the-tester";

/// Every route, its verb and the `act` it needs.
const ROUTES: [(&str, &str, &str); 4] = [
    ("GET", ACCOUNT_PATH, "read"),
    ("POST", REVEAL_PATH, "reveal"),
    ("POST", REVOKE_PATH, "revoke"),
    ("POST", ISSUE_PATH, "issue"),
];
const ACTS: [&str; 4] = ["read", "reveal", "revoke", "issue"];

fn hub_secret() -> HubSecret {
    HubSecret::parse(&json!({ "current": SECRET }).to_string()).unwrap()
}

static JTI: AtomicUsize = AtomicUsize::new(0);

/// The claims the hub signs for one call by our test account.
fn claims(act: &str) -> Map<String, Value> {
    let now = now_secs();
    let n = JTI.fetch_add(1, Ordering::SeqCst);
    json!({
        "iss": ISSUER,
        "aud": AUDIENCE,
        "sub": format!("discord:{USER_ID}"),
        "name": USERNAME,
        "iat": now,
        "exp": now + 60,
        "jti": format!("jti-{n:06}-hub-test"),
        "act": act,
    })
    .as_object()
    .unwrap()
    .clone()
}

/// A token for `claims` under `secret`, as decision 15 builds one.
fn sign(claims: &Map<String, Value>, secret: &str) -> String {
    format!(
        "v1.{}",
        crypto::sign(
            &STANDARD.decode(secret).unwrap(),
            CONTEXT,
            &serde_json::to_vec(claims).unwrap()
        )
    )
}

/// A good token for `act`.
fn token(act: &str) -> String {
    sign(&claims(act), SECRET)
}

/// A token for `act` with `edit` applied to its claims.
fn token_with(act: &str, edit: impl FnOnce(&mut Map<String, Value>)) -> String {
    let mut c = claims(act);
    edit(&mut c);
    sign(&c, SECRET)
}

/// The portal's sources, loaded: sign-in (for `/api/usage`'s session), the
/// mock control plane, and the eligibility knobs with `min_age` minutes.
fn loaded(mock: &MockGateway, min_age: &str) -> Loaded {
    Loaded {
        oauth: Some(Arc::new(oauth_secret())),
        gateway: Some(Arc::new(test_gateway(&mock.base))),
        settings: Some(Arc::new(eligibility(GUILD_ID, min_age))),
    }
}

fn config(portal_enabled: bool) -> AppConfig {
    AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![],
        portal_enabled,
        portal_oauth: None,
        portal_endpoints: Default::default(),
        portal_keys: None,
        portal_eligibility: None,
        portal_rate_limit: Some(RATE_LIMIT),
        portal_web_origin: Some(WEB_ORIGIN.to_string()),
    }
}

fn app_on(sources: PortalSources) -> Router {
    app_with_portal(&config(true), AppState::without_ch(), sources)
}

/// The whole router, the portal open, the hub's secret configured.
fn hub_app(mock: &MockGateway) -> Router {
    hub_app_with(mock, "5", HubSource::ready(Some(hub_secret())))
}

fn hub_app_with(mock: &MockGateway, min_age: &str, hub: HubSource) -> Router {
    app_on(PortalSources::ready(loaded(mock, min_age)).with_hub(hub))
}

/// The four routes alone, with a shortened deadline — for the timeout, which
/// `AppConfig` does not carry (as `keys_router_with_deadline`).
fn hub_router_with_deadline(mock: &MockGateway, deadline: Duration) -> Router {
    hub_routes(
        HubState::new(
            PortalSources::ready(loaded(mock, "5")).with_hub(HubSource::ready(Some(hub_secret()))),
            UsageState::new(None, None),
            Some(RATE_LIMIT),
        )
        .with_deadline(deadline),
    )
}

async fn hub_call(router: &Router, method: &str, path: &str, token: Option<&str>) -> Reply {
    hub_call_with(
        router,
        method,
        path,
        token.map(|t| format!("Bearer {t}")),
        &[],
    )
    .await
}

/// A call with `authorization` as given (or none) and `extra` headers.
async fn hub_call_with(
    router: &Router,
    method: &str,
    path: &str,
    authorization: Option<String>,
    extra: &[(&str, &str)],
) -> Reply {
    let mut headers: Vec<(&str, &str)> = extra.to_vec();
    if let Some(value) = authorization.as_deref() {
        headers.push(("authorization", value));
    }
    call_path_with(router.clone(), method, path, None, &headers).await
}

fn key_name() -> String {
    format!("discord-{USER_ID}-key")
}

fn rfc3339(secs: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs as i64, 0)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The error envelope's code, and that it is the whole envelope: a code and
/// words, no details, no stack.
fn envelope_code(reply: &Reply) -> String {
    let body = reply.json();
    let fields: Vec<&String> = body.as_object().unwrap().keys().collect();
    assert_eq!(fields, ["code", "message"], "{body}");
    body["code"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// GET account
// ---------------------------------------------------------------------------

#[tokio::test]
async fn account_without_a_key_is_none_and_can_issue() {
    let mock = MockGateway::start().await;
    let reply = hub_call(&hub_app(&mock), "GET", ACCOUNT_PATH, Some(&token("read"))).await;

    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.cache_control(), "no-store");
    assert_eq!(
        reply.json(),
        json!({
            "key": { "state": "none", "can_issue": true },
            "details": { "usage": null, "free_rate_limit_per_second": RATE_LIMIT },
        })
    );
    assert_eq!(
        mock.with(|s| (s.list_calls, s.plans_calls, s.usage_calls)),
        (1, 0, 0)
    );
}

/// One listing serves the key's state and its usage; the key's value is
/// never read — every `GetApiKey` would fail here, and the listing asks for
/// no values (`act=read` reads no credential).
#[tokio::test]
async fn account_with_a_key_lists_once_and_reads_no_value() {
    let mock = MockGateway::start().await;
    let created = now_secs() - 3_600;
    let id = mock.with(|s| {
        let id = s.seed_on_free_plan(&key_name(), created);
        s.usage.insert(id.clone(), vec![vec![120, 99_880]]);
        s.fail_reads = true;
        id
    });

    let reply = hub_call(&hub_app(&mock), "GET", ACCOUNT_PATH, Some(&token("read"))).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json());
    let body = reply.json();
    assert_eq!(
        body["key"],
        json!({
            "state": "active",
            "key_id": id,
            "created_at": rfc3339(created),
            "last_updated_at": rfc3339(created),
            "can_issue": false,
        })
    );
    let usage = &body["details"]["usage"];
    assert_eq!(usage["used"], 120);
    assert_eq!(usage["remaining"], 99_880);
    assert_eq!(usage["limit"], 100_000);
    assert_eq!(usage["plan"]["tier"], "free");
    assert_eq!(body["details"]["free_rate_limit_per_second"], RATE_LIMIT);
    assert!(!body.to_string().contains("CANARY"), "{body}");
    assert_eq!(
        mock.with(|s| (s.list_calls, s.plans_calls, s.usage_calls)),
        (1, 1, 1),
        "one listing for both halves"
    );
    assert_eq!(mock.with(|s| s.include_values_seen.clone()), ["false"]);
}

/// A key revoked this period: revoked, with both instants, no new key until
/// the next period — and its usage still shown (the counter is preserved).
#[tokio::test]
async fn account_with_a_key_revoked_this_period_is_capped() {
    let mock = MockGateway::start().await;
    let now = now_secs();
    mock.with(|s| {
        let id = s.seed_revoked(&key_name(), now - 3_600, now);
        s.plan_keys.push((PLAN_ID.to_string(), id));
    });

    let reply = hub_call(&hub_app(&mock), "GET", ACCOUNT_PATH, Some(&token("read"))).await;

    let body = reply.json();
    assert_eq!(
        body["key"],
        json!({
            "state": "revoked",
            "revoked_at": rfc3339(now),
            "next_eligible_at": Period::now().resets_at(),
            "can_issue": false,
        })
    );
    assert_eq!(body["details"]["usage"]["plan"]["tier"], "free");
}

/// Revoked in an earlier period: still revoked, and a new key can be had
/// now. No usage: the reveal and `/usage` say "no key" for it too.
#[tokio::test]
async fn account_with_a_key_revoked_last_period_can_issue() {
    let mock = MockGateway::start().await;
    let last_month = Period::now().start_secs() - 86_400;
    mock.with(|s| s.seed_revoked(&key_name(), last_month - 60, last_month));

    let reply = hub_call(&hub_app(&mock), "GET", ACCOUNT_PATH, Some(&token("read"))).await;

    assert_eq!(
        reply.json(),
        json!({
            "key": { "state": "revoked", "revoked_at": rfc3339(last_month), "can_issue": true },
            "details": { "usage": null, "free_rate_limit_per_second": RATE_LIMIT },
        })
    );
}

/// The usage is cached as `/api/usage`'s is: a second `account` inside the
/// TTL lists again (the key's state is never cached) and reads no usage.
#[tokio::test]
async fn account_usage_is_cached_like_the_portals() {
    let mock = MockGateway::start().await;
    mock.with(|s| s.seed_on_free_plan(&key_name(), now_secs()));
    let app = hub_app(&mock);

    let first = hub_call(&app, "GET", ACCOUNT_PATH, Some(&token("read"))).await;
    let second = hub_call(&app, "GET", ACCOUNT_PATH, Some(&token("read"))).await;

    assert_eq!(first.json(), second.json());
    assert_eq!(
        mock.with(|s| (s.list_calls, s.plans_calls, s.usage_calls)),
        (2, 1, 1)
    );
}

/// The usage half failing with nothing cached leaves the key's state
/// standing: `usage` is `null`, as the dashboard states no figure while
/// `/usage` fails.
#[tokio::test]
async fn account_without_its_usage_still_answers_the_key() {
    let mock = MockGateway::start().await;
    mock.with(|s| {
        s.seed_on_free_plan(&key_name(), now_secs());
        s.throttle_plans = true;
    });

    let reply = hub_call(&hub_app(&mock), "GET", ACCOUNT_PATH, Some(&token("read"))).await;

    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(body["key"]["state"], "active");
    assert_eq!(body["details"]["usage"], Value::Null);
}

// ---------------------------------------------------------------------------
// POST key/reveal
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reveal_answers_the_key_and_its_value() {
    let mock = MockGateway::start().await;
    let created = now_secs() - 60;
    let id = mock.with(|s| s.seed_on_free_plan(&key_name(), created));

    let reply = hub_call(&hub_app(&mock), "POST", REVEAL_PATH, Some(&token("reveal"))).await;

    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.cache_control(), "no-store");
    assert_eq!(
        reply.json(),
        json!({
            "key_id": id,
            "name": key_name(),
            "value": format!("VALUE-{id}-CANARY"),
            "created_at": rfc3339(created),
            "last_updated_at": rfc3339(created),
        })
    );
}

#[tokio::test]
async fn reveal_without_a_key_is_404_no_key() {
    let mock = MockGateway::start().await;
    let reply = hub_call(&hub_app(&mock), "POST", REVEAL_PATH, Some(&token("reveal"))).await;

    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(envelope_code(&reply), "no_key");
}

/// As the portal's reveal: `404 key_revoked`, with both instants in
/// `details` — the one code whose details the hub passes on.
#[tokio::test]
async fn reveal_of_a_key_revoked_this_period_is_404_key_revoked() {
    let mock = MockGateway::start().await;
    let now = now_secs();
    mock.with(|s| s.seed_revoked(&key_name(), now - 60, now));

    let reply = hub_call(&hub_app(&mock), "POST", REVEAL_PATH, Some(&token("reveal"))).await;

    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let body = reply.json();
    assert_eq!(body["code"], "key_revoked");
    assert_eq!(
        body["details"],
        json!({ "next_eligible_at": Period::now().resets_at(), "revoked_at": rfc3339(now) })
    );
}

/// Revoked in an earlier period: nothing to show and nothing to wait for —
/// `404 no_key`, as the portal's reveal says, and nothing is written.
#[tokio::test]
async fn reveal_of_a_key_revoked_in_an_earlier_period_is_404_no_key() {
    let mock = MockGateway::start().await;
    let last_month = Period::now().start_secs() - 86_400;
    mock.with(|s| s.seed_revoked(&key_name(), last_month - 60, last_month));

    let reply = hub_call(&hub_app(&mock), "POST", REVEAL_PATH, Some(&token("reveal"))).await;

    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.cache_control(), "no-store");
    assert_eq!(envelope_code(&reply), "no_key");
    assert!(mock.with(|s| s.ops.is_empty()));
}

// ---------------------------------------------------------------------------
// POST key/revoke
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_disables_the_key_and_names_the_next_period() {
    let mock = MockGateway::start().await;
    let now = now_secs();
    let id = mock.with(|s| {
        s.disable_stamps_at = Some(now);
        s.seed_on_free_plan(&key_name(), now - 60)
    });

    let reply = hub_call(&hub_app(&mock), "POST", REVOKE_PATH, Some(&token("revoke"))).await;

    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.cache_control(), "no-store");
    assert_eq!(
        reply.json(),
        json!({
            "revoked": true,
            "next_eligible_at": Period::now().resets_at(),
            "revoked_at": rfc3339(now),
        })
    );
    assert_eq!(mock.with(|s| s.ops.clone()), [format!("disable:{id}")]);
}

/// No `Origin`, no marker header, no cookie: the token is the whole of the
/// authorization, as it is for every route here.
#[tokio::test]
async fn revoke_without_a_key_is_404_no_key_and_writes_nothing() {
    let mock = MockGateway::start().await;
    let reply = hub_call(&hub_app(&mock), "POST", REVOKE_PATH, Some(&token("revoke"))).await;

    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(envelope_code(&reply), "no_key");
    assert!(mock.with(|s| s.ops.is_empty()));
}

// ---------------------------------------------------------------------------
// POST key/issue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn issue_creates_a_key_the_hub_issued() {
    let mock = MockGateway::start().await;
    let reply = hub_call(&hub_app(&mock), "POST", ISSUE_PATH, Some(&token("issue"))).await;

    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.cache_control(), "no-store");
    assert_eq!(reply.json(), json!({ "outcome": "issued" }));
    let keys = mock.with(|s| s.keys.clone());
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].name, key_name());
    assert!(keys[0].enabled);
    assert_eq!(keys[0].tags["IssuedBy"], "hub-contract");
    assert_eq!(keys[0].tags["ManagedBy"], "prices-portal");
    assert_eq!(
        mock.with(|s| s.plan_keys.clone()),
        [(PLAN_ID.to_string(), keys[0].id.clone())]
    );
}

/// One active key per account: a second issue adopts the first.
#[tokio::test]
async fn issue_with_a_key_already_is_has_key() {
    let mock = MockGateway::start().await;
    mock.with(|s| s.seed_on_free_plan(&key_name(), now_secs() - 60));

    let reply = hub_call(&hub_app(&mock), "POST", ISSUE_PATH, Some(&token("issue"))).await;

    assert_eq!(reply.json(), json!({ "outcome": "has_key" }));
    assert_eq!(mock.with(|s| s.create_calls), 0);
}

/// The month cap after a revoke holds for the hub as for the portal.
#[tokio::test]
async fn issue_after_a_revoke_this_period_is_capped() {
    let mock = MockGateway::start().await;
    let now = now_secs();
    mock.with(|s| s.seed_revoked(&key_name(), now - 60, now));

    let reply = hub_call(&hub_app(&mock), "POST", ISSUE_PATH, Some(&token("issue"))).await;

    assert_eq!(
        reply.json(),
        json!({ "outcome": "capped", "next_eligible_at": Period::now().resets_at() })
    );
    assert!(mock.with(|s| s.ops.is_empty()));
}

/// Revoked in an earlier period, the cap has rolled: a new key, as the
/// portal's issue does since task 0311 — on the plan the revoked key was on
/// (Basic here, not free), created and attached before the revoked record is
/// deleted, and tagged as the hub's.
#[tokio::test]
async fn issue_after_a_revoke_in_an_earlier_period_keeps_the_previous_plan() {
    let mock = MockGateway::start().await;
    let last_month = Period::now().start_secs() - 86_400;
    let dead = mock.with(|s| {
        s.plans.push(StoredPlan::basic());
        let id = s.seed_revoked(&key_name(), last_month - 60, last_month);
        s.plan_keys.push((BASIC_PLAN_ID.to_string(), id.clone()));
        id
    });

    let reply = hub_call(&hub_app(&mock), "POST", ISSUE_PATH, Some(&token("issue"))).await;

    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json!({ "outcome": "issued" }));
    mock.with(|s| {
        assert_eq!(s.keys.len(), 1, "the revoked record is gone: {:?}", s.ops);
        let new = s.keys[0].id.clone();
        assert_ne!(new, dead);
        assert!(s.keys[0].enabled);
        assert_eq!(s.keys[0].tags["IssuedBy"], "hub-contract");
        assert!(
            s.plan_keys
                .contains(&(BASIC_PLAN_ID.to_string(), new.clone())),
            "the new key keeps the previous key's plan: {:?}",
            s.plan_keys
        );
        assert!(
            !s.plan_keys.contains(&(PLAN_ID.to_string(), new.clone())),
            "the new key must not land on free"
        );
        assert_eq!(
            s.ops,
            [
                format!("create:{new}"),
                format!("attach:{new}"),
                format!("delete:{dead}")
            ]
        );
    });
}

/// The age backstop runs first, against our own threshold, and asks AWS
/// nothing: `too_young`, with the instant the account clears it.
#[tokio::test]
async fn issue_for_an_account_too_young_asks_nothing_of_aws() {
    let mock = MockGateway::start().await;
    let min_age: u64 = 100_000_000; // minutes: about 190 years
    let app = hub_app_with(
        &mock,
        &min_age.to_string(),
        HubSource::ready(Some(hub_secret())),
    );

    let reply = hub_call(&app, "POST", ISSUE_PATH, Some(&token("issue"))).await;

    let created_ms = prices_api::portal::eligibility::account_created_ms(USER_ID).unwrap();
    let clears = (created_ms + min_age * 60_000).div_ceil(1000);
    assert_eq!(
        reply.json(),
        json!({ "outcome": "too_young", "next_eligible_at": rfc3339(clears) })
    );
    assert_eq!(mock.with(|s| s.list_calls), 0);
}

/// A threshold that cannot be read is our fault, and refuses: `failed`,
/// before any control-plane call.
#[tokio::test]
async fn issue_without_a_readable_age_threshold_is_failed() {
    let mock = MockGateway::start().await;
    let app = hub_app_with(&mock, "five", HubSource::ready(Some(hub_secret())));

    let reply = hub_call(&app, "POST", ISSUE_PATH, Some(&token("issue"))).await;

    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json!({ "outcome": "failed" }));
    assert_eq!(mock.with(|s| s.list_calls), 0);
}

#[tokio::test]
async fn issue_on_a_failing_control_plane_is_failed() {
    let mock = MockGateway::start().await;
    mock.with(|s| s.fail_list = true);

    let reply = hub_call(&hub_app(&mock), "POST", ISSUE_PATH, Some(&token("issue"))).await;

    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json!({ "outcome": "failed" }));
}

// ---------------------------------------------------------------------------
// The token
// ---------------------------------------------------------------------------

/// Review Focus 1 through the router: every refused token, on every route,
/// is the same `401 invalid_token` — and nothing is asked of AWS.
#[tokio::test]
async fn a_missing_malformed_or_refused_token_is_401_with_no_aws_call() {
    let mock = MockGateway::start().await;
    mock.with(|s| s.seed_on_free_plan(&key_name(), now_secs()));
    let app = hub_app(&mock);
    let now = now_secs();

    for (_, path, act) in ROUTES {
        let good = token(act);
        let headers: Vec<Option<String>> = vec![
            None,
            Some(String::new()),
            Some("Bearer".into()),
            Some("Bearer ".into()),
            Some(format!("Basic {good}")),
            Some(format!("Token {good}")),
            Some(format!("Bearer  {good}")),
            Some(format!("Bearer {good} extra")),
            Some(good.clone()),
            Some("Bearer v1.not.a-token".into()),
            Some(format!("Bearer {}", good.replacen("v1.", "v2.", 1))),
            Some(format!("Bearer {}", sign(&claims(act), OTHER_SECRET))),
            Some(format!(
                "Bearer {}",
                token_with(act, |c| {
                    c.insert("iat".into(), json!(now - 120));
                    c.insert("exp".into(), json!(now - 60));
                })
            )),
            Some(format!(
                "Bearer {}",
                token_with(act, |c| {
                    c.insert("exp".into(), json!(now + 3_600));
                })
            )),
            Some(format!(
                "Bearer {}",
                token_with(act, |c| {
                    c.insert("aud".into(), json!("x402"));
                })
            )),
            Some(format!(
                "Bearer {}",
                token_with(act, |c| {
                    c.insert("sub".into(), json!(format!("discord:0{USER_ID}")));
                })
            )),
        ];
        for authorization in headers {
            let method = if path == ACCOUNT_PATH { "GET" } else { "POST" };
            let reply = hub_call_with(&app, method, path, authorization.clone(), &[]).await;
            assert_eq!(
                reply.status,
                StatusCode::UNAUTHORIZED,
                "{path} with {authorization:?}"
            );
            assert_eq!(envelope_code(&reply), "invalid_token", "{path}");
            assert_eq!(reply.cache_control(), "no-store");
        }
    }
    assert_eq!(
        mock.with(|s| s.list_calls),
        0,
        "a refused token reached AWS"
    );
    assert!(mock.with(|s| s.ops.is_empty()));
}

/// The scheme's case is not the token's: `bearer` is `Bearer` (RFC 9110).
#[tokio::test]
async fn the_bearer_scheme_is_case_insensitive() {
    let mock = MockGateway::start().await;
    let reply = hub_call_with(
        &hub_app(&mock),
        "GET",
        ACCOUNT_PATH,
        Some(format!("bearer {}", token("read"))),
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
}

/// Review Focus 2: a `read` token cannot revoke or issue — `403
/// action_not_allowed`, nothing asked of AWS, nothing written. And every
/// route refuses every `act` but its own.
#[tokio::test]
async fn a_token_for_another_act_is_403_with_no_aws_call() {
    let mock = MockGateway::start().await;
    mock.with(|s| s.seed_on_free_plan(&key_name(), now_secs()));
    let app = hub_app(&mock);

    for path in [REVOKE_PATH, ISSUE_PATH] {
        let reply = hub_call(&app, "POST", path, Some(&token("read"))).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(envelope_code(&reply), "action_not_allowed");
        assert_eq!(reply.cache_control(), "no-store");
    }
    for (method, path, needs) in ROUTES {
        for act in ACTS.into_iter().filter(|act| *act != needs) {
            let reply = hub_call(&app, method, path, Some(&token(act))).await;
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{act} on {path}");
            assert_eq!(envelope_code(&reply), "action_not_allowed");
        }
    }

    assert_eq!(mock.with(|s| s.list_calls), 0, "a wrong act reached AWS");
    assert!(mock.with(|s| s.ops.is_empty()));
    assert_eq!(mock.with(|s| s.create_calls), 0);
}

// ---------------------------------------------------------------------------
// What closes the contract, and what does not
// ---------------------------------------------------------------------------

async fn config_enabled(app: &Router) -> bool {
    let reply = call_path(app.clone(), "GET", "/api/config", None).await;
    assert_eq!(reply.status, StatusCode::OK);
    reply.json()["enabled"] == true
}

/// No hub secret, or one that does not parse: the contract answers `503
/// hub_unconfigured` on every route, and the portal is open.
#[tokio::test]
async fn a_missing_or_malformed_secret_closes_the_contract_and_not_the_portal() {
    let mock = MockGateway::start().await;
    let malformed: HubLoader = Arc::new(|| Box::pin(async { HubSecret::parse("{}").map(Some) }));
    for hub in [HubSource::ready(None), HubSource::lazy(malformed)] {
        let app = hub_app_with(&mock, "5", hub);
        for (method, path, act) in ROUTES {
            let reply = hub_call(&app, method, path, Some(&token(act))).await;
            assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
            assert_eq!(envelope_code(&reply), "hub_unconfigured");
            assert_eq!(reply.cache_control(), "no-store");
        }
        assert!(config_enabled(&app).await, "a hub secret closed the portal");
    }
    assert_eq!(mock.with(|s| s.list_calls), 0);
}

/// The portal's own sources unwired: `503 keys_unconfigured`, the portal's
/// code for it.
#[tokio::test]
async fn unwired_key_sources_are_503_keys_unconfigured() {
    let app = app_on(
        PortalSources::ready(Loaded::default()).with_hub(HubSource::ready(Some(hub_secret()))),
    );
    for (method, path, act) in ROUTES {
        let reply = hub_call(&app, method, path, Some(&token(act))).await;
        assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert_eq!(envelope_code(&reply), "keys_unconfigured");
    }
}

/// While the portal is closed the contract is the gate's empty `404`, like
/// every path under the prefix.
#[tokio::test]
async fn a_closed_portal_hides_the_contract() {
    let mock = MockGateway::start().await;
    let app = app_with_portal(
        &config(false),
        AppState::without_ch(),
        PortalSources::ready(loaded(&mock, "5")).with_hub(HubSource::ready(Some(hub_secret()))),
    );
    for (method, path, act) in ROUTES {
        let reply = hub_call(&app, method, path, Some(&token(act))).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert!(reply.body.is_empty(), "{path}");
    }
    assert_eq!(mock.with(|s| s.list_calls), 0);
}

#[tokio::test]
async fn the_contract_is_not_in_the_openapi_document() {
    let mock = MockGateway::start().await;
    let reply = call_path(hub_app(&mock), "GET", "/api-docs-json", None).await;
    assert_eq!(reply.status, StatusCode::OK);
    let document = String::from_utf8(reply.body).unwrap();
    assert!(document.contains("/v1/"), "not the OpenAPI document");
    assert!(!document.contains("/api/hub"));
}

// ---------------------------------------------------------------------------
// Review Focus 4: the control plane failing
// ---------------------------------------------------------------------------

/// A failing control plane is the portal's `502 key_unavailable` on the
/// three routes that answer with a status — a code and words, no token, no
/// error text. `issue` answers its own `failed` (above).
#[tokio::test]
async fn a_failing_control_plane_is_502_key_unavailable() {
    let mock = MockGateway::start().await;
    mock.with(|s| s.fail_list = true);
    let app = hub_app(&mock);

    for (method, path, act) in &ROUTES[..3] {
        let token = token(act);
        let reply = hub_call(&app, method, path, Some(&token)).await;
        assert_eq!(reply.status, StatusCode::BAD_GATEWAY, "{path}");
        assert_eq!(envelope_code(&reply), "key_unavailable", "{path}");
        assert_eq!(reply.cache_control(), "no-store");
        let text = String::from_utf8_lossy(&reply.body).to_string();
        for leaked in [
            token.as_str(),
            "API Gateway",
            "GetApiKeys",
            "unwell",
            USER_ID,
        ] {
            assert!(!text.contains(leaked), "{path} answered {text}");
        }
    }
}

/// A slow control plane is the portal's `503 key_unavailable`, inside the
/// deadline; `issue` answers `failed`.
#[tokio::test]
async fn a_slow_control_plane_is_503_key_unavailable() {
    let mock = MockGateway::start().await;
    mock.with(|s| {
        s.seed_on_free_plan(&key_name(), now_secs());
        s.list_delay_ms = 1_000;
    });
    let router = hub_router_with_deadline(&mock, Duration::from_millis(50));

    for (method, path, act) in &ROUTES[..3] {
        let reply = hub_call(&router, method, path, Some(&token(act))).await;
        assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert_eq!(envelope_code(&reply), "key_unavailable", "{path}");
        assert_eq!(reply.cache_control(), "no-store");
    }
    let issued = hub_call(&router, "POST", ISSUE_PATH, Some(&token("issue"))).await;
    assert_eq!(issued.json(), json!({ "outcome": "failed" }));
    assert!(mock.with(|s| s.ops.is_empty()));
}

// ---------------------------------------------------------------------------
// Review Focus 5: a hub write evicts the account's cached usage
// ---------------------------------------------------------------------------

/// The portal's `/api/usage`, as the account's session would call it.
async fn portal_usage(app: &Router) -> Reply {
    call_path(
        app.clone(),
        "GET",
        "/api/usage",
        Some(&session_cookie(USER_ID)),
    )
    .await
}

/// `account` caches "no key"; a hub issue makes it false and evicts it, so
/// the portal's `/usage` — which has no listing to check it against — shows
/// the new key at once, not after the TTL.
#[tokio::test]
async fn a_hub_issue_evicts_a_cached_no_key() {
    let mock = MockGateway::start().await;
    let app = hub_app(&mock);

    let before = hub_call(&app, "GET", ACCOUNT_PATH, Some(&token("read"))).await;
    assert_eq!(before.json()["details"]["usage"], Value::Null);
    let issued = hub_call(&app, "POST", ISSUE_PATH, Some(&token("issue"))).await;
    assert_eq!(issued.json(), json!({ "outcome": "issued" }));

    let usage = portal_usage(&app).await;
    assert_eq!(usage.status, StatusCode::OK, "{}", usage.json());
    assert_eq!(usage.json()["plan"]["tier"], "free");

    let after = hub_call(&app, "GET", ACCOUNT_PATH, Some(&token("read"))).await;
    assert_eq!(after.json()["key"]["state"], "active");
    assert_eq!(after.json()["details"]["usage"]["plan"]["tier"], "free");
}

/// `account` caches "no key"; a key made elsewhere (the portal's own
/// round-trip, or another execution environment) and then revealed through
/// the hub proves it false, and the reveal evicts it, so the portal's
/// `/usage` shows the key inside the TTL.
#[tokio::test]
async fn a_hub_reveal_evicts_a_cached_no_key() {
    let mock = MockGateway::start().await;
    let app = hub_app(&mock);

    let before = hub_call(&app, "GET", ACCOUNT_PATH, Some(&token("read"))).await;
    assert_eq!(before.json()["key"]["state"], "none");
    assert_eq!(before.json()["details"]["usage"], Value::Null);

    // The key appears, created in another request.
    mock.with(|s| s.seed_on_free_plan(&key_name(), now_secs() - 60));

    let revealed = hub_call(&app, "POST", REVEAL_PATH, Some(&token("reveal"))).await;
    assert_eq!(revealed.status, StatusCode::OK);

    // Inside the 60 s TTL: only the eviction explains a key here.
    let usage = portal_usage(&app).await;
    assert_eq!(usage.status, StatusCode::OK, "{}", usage.json());
    assert_eq!(usage.json()["plan"]["tier"], "free");
}

/// `account` caches the key's usage; a hub revoke evicts it, so the next
/// `account` and `/usage` read the revoked key afresh.
#[tokio::test]
async fn a_hub_revoke_evicts_the_cached_usage() {
    let mock = MockGateway::start().await;
    mock.with(|s| s.seed_on_free_plan(&key_name(), now_secs() - 60));
    let app = hub_app(&mock);

    let before = hub_call(&app, "GET", ACCOUNT_PATH, Some(&token("read"))).await;
    assert_eq!(before.json()["key"]["state"], "active");
    assert_eq!(mock.with(|s| s.plans_calls), 1);

    let revoked = hub_call(&app, "POST", REVOKE_PATH, Some(&token("revoke"))).await;
    assert_eq!(revoked.status, StatusCode::OK);

    let after = hub_call(&app, "GET", ACCOUNT_PATH, Some(&token("read"))).await;
    assert_eq!(after.json()["key"]["state"], "revoked");
    assert_eq!(
        mock.with(|s| s.plans_calls),
        2,
        "the revoked key's usage was served from the cache"
    );

    let plans_before = mock.with(|s| s.plans_calls);
    assert_eq!(portal_usage(&app).await.status, StatusCode::OK);
    assert_eq!(
        mock.with(|s| s.plans_calls),
        plans_before,
        "/usage shares it"
    );
}

// ---------------------------------------------------------------------------
// Every answer: no-store, no CORS
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_answer_is_no_store() {
    let mock = MockGateway::start().await;
    let app = hub_app(&mock);
    let unconfigured = hub_app_with(&mock, "5", HubSource::ready(None));

    let replies = [
        hub_call(&app, "GET", ACCOUNT_PATH, Some(&token("read"))).await,
        hub_call(&app, "POST", REVEAL_PATH, Some(&token("reveal"))).await,
        hub_call(&app, "POST", REVOKE_PATH, Some(&token("revoke"))).await,
        hub_call(&app, "POST", ISSUE_PATH, Some(&token("issue"))).await,
        hub_call(&app, "POST", REVEAL_PATH, Some(&token("reveal"))).await,
        hub_call(&app, "GET", ACCOUNT_PATH, None).await,
        hub_call(&app, "GET", ACCOUNT_PATH, Some(&token("issue"))).await,
        hub_call(&app, "GET", REVEAL_PATH, Some(&token("reveal"))).await,
        hub_call(&app, "POST", ACCOUNT_PATH, Some(&token("read"))).await,
        hub_call(&unconfigured, "GET", ACCOUNT_PATH, Some(&token("read"))).await,
    ];
    let statuses: Vec<u16> = replies.iter().map(|r| r.status.as_u16()).collect();
    assert_eq!(
        statuses,
        [200, 404, 404, 200, 200, 401, 403, 405, 405, 503],
        "the replies this test means to cover"
    );
    for reply in &replies {
        assert_eq!(reply.cache_control(), "no-store", "{}", reply.status);
    }
}

/// No browser calls the contract, so no origin is granted it — not even the
/// bundle's, which the portal's own routes grant (asserted alongside, so this
/// cannot pass on a deployment that grants nobody).
#[tokio::test]
async fn the_contract_answers_no_cors_headers() {
    let mock = MockGateway::start().await;
    let app = hub_app(&mock);
    let origin = [("origin", WEB_ORIGIN)];

    for (method, path, act) in ROUTES {
        let reply = hub_call_with(
            &app,
            method,
            path,
            Some(format!("Bearer {}", token(act))),
            &origin,
        )
        .await;
        for header in [
            "access-control-allow-origin",
            "access-control-allow-credentials",
        ] {
            assert!(reply.headers.get(header).is_none(), "{header} on {path}");
        }
    }
    let preflight = hub_call_with(
        &app,
        "OPTIONS",
        REVOKE_PATH,
        None,
        &[
            ("origin", WEB_ORIGIN),
            ("access-control-request-method", "POST"),
            ("access-control-request-headers", "authorization"),
        ],
    )
    .await;
    assert!(
        preflight
            .headers
            .get("access-control-allow-origin")
            .is_none()
    );

    let portal = call_path_with(app.clone(), "GET", "/api/usage", None, &origin).await;
    assert_eq!(portal.headers["access-control-allow-origin"], WEB_ORIGIN);
}

// ---------------------------------------------------------------------------
// The two loads, concurrently, inside the invocation
// ---------------------------------------------------------------------------

/// A portal load that never finishes.
fn hung_portal() -> Loader {
    Arc::new(|| Box::pin(std::future::pending()))
}

/// A hub secret read that never finishes.
fn hung_hub() -> HubLoader {
    Arc::new(|| Box::pin(std::future::pending()))
}

/// Both reads hung: each is cut at its own budget, awaited together, so the
/// answer comes at the slower budget (4 s), not their sum — well inside the
/// 15 s invocation.
#[tokio::test(start_paused = true)]
async fn with_both_reads_hung_the_answer_comes_inside_the_invocation() {
    let app = app_on(PortalSources::lazy(hung_portal()).with_hub(HubSource::lazy(hung_hub())));

    let started = tokio::time::Instant::now();
    let reply = hub_call(&app, "GET", ACCOUNT_PATH, Some(&token("read"))).await;
    let waited = started.elapsed();

    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(envelope_code(&reply), "hub_unconfigured");
    assert!(
        waited >= Duration::from_secs(3) && waited <= Duration::from_secs(4),
        "awaited one after the other, or not at all: {waited:?}"
    );
    assert!(waited < Duration::from_secs(15));
}

/// The portal's load hung, the hub's secret fine: `503 keys_unconfigured`
/// at the portal's budget.
#[tokio::test(start_paused = true)]
async fn a_hung_portal_load_is_503_keys_unconfigured_inside_its_budget() {
    let app =
        app_on(PortalSources::lazy(hung_portal()).with_hub(HubSource::ready(Some(hub_secret()))));

    let started = tokio::time::Instant::now();
    let reply = hub_call(&app, "GET", ACCOUNT_PATH, Some(&token("read"))).await;

    assert_eq!(envelope_code(&reply), "keys_unconfigured");
    assert!(started.elapsed() <= Duration::from_secs(4));
}

/// The hub's read fails before the portal's load finishes: the contract
/// still waits for the portal's load rather than dropping it — a dropped
/// load counts as a failed one and would close `/api/config` for the
/// cooldown.
#[tokio::test(start_paused = true)]
async fn the_portal_load_is_never_cut_short_by_the_hubs() {
    let mock_loaded = Loaded::default();
    let portal: Loader = Arc::new(move || {
        let loaded = mock_loaded.clone();
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(3_500)).await;
            Ok(loaded)
        })
    });
    let app = app_on(PortalSources::lazy(portal).with_hub(HubSource::lazy(hung_hub())));

    let reply = hub_call(&app, "GET", ACCOUNT_PATH, Some(&token("read"))).await;
    assert_eq!(envelope_code(&reply), "hub_unconfigured");

    assert!(
        config_enabled(&app).await,
        "the portal's load was dropped and recorded as a failure"
    );
}

/// A refused token wins over a failed portal load: `401`, whatever else
/// happened.
#[tokio::test]
async fn a_bad_token_is_401_even_when_the_portal_did_not_load() {
    let failing: Loader = Arc::new(|| {
        Box::pin(async {
            Err(prices_api::config::PortalLoadError::TimedOut(
                Duration::from_secs(4),
            ))
        })
    });
    let app = app_on(PortalSources::lazy(failing).with_hub(HubSource::ready(Some(hub_secret()))));

    let reply = hub_call(&app, "GET", ACCOUNT_PATH, Some("v1.forged.token")).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}
