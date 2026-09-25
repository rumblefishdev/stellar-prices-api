//! The portal's sources load on the first portal request, not at cold start
//! (task 0311).
//!
//! Driven through the real router (`prices_api::app_with_portal`) with a
//! counting, scripted loader in place of the environment one. Pinned here:
//!
//! - building the router and serving `/health` and `/v1` read nothing;
//! - `/config` loads, answers `enabled` by the outcome, and a success is kept;
//! - a failure is kept only for `LOAD_FAILURE_COOLDOWN`: inside it a request
//!   answers unavailable without loading, and the first after it loads again;
//! - a portal route answers `503` on a failed load and works after the cooldown;
//! - with `PORTAL_ENABLED=false` the portal is a `404` and nothing loads;
//! - concurrent first requests share one successful load;
//! - login on a failed load lands on a retryable failure;
//! - a callback on a failed load lands where its flow renders a failure (the
//!   cases with a real login, and the slow load, are in `tests/portal_auth.rs`);
//! - `main.rs` performs no portal load.
//!
//! The tests that load twice run on tokio's paused clock, so the cooldown is
//! skipped with `advance` rather than slept through.
//!
//! The retry around each read is unit-tested in `portal::extension`; the log
//! lines a failed load and its cooldown write are in `tests/portal_load_logs.rs`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use prices_api::config::PortalLoadError;
use prices_api::portal::auth::secret::{OauthSecret, SecretError};
use prices_api::portal::keys::gateway::Gateway;
use prices_api::portal::sources::{LOAD_FAILURE_COOLDOWN, Loaded, Loader, PortalSources};
use prices_api::{AppConfig, AppState, app_with_portal};
use serde_json::{Value, json};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn oauth_secret() -> OauthSecret {
    OauthSecret::parse(
        &json!({
            "client_id": "a-client-id",
            "client_secret": "the-client-secret",
            "redirect_uri": "https://portal.example/api/auth/callback",
            "session_signing_key":
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        })
        .to_string(),
    )
    .expect("the test bundle must be valid")
}

/// Never contacted: every request in this file stops before the control
/// plane (no session cookie, or no load).
fn gateway() -> Gateway {
    Gateway::against(
        "http://127.0.0.1:9",
        "plan-free".to_string(),
        "api-id".to_string(),
        "production".to_string(),
    )
}

fn loaded_with_keys() -> Loaded {
    Loaded {
        oauth: Some(Arc::new(oauth_secret())),
        gateway: Some(Arc::new(gateway())),
        settings: None,
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
        portal_rate_limit: None,
        portal_web_origin: None,
    }
}

/// What one scripted attempt does.
#[derive(Clone)]
enum Step {
    Fail,
    Succeed(Loaded),
}

/// A loader that plays `script` (repeating the last step once it runs out),
/// each attempt after `delay`, counting its calls.
fn scripted(script: Vec<Step>, delay: Duration) -> (PortalSources, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let steps = Arc::new(Mutex::new(script.into_iter().collect::<VecDeque<_>>()));
    let last = Arc::new(Mutex::new(Step::Fail));
    let counter = calls.clone();
    let loader: Loader = Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        let step = {
            let mut last = last.lock().unwrap();
            if let Some(step) = steps.lock().unwrap().pop_front() {
                *last = step;
            }
            last.clone()
        };
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            match step {
                Step::Fail => Err(PortalLoadError::Oauth(SecretError::NoSource)),
                Step::Succeed(loaded) => Ok(loaded),
            }
        })
    });
    (PortalSources::lazy(loader), calls)
}

fn router(portal_enabled: bool, sources: PortalSources) -> Router {
    app_with_portal(&config(portal_enabled), AppState::without_ch(), sources)
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("the body should be JSON")
    }

    fn no_store(&self) -> bool {
        self.headers
            .get(header::CACHE_CONTROL)
            .is_some_and(|v| v.to_str().unwrap().contains("no-store"))
    }

    fn location(&self) -> String {
        self.headers
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default()
    }
}

