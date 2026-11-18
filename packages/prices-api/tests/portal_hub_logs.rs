//! **What the hub contract's logs never say** — one assertion, one test
//! binary (task 0348).
//!
//! A binary of its own for the reason `tests/portal_keys_logs.rs` gives:
//! `tracing`'s per-callsite interest is cached globally by whichever thread
//! reaches a callsite first, so a capture beside parallel tests can see
//! nothing and pass for it.
//!
//! The contract's log rule (the plan's Global Constraints): a call is named
//! by its `jti` and `act`, with its outcome and `key_id`; never the token,
//! the key's value, the Discord id or the username. Every route runs here,
//! on its happy path and its refusals, with logging all the way up through
//! the deployment's guard; then the captured text is searched for each.
//!
//! Non-vacuity: a `trace!` from this test must appear, and so must the
//! contract's own lines, each with its `jti`.

#[path = "portal_keys/harness.rs"]
mod harness;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use harness::*;
use prices_api::portal::auth::crypto;
use prices_api::portal::hub::secret::HubSecret;
use prices_api::portal::hub::token::{AUDIENCE, CONTEXT, ISSUER};
use prices_api::portal::hub::{ACCOUNT_PATH, ISSUE_PATH, REVEAL_PATH, REVOKE_PATH};
use prices_api::portal::sources::{HubSource, Loaded, PortalSources};
use prices_api::{AppConfig, AppState, app_with_portal};
use serde_json::json;

const SECRET: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA=";
/// A username no log line could contain by accident.
const USERNAME: &str = "nelly-zq7-username";

/// A `MakeWriter` that keeps every byte the subscriber emits.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

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

/// A token for `act`, with `jti`, signed as the hub signs one.
fn token(act: &str, jti: &str) -> String {
    let now = now_secs();
    let claims = json!({
        "iss": ISSUER,
        "aud": AUDIENCE,
        "sub": format!("discord:{USER_ID}"),
        "name": USERNAME,
        "iat": now,
        "exp": now + 60,
        "jti": jti,
        "act": act,
    });
    format!(
        "v1.{}",
        crypto::sign(
            &STANDARD.decode(SECRET).unwrap(),
            CONTEXT,
            &serde_json::to_vec(&claims).unwrap()
        )
    )
}

async fn hub_call(app: &Router, method: &str, path: &str, token: &str) -> Reply {
    let authorization = format!("Bearer {token}");
    call_path_with(
        app.clone(),
        method,
        path,
        None,
        &[("authorization", authorization.as_str())],
    )
    .await
}

