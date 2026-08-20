//! **No key value reaches the logs** — one assertion, one test binary
//! (task 0187).
//!
//! # Why this is not in `portal_keys.rs`
//!
//! It was, and there it passed while the thing it asserts was false.
//!
//! `tracing`'s per-callsite `Interest` is cached **globally**, once, by whichever
//! thread reaches a callsite first; `tracing::subscriber::set_default` is
//! **thread-local**. In a test binary running its tests in parallel, some other
//! test's thread touches the AWS SDK's `trace!` callsites while no subscriber is
//! installed on it, the callsites are cached as `Interest::never`, and the one
//! test that did install a `TRACE` subscriber then captures nothing and asserts
//! nothing. Run alone (`--test-threads=1`, or a filter that selects it) the same
//! test failed, printing a live key value out of the SDK's own log line.
//!
//! A binary of its own is a process of its own: this is the only test in it, so
//! there is no other thread to lose the race to. That is the whole reason for
//! the file, and for `portal_keys/harness.rs` being a shared module.
//!
//! # What it proves
//!
//! The filter in [`prices_api::telemetry`] is what stops the leak — the SDK
//! prints a `GetApiKey` response, key value included, at `trace`, and
//! `aws-sdk-apigateway` marks nothing `@sensitive`, so smithy's own redaction
//! never fires. So the subscriber here is built the way the binaries build
//! theirs: everything the operator could ask for (`trace`, globally), through
//! the same guard. If the guard is weakened, this test prints the key it was
//! meant to protect.
//!
//! Two assertions make it non-vacuous, both of which would fail under exactly
//! the callsite-cache poisoning described above:
//!
//! - a `trace!` from this test itself must appear, proving `TRACE` really flows
//!   into the captured writer;
//! - the handler's own `info!` must appear, proving the routes ran under this
//!   subscriber rather than beside it.

#[path = "portal_keys/harness.rs"]
mod harness;
#[path = "common/mock_discord.rs"]
mod mock_discord;

use std::sync::{Arc, Mutex};

use harness::*;
use mock_discord::{GRANTED_SCOPE, MockDiscord};

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

/// No key value reaches the logs, on any path, with logging turned all the way
/// up.
///
/// The reconciler's delete path runs too (two seeded duplicates), because that
/// is the path that legitimately *does* name key ids — the assertion has to be
/// about the credential, not about log lines being sparse.
#[tokio::test]
async fn no_key_value_ever_reaches_the_logs() {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        // `trace`, through the guard — the loudest thing an operator can set,
        // filtered exactly as `main.rs` and `serve.rs` filter it. Not
        // `with_max_level(TRACE)`: that would be a subscriber no deployment
        // has, and the leak this test exists for lives in what the guard drops.
        .with_env_filter(prices_api::telemetry::filter_from("trace"))
        .finish();
    // A guard rather than `with_default`'s closure: the default is thread-local
    // and `#[tokio::test]` drives this future on this same thread, so the
    // subscriber is in force across every `await` below without a nested runtime.
    let _guard = tracing::subscriber::set_default(subscriber);

    tracing::trace!("trace probe: the captured writer really sees TRACE");

    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    mock.with(|s| {
        s.seed(&name, 1_000);
        s.seed(&name, 2_000);
    });

    // The full issue round-trip (task 0189): the create and delete paths run
    // through the callback now, and the delete path is the one that
    // legitimately names key IDS in its logs — the assertion has to be about
    // the credential, not about log lines being sparse. Then the reveal, which
    // is the path the VALUE flows through.
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let app = build_app_with(
        true,
        Some(prices_api::portal::keys::gateway::Gateway::against(
            &mock.base,
            PLAN_ID.to_string(),
        )),
        prices_api::portal::auth::discord::Endpoints {
            api_base: discord.base.clone(),
            ..Default::default()
        },
        Some(eligibility(GUILD_ID, "5")),
    );
    let issued = issue_round_trip(&app).await;
    assert_eq!(issued.location(), "/api-tokens/?issue=ok");
    let body = reveal(&mock, USER_ID).await.json();

    let text = String::from_utf8_lossy(&logs.0.lock().unwrap()).to_string();

    // Non-vacuity first: a test that captured nothing would pass every
    // assertion below for the worst possible reason.
    assert!(
        text.contains("trace probe"),
        "the subscriber captured no TRACE events, so this test proves nothing: {text:?}"
    );
    assert!(
        text.contains("portal issued an API key"),
        "the issue flow did not run under this subscriber, so this test proves nothing: {text:?}"
    );
    assert!(
        text.contains("portal revealed an API key"),
        "the reveal did not run under this subscriber, so this test proves nothing: {text:?}"
    );

    let value = body["value"].as_str().unwrap();
    assert!(!value.is_empty());
    assert!(
        !text.contains(value),
        "a key value reached the logs: {text:?}"
    );
    assert!(
        !text.contains("CANARY"),
        "a key value reached the logs: {text:?}"
    );
}
