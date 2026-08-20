//! What the log filter actually lets through — one test, one process
//! (task 0187).
//!
//! The unit tests beside `src/telemetry.rs` assert on `EnvFilter`'s directive
//! list, which is a statement about what was *parsed*. This one asserts on what
//! was **emitted**: an event on a credential-bearing target, inside the span the
//! AWS SDK wraps every control-plane call in, with the loudest `RUST_LOG` a
//! person could set. That distinction is not academic — the bypass this file
//! exists for was invisible at the directive level, because the hostile
//! directive parses to something that looks like it is about a span and is in
//! fact about every event inside one.
//!
//! **One test, for the reason `portal_keys_logs.rs` is one test:** installing a
//! subscriber with `set_default` is thread-local while `tracing`'s callsite
//! interest cache is global, so a second test running in parallel here could
//! register these callsites under no subscriber and leave this one asserting
//! nothing.

use std::sync::{Arc, Mutex};

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

/// The credential a leaking `GetApiKey` response would carry.
const CANARY: &str = "A-KEY-VALUE-CANARY";

/// Emit, under `rust_log`, the two events that matter: one on an SDK target
/// inside the SDK's own span, and one of ours. Returns what was written.
fn emit_under(rust_log: &str) -> String {
    let logs = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_env_filter(prices_api::telemetry::filter_from(rust_log))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    // `try_op` is the orchestrator's own span name, which is what makes a
    // targetless `[try_op]=trace` a plausible thing for somebody to type.
    let span = tracing::info_span!("try_op");
    let _entered = span.enter();
    tracing::trace!(
        target: "aws_smithy_runtime::client::orchestrator",
        "output_or_error=GetApiKeyOutput {{ value: Some(\"{CANARY}\") }}"
    );
    tracing::trace!(target: "prices_api::portal::keys", "OUR-OWN-TRACE-EVENT");

    let written = logs.0.lock().unwrap().clone();
    String::from_utf8_lossy(&written).to_string()
}

#[test]
fn no_rust_log_value_lets_an_sdk_credential_through() {
    // Non-vacuity first: with the SDK target left alone, the harness observes
    // events at `trace` inside the span. Without this, every assertion below
    // could be passing because nothing was captured at all.
    let ours = emit_under("trace");
    assert!(
        ours.contains("OUR-OWN-TRACE-EVENT"),
        "the harness captured no TRACE events, so this test proves nothing: {ours:?}"
    );

    for hostile in [
        // The obvious one.
        "trace",
        // Aimed at the crate, and at the exact module that prints the value.
        "aws_smithy_runtime=trace",
        "aws_smithy_runtime::client::orchestrator=trace",
        // ONE TRAILING COLON. This is what walked past the blocklist: targets
        // match by `starts_with`, so `aws_smithy_runtime:` matches
        // `aws_smithy_runtime::client::orchestrator`, and directives sort by
        // target length, so 19 characters outranked the 18-character pin. Every
        // other case in this list passed while this one leaked.
        "aws_smithy_runtime:=trace",
        "aws_smithy_runtime::=trace",
        "aws_smithy_runtime:::=trace",
        // Aimed at the SPAN rather than the target. This is the one that got
        // through: a dynamic scope directive outranks a static target pin for
        // events inside a matching span.
        "[try_op]=trace",
        // The same trick wearing a target the guard's crate list would not
        // match, since `EnvFilter` matches targets by prefix.
        "aws_smithy[try_op]=trace",
        "aws[try_op]=trace",
        // A static prefix of a guarded crate. Under the blocklist this was kept
        // and survived only because the pin is longer; under the allowlist it is
        // refused outright, and the test no longer depends on which of the two
        // reasons is doing the work.
        "aws_smithy=trace",
        "aws=trace",
        // Belt and braces.
        "trace,[try_op]=trace,aws_smithy_runtime=trace",
    ] {
        let logged = emit_under(hostile);
        assert!(
            !logged.contains(CANARY),
            "RUST_LOG={hostile:?} put a key value in the logs: {logged:?}"
        );
    }
}
