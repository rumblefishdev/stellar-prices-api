//! What the SDK's signer writes about the Lambda's credentials — one test, one
//! process (task 0349).
//!
//! In the Lambda the api-handler signs with the execution role's credentials,
//! which carry a session token, and SigV4 signs that token as the
//! `x-amz-security-token` header. `aws_sigv4` logs every request it signs at
//! `trace`: the request (method, URI with its query string, the headers set
//! before signing) with the signing parameters, then the canonical request,
//! signed headers and all. The other tests sign with static credentials and no
//! token, so whether a trace log of the Lambda carries the token itself is
//! answered here: the same SDK client, signing for a token-carrying identity,
//! against a loopback port nobody listens on, so every request is signed and
//! then refused at connect. No AWS call.
//!
//! The capture is taken twice, both at `trace`: through a bare `EnvFilter`,
//! the guard off (what the crates print), and through
//! [`prices_api::telemetry::filter_from`], the guard on (what the deployment
//! prints).
//!
//! **One test, for the reason `portal_keys_logs.rs` is one test:** a
//! subscriber installed with `set_default` is thread-local while `tracing`'s
//! callsite interest cache is global, so a parallel test could leave this one
//! capturing nothing.

use std::sync::{Arc, Mutex};

use aws_sdk_apigateway::Client;
use aws_sdk_apigateway::config::retry::RetryConfig;
use aws_sdk_apigateway::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_apigateway::error::SdkError;
use tracing_subscriber::EnvFilter;

/// The role's session token, as the Lambda's environment hands it over.
const SESSION_TOKEN: &str = "SESSION-TOKEN-CANARY-0349";
/// A key lookup's name: the account's Discord id inside it.
const DISCORD_ID: &str = "308994132968210433";

/// A `MakeWriter` that keeps every byte the subscriber emits.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// A loopback URL nothing listens on: bound, read and let go.
fn closed_port() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

/// The control-plane client `Gateway` builds, with the role's kind of
/// credentials (a session token) instead of the tests' static ones, and no
/// retries, so one call is one signature.
fn client_with_session_token(endpoint: &str) -> Client {
    let config = aws_sdk_apigateway::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("eu-central-1"))
        .credentials_provider(Credentials::new(
            "ASIA0349EXAMPLEKEYID",
            "secret-access-key-0349",
            Some(SESSION_TOKEN.to_string()),
            None,
            "telemetry-sigv4-test",
        ))
        .endpoint_url(endpoint)
        .retry_config(RetryConfig::disabled())
        .build();
    Client::from_conf(config)
}

/// One key lookup by name, signed under `filter`. Returns what was written.
async fn sign_one_lookup_under(filter: EnvFilter) -> String {
    let logs = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_env_filter(filter)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    tracing::trace!(target: "prices_api", "trace probe: the captured writer really sees TRACE");

    let outcome = client_with_session_token(&closed_port())
        .get_api_keys()
        .name_query(format!("discord-{DISCORD_ID}-key"))
        .include_values(false)
        .send()
        .await;
    // A dispatch failure is a request that was signed and then sent: signing
    // comes before the connection is opened.
    assert!(
        matches!(outcome, Err(SdkError::DispatchFailure(_))),
        "the lookup was not signed and sent: {outcome:?}"
    );

    let written = logs.0.lock().unwrap().clone();
    String::from_utf8_lossy(&written).to_string()
}

#[tokio::test]
async fn the_signer_never_logs_the_session_token_and_the_guard_holds_the_rest() {
    // The guard off: every crate at `trace`.
    let unguarded = sign_one_lookup_under(EnvFilter::new("trace")).await;

    // Non-vacuity: the signer's lines are here, they name the account, and
    // the token was signed, so the identity carried one.
    assert!(
        unguarded.contains("aws_sigv4") && unguarded.contains("signing request"),
        "the signer wrote nothing at trace, so this test proves nothing: {unguarded}"
    );
    assert!(
        unguarded.contains(DISCORD_ID),
        "the signer's lines no longer name the account; 0349's finding is out of date: {unguarded}"
    );
    assert!(
        unguarded.contains("x-amz-security-token"),
        "the session token was not signed, so this test proves nothing: {unguarded}"
    );
    // The finding: no crate prints the token's value, the guard off.
    // `aws_sigv4` writes the header's length in its place.
    assert!(
        !unguarded.contains(SESSION_TOKEN),
        "a crate printed the session token with the guard off; the guard is now \
         the only thing holding it, and 0349's finding is out of date: {unguarded}"
    );

    // The guard on: what the deployment writes at `RUST_LOG=trace`.
    let guarded = sign_one_lookup_under(prices_api::telemetry::filter_from("trace")).await;

    assert!(
        guarded.contains("trace probe"),
        "the subscriber captured no TRACE events, so this test proves nothing: {guarded}"
    );
    assert!(
        !guarded.contains("aws_sigv4"),
        "the signer's lines got past the guard: {guarded}"
    );
    assert!(
        !guarded.contains(DISCORD_ID),
        "a Discord id reached the logs: {guarded}"
    );
    assert!(
        !guarded.contains(SESSION_TOKEN),
        "the session token reached the logs: {guarded}"
    );
}