#[tokio::test]
async fn no_token_key_discord_id_or_username_reaches_the_logs() {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        // `trace`, through the guard, as `portal_keys_logs.rs` explains.
        .with_env_filter(prices_api::telemetry::filter_from("trace"))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    tracing::trace!("trace probe: the captured writer really sees TRACE");

    let mock = MockGateway::start().await;
    let config = AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![],
        portal_enabled: true,
        portal_oauth: None,
        portal_endpoints: Default::default(),
        portal_keys: None,
        portal_eligibility: None,
        portal_rate_limit: Some(5),
        portal_web_origin: None,
    };
    let secret = HubSecret::parse(&json!({ "current": SECRET }).to_string()).unwrap();
    let app = app_with_portal(
        &config,
        AppState::without_ch(),
        PortalSources::ready(Loaded {
            oauth: Some(Arc::new(oauth_secret())),
            gateway: Some(Arc::new(test_gateway(&mock.base))),
            settings: Some(Arc::new(eligibility(GUILD_ID, "5"))),
        })
        .with_hub(HubSource::ready(Some(secret))),
    );

    // Every route, in the order a visitor would meet them, each with a jti
    // of its own; then the refusals and a failure.
    let mut tokens = Vec::new();
    let mut call = async |method: &str, path: &str, act: &str, jti: &str| {
        let token = token(act, jti);
        let reply = hub_call(&app, method, path, &token).await;
        tokens.push(token);
        reply
    };
    let issued = call("POST", ISSUE_PATH, "issue", "jti-issue-0001").await;
    assert_eq!(issued.json(), json!({ "outcome": "issued" }));
    let account = call("GET", ACCOUNT_PATH, "read", "jti-account-0002").await;
    assert_eq!(account.json()["key"]["state"], "active");
    let revealed = call("POST", REVEAL_PATH, "reveal", "jti-reveal-0003").await;
    assert_eq!(revealed.status, StatusCode::OK);
    let has_key = call("POST", ISSUE_PATH, "issue", "jti-issue-0004").await;
    assert_eq!(has_key.json(), json!({ "outcome": "has_key" }));
    // A duplicate under the account's name that refuses to be disabled, made
    // after the key's creation so the issued key stays the current one: the
    // revoke is partial, and its line must say so.
    mock.with(|s| {
        let stubborn = s.seed(&format!("discord-{USER_ID}-key"), now_secs() + 1_000);
        s.fail_disable_of = vec![stubborn];
    });
    let revoked = call("POST", REVOKE_PATH, "revoke", "jti-revoke-0005").await;
    assert_eq!(revoked.status, StatusCode::OK);
    assert_eq!(revoked.json()["partial"], true);
    // The duplicate still answers, so the account still has a key.
    let still_has = call("POST", ISSUE_PATH, "issue", "jti-issue-0006").await;
    assert_eq!(still_has.json()["outcome"], "has_key");
    let wrong_act = call("POST", REVOKE_PATH, "read", "jti-wrong-act-0007").await;
    assert_eq!(wrong_act.status, StatusCode::FORBIDDEN);
    mock.with(|s| s.fail_list = true);
    let failed = call("GET", ACCOUNT_PATH, "read", "jti-failed-0008").await;
    assert_eq!(failed.status, StatusCode::BAD_GATEWAY);
    let forged = format!("{}x", tokens[0]);
    let refused = hub_call(&app, "GET", ACCOUNT_PATH, &forged).await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    tokens.push(forged);

    let text = String::from_utf8_lossy(&logs.0.lock().unwrap()).to_string();

    // Non-vacuity first.
    assert!(
        text.contains("trace probe"),
        "the subscriber captured no TRACE events, so this test proves nothing: {text:?}"
    );
    for (jti, line) in [
        ("jti-issue-0001", "hub issued an API key"),
        ("jti-account-0002", "hub answered an account"),
        ("jti-reveal-0003", "hub revealed an API key"),
        ("jti-revoke-0005", "hub revoked an API key"),
        ("jti-wrong-act-0007", "hub call refused"),
        ("jti-failed-0008", "hub account lookup failed"),
    ] {
        assert!(
            text.lines().any(|l| l.contains(jti) && l.contains(line)),
            "no `{line}` line with {jti}: {text}"
        );
    }
    assert!(
        !text.contains("portal key reveal failed"),
        "the account's failure was logged as a reveal: {text}"
    );
    // The log rule's `key_id`, on every line that names a key the call
    // touched: the issue (created and adopted), the reveal and the revoke.
    let key_id = revealed.json()["key_id"].as_str().unwrap().to_string();
    let with_id = format!("key_id={key_id}");
    for (jti, line) in [
        ("jti-issue-0001", "hub issued an API key"),
        ("jti-account-0002", "hub answered an account"),
        ("jti-reveal-0003", "hub revealed an API key"),
        ("jti-issue-0004", "hub issued an API key"),
        ("jti-revoke-0005", "hub revoked an API key"),
    ] {
        assert!(
            text.lines()
                .any(|l| l.contains(jti) && l.contains(line) && l.contains(&with_id)),
            "no `{line}` line with {jti} and {with_id}: {text}"
        );
    }

    // The revoke line says it was partial, not only that it revoked.
    assert!(
        text.lines().any(|l| l.contains("jti-revoke-0005")
            && l.contains("hub revoked an API key")
            && l.contains("partial=true")),
        "the partial revoke's line does not say so: {text}"
    );

    let value = revealed.json()["value"].as_str().unwrap().to_string();
    assert!(!value.is_empty());
    for (what, secret) in [("a key value", value.as_str()), ("a key value", "CANARY")] {
        assert!(!text.contains(secret), "{what} reached the logs: {text}");
    }
    for token in &tokens {
        let (payload, mac) = token.strip_prefix("v1.").unwrap().split_once('.').unwrap();
        for part in [token.as_str(), payload, mac] {
            assert!(!text.contains(part), "a token reached the logs: {text}");
        }
    }
    // The account's own lines: everything this crate logs. Lines from the
    // SDK and the HTTP stack beneath it are the guard's business
    // (`telemetry`), not this contract's.
    let ours: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("prices_api"))
        .collect();
    assert!(!ours.is_empty());
    for line in ours {
        assert!(
            !line.contains(USER_ID),
            "a Discord id reached the logs: {line}"
        );
        assert!(
            !line.contains(USERNAME),
            "a username reached the logs: {line}"
        );
    }
    assert!(
        !text.contains(USERNAME),
        "a username reached the logs: {text}"
    );
}