async fn get(router: &Router, uri: &str) -> Reply {
    let response = router
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
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

// ---------------------------------------------------------------------------
// The cold-start path reads nothing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_router_health_and_v1_load_nothing_and_config_loads_once() {
    let (sources, calls) = scripted(vec![Step::Succeed(Loaded::default())], Duration::ZERO);
    let router = router(true, sources);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "building the router loaded"
    );

    assert_eq!(get(&router, "/health").await.status, StatusCode::OK);
    // A real `/v1` handler, reached without ClickHouse: `post_batch` refuses
    // an empty list itself, before `state.ch()`.
    let v1 = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/prices/batch")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"assets": []}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(v1.status(), StatusCode::BAD_REQUEST);
    assert_eq!(calls.load(Ordering::SeqCst), 0, "/health or /v1 loaded");

    let config = get(&router, "/api/config").await;
    assert_eq!(config.status, StatusCode::OK);
    assert_eq!(config.json()["enabled"], json!(true));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let again = get(&router, "/api/config").await;
    assert_eq!(again.json()["enabled"], json!(true));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "a success must be kept");
}

/// `main.rs` cannot be run from a test (`required-features = ["lambda"]`), so
/// its source is the evidence: no portal load before the router is built.
/// `client_from_lambda_env` is asserted too, so this cannot pass on an empty
/// or moved file.
#[test]
fn main_rs_performs_no_portal_load() {
    let main = include_str!("../src/main.rs");
    assert!(!main.contains("load_portal"), "main.rs loads the portal");
    assert!(
        !main.contains("PortalSources"),
        "main.rs builds portal sources"
    );
    assert!(main.contains("client_from_lambda_env"));
    assert!(main.contains("let config = AppConfig::from_env();"));
}

// ---------------------------------------------------------------------------
// /config answers by the load's outcome; a failure is kept for the cooldown
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn config_says_closed_on_a_failed_load_and_open_after_the_cooldown() {
    let (sources, calls) = scripted(
        vec![Step::Fail, Step::Succeed(Loaded::default())],
        Duration::ZERO,
    );
    let router = router(true, sources);

    let failed = get(&router, "/api/config").await;
    assert_eq!(failed.status, StatusCode::OK);
    assert_eq!(failed.json()["enabled"], json!(false));
    assert!(failed.no_store(), "a closed answer must never be cached");

    // Inside the cooldown (review WR-02): the same answer, and no load — an
    // anonymous page view must not re-run five retried reads against a
    // throttled SSM.
    let cooling = get(&router, "/api/config").await;
    assert_eq!(cooling.json()["enabled"], json!(false));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a load ran inside the cooldown"
    );

    tokio::time::advance(LOAD_FAILURE_COOLDOWN).await;
    let recovered = get(&router, "/api/config").await;
    assert_eq!(recovered.json()["enabled"], json!(true));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "the failure outlived the cooldown"
    );
}

// ---------------------------------------------------------------------------
// Portal routes: 503 on a failed load, working on the next
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn usage_is_503_on_a_failed_load_and_answers_after_the_cooldown() {
    let (sources, _) = scripted(
        vec![Step::Fail, Step::Succeed(loaded_with_keys())],
        Duration::ZERO,
    );
    let router = router(true, sources);

    let failed = get(&router, "/api/usage").await;
    assert_eq!(failed.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(failed.json()["code"], json!("usage_unconfigured"));
    assert!(failed.no_store());

    // Past the unprovisioned branch: no session cookie is now the answer.
    tokio::time::advance(LOAD_FAILURE_COOLDOWN).await;
    let next = get(&router, "/api/usage").await;
    assert_eq!(next.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test(start_paused = true)]
async fn key_is_503_on_a_failed_load_and_answers_after_the_cooldown() {
    let (sources, _) = scripted(
        vec![Step::Fail, Step::Succeed(loaded_with_keys())],
        Duration::ZERO,
    );
    let router = router(true, sources);

    let failed = get(&router, "/api/key").await;
    assert_eq!(failed.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(failed.json()["code"], json!("keys_unconfigured"));
    assert!(failed.no_store());

    tokio::time::advance(LOAD_FAILURE_COOLDOWN).await;
    let next = get(&router, "/api/key").await;
    assert_eq!(next.status, StatusCode::UNAUTHORIZED);
}

/// `/me` does not say "signed out" when it cannot check: that would be a lie
/// to a visitor who is signed in.
#[tokio::test(start_paused = true)]
async fn me_is_503_on_a_failed_load_and_answers_after_the_cooldown() {
    let (sources, _) = scripted(
        vec![Step::Fail, Step::Succeed(loaded_with_keys())],
        Duration::ZERO,
    );
    let router = router(true, sources);

    let failed = get(&router, "/api/auth/me").await;
    assert_eq!(failed.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(failed.json()["code"], json!("session_unavailable"));
    assert!(failed.no_store());

    tokio::time::advance(LOAD_FAILURE_COOLDOWN).await;
    let next = get(&router, "/api/auth/me").await;
    assert_eq!(next.status, StatusCode::OK);
    assert_eq!(next.json()["authenticated"], json!(false));
}

// ---------------------------------------------------------------------------
// PORTAL_ENABLED=false: a 404, and nothing ever loads
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_closed_portal_is_a_404_and_never_loads() {
    let (sources, calls) = scripted(vec![Step::Succeed(loaded_with_keys())], Duration::ZERO);
    let router = router(false, sources);

    for path in ["/api/key", "/api/usage", "/api/auth/me"] {
        let reply = get(&router, path).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert!(reply.body.is_empty(), "{path} carried a body");
    }
    let config = get(&router, "/api/config").await;
    assert_eq!(config.json()["enabled"], json!(false));
    assert_eq!(calls.load(Ordering::SeqCst), 0, "a closed portal loaded");
}

// ---------------------------------------------------------------------------
// Single flight
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_first_requests_share_one_load() {
    let (sources, calls) = scripted(
        vec![Step::Succeed(Loaded::default())],
        Duration::from_millis(50),
    );
    let router = router(true, sources);

    let mut set = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let router = router.clone();
        set.spawn(async move { get(&router, "/api/config").await.json()["enabled"].clone() });
    }
    while let Some(enabled) = set.join_next().await {
        assert_eq!(enabled.unwrap(), json!(true));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------
// The sign-in callback
// ---------------------------------------------------------------------------

/// A callback that claims no action — no pending cookie, a `state` that is
/// not one of ours — lands on sign-in's failure, and the pending cookie is
/// left alone as on every refusal before verification. The issue and
/// sign-in round-trips on a failed load, with real tokens, are in
/// `tests/portal_auth.rs`.
#[tokio::test]
async fn a_callback_on_a_failed_load_that_claims_no_action_lands_on_signin_failed() {
    let (sources, _) = scripted(vec![Step::Fail], Duration::ZERO);
    let router = router(true, sources);

    let reply = get(&router, "/api/auth/callback?code=c&state=s").await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location(), "/api/?signin=failed");
    assert!(reply.headers.get(header::SET_COOKIE).is_none());
}

// ---------------------------------------------------------------------------
// Login on a failed load (review WR-04)
// ---------------------------------------------------------------------------

/// A failed load is transient, so login lands it on a failure the visitor
/// can retry — sign-in's card for a sign-in, the dashboard for an issue
/// press — never on `?signin=not_open`'s "not yet available", which states a
/// permanent condition. That it earns exactly ONE ERROR line (the alarm's,
/// with no "deployment that cannot complete one" line beside it) is asserted
/// in `tests/portal_load_logs.rs`, a binary of its own.
#[tokio::test]
async fn login_on_a_failed_load_lands_on_a_retryable_failure() {
    for (uri, landing) in [
        ("/api/auth/login", "/api/?signin=failed"),
        ("/api/auth/login?action=issue", "/api/?issue=failed"),
    ] {
        let (sources, _) = scripted(vec![Step::Fail], Duration::ZERO);
        let router = router(true, sources);
        let reply = get(&router, uri).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER, "{uri}");
        assert_eq!(reply.location(), landing, "{uri}");
        assert!(reply.headers.get(header::SET_COOKIE).is_none(), "{uri}");
    }
}
