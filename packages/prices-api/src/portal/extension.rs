//! Retry around every portal read through the Parameters and Secrets extension
//! (task 0311).
//!
//! The extension retries SSM itself — three times, with backoff — and that is
//! slower than our 2 s client timeout, so a throttled read reached us as
//! "extension unreachable" with nothing retried on our side (lore note
//! `R-five-plan-herd-and-capacity-test.md`). One such read used to close the
//! portal. It now costs at most a retry.
//!
//! **Three attempts, two gaps: up to 100 ms, then up to 300 ms, full jitter.**
//! Jitter spreads a herd of environments retrying at once; it comes from
//! `getrandom`, already a dependency.
//!
//! **What a retry buys depends on how the read failed** (review IN-03):
//!
//! - A **fast** failure — a `5xx` or `429` answered at once — gets all three
//!   attempts inside `portal::sources::LOAD_BUDGET` (4 s).
//! - A **hung** read — the measured throttling case, where the extension's own
//!   SSM backoff outlives our 2 s client timeout — gets **one retry at most**,
//!   and that retry is cut short: attempt 1 times out at 2 s, attempt 2 starts
//!   by 2.1 s, and the 4 s budget cancels it after ~1.9–2.0 s, before its own
//!   2 s timeout. It helps only if SSM answers within that window. The
//!   per-issuance eligibility reads run inside their own 2 s
//!   `eligibility::PARAMETER_TIMEOUT`, so a hung one there gets no retry at
//!   all. `a_hung_read_gets_one_retry_inside_the_load_budget` pins this.
//!
//! **Only a transient fetch failure is retried** (review WR-01), classified from
//! the `MtlsError::Fetch` text `prices_clickhouse::mtls` builds — see
//! [`is_transient`] for exactly what is matched. Everything that fails the same
//! way on every attempt returns at once: a `403`/`404`/other `4xx`, a response
//! that parsed but lacks its field, a client that could not be built, a missing
//! env var. Parsing and validation of the value (a malformed secret, an empty
//! or non-snowflake parameter) happen outside this wrapper and are never
//! retried either.
//!
//! `prices_clickhouse::mtls`, and with it the `/v1` mTLS path, is deliberately
//! untouched: this is a wrapper in prices-api around its two fetch functions.

use std::fmt::Display;
use std::future::Future;
use std::time::Duration;

/// How many times one read is tried, the first included.
#[cfg_attr(not(feature = "aws-mtls"), allow(dead_code))]
pub(crate) const ATTEMPTS: usize = 3;

/// The ceiling of the jittered wait before the 2nd and the 3rd attempt.
#[cfg_attr(not(feature = "aws-mtls"), allow(dead_code))]
pub(crate) const BACKOFF_CAPS: [Duration; ATTEMPTS - 1] =
    [Duration::from_millis(100), Duration::from_millis(300)];

/// Run `op` up to [`ATTEMPTS`] times, waiting a jittered backoff after each
/// failure `is_transient` accepts. Returns the first success, the first
/// permanent error at once, or the last error.
///
/// Each retry logs one WARN naming `what`; the caller logs the final
/// failure, if it is one.
#[cfg_attr(not(feature = "aws-mtls"), allow(dead_code))]
pub(crate) async fn with_retry<T, E, F, Fut>(
    what: &str,
    is_transient: impl Fn(&E) -> bool,
    mut op: F,
) -> Result<T, E>
where
    E: Display,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut attempt = 0;
    loop {
        let error = match op().await {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        if attempt + 1 >= ATTEMPTS || !is_transient(&error) {
            return Err(error);
        }
        let wait = full_jitter(BACKOFF_CAPS[attempt]);
        attempt += 1;
        tracing::warn!(
            what,
            attempt,
            wait_ms = wait.as_millis() as u64,
            error = %error,
            "extension read failed; retrying"
        );
        tokio::time::sleep(wait).await;
    }
}

/// A uniformly random wait in `[0, cap]` (AWS's "full jitter"), to the
/// millisecond. Half the cap if the OS cannot supply randomness — a backoff
/// is not a secret, and waiting is better than not retrying.
#[cfg_attr(not(feature = "aws-mtls"), allow(dead_code))]
fn full_jitter(cap: Duration) -> Duration {
    let cap_ms = cap.as_millis() as u64;
    match getrandom::u64() {
        Ok(draw) => Duration::from_millis(draw % (cap_ms + 1)),
        Err(_) => cap / 2,
    }
}

/// Read an SSM parameter through the extension, with the retry.
#[cfg(feature = "aws-mtls")]
pub(crate) async fn parameter_string(
    name: &str,
) -> Result<String, prices_clickhouse::mtls::MtlsError> {
    with_retry(name, is_transient, || {
        prices_clickhouse::mtls::fetch_parameter_string(name)
    })
    .await
}

/// Read a Secrets Manager secret through the extension, with the retry.
#[cfg(feature = "aws-mtls")]
pub(crate) async fn secret_string(
    name: &str,
) -> Result<String, prices_clickhouse::mtls::MtlsError> {
    with_retry(name, is_transient, || {
        prices_clickhouse::mtls::fetch_secret_string(name)
    })
    .await
}

/// Worth retrying: the fetch failed in a way the next attempt may not.
#[cfg(feature = "aws-mtls")]
fn is_transient(error: &prices_clickhouse::mtls::MtlsError) -> bool {
    match error {
        prices_clickhouse::mtls::MtlsError::Fetch(message) => is_transient_fetch(message),
        // A missing env var, above all, fails the same way every time.
        _ => false,
    }
}

/// Classify one `MtlsError::Fetch` message.
///
/// `prices_clickhouse::mtls` is frozen (the `/v1` path), so this reads the
/// text it builds rather than a typed status. The four shapes, pinned against
/// that file by `the_fetch_messages_this_classifies_are_the_ones_mtls_builds`:
///
/// | message | retried |
/// | --- | --- |
/// | `extension at … unreachable …` — connect error or the 2 s timeout | yes |
/// | `extension returned HTTP {status} …` | `5xx` and `429`, and a `400` naming throttling (below) |
/// | `response body parse failed` / `response missing … field` | no |
/// | `reqwest client build failed` | no |
///
/// ⚠️ **A throttled read the extension answers `400` is NOT retried today.**
/// The extension surfaces SSM/Secrets Manager throttling as HTTP `400`
/// (measured 2026-09-24/25), the same status as `ParameterNotFound`, and
/// `mtls` does not put the response body in the message — so the two cannot
/// be told apart, and retrying every `400` would retry a missing parameter on
/// every load. A `400` is retried only when the message itself names
/// throttling (`ThrottlingException`, `Rate exceeded`), which it will if `mtls`
/// ever carries the body. The measured throttling failure is the hung read,
/// which arrives as "unreachable" and IS retried.
///
/// Anything unrecognised is permanent: a retry is a cost paid on every load
/// while a fault lasts, so it has to be earned.
#[cfg_attr(not(feature = "aws-mtls"), allow(dead_code))]
fn is_transient_fetch(message: &str) -> bool {
    if message.starts_with("extension at ") && message.contains(" unreachable ") {
        return true;
    }
    let Some(status) = message
        .strip_prefix("extension returned HTTP ")
        .and_then(|rest| rest.get(..3))
        .and_then(|code| code.parse::<u16>().ok())
    else {
        return false;
    };
    match status {
        429 | 500..=599 => true,
        400 => THROTTLING_MARKERS
            .iter()
            .any(|marker| message.contains(marker)),
        _ => false,
    }
}

/// What AWS calls throttling in an error body: the SSM / Secrets Manager
/// exception name and the API Gateway-style message.
#[cfg_attr(not(feature = "aws-mtls"), allow(dead_code))]
const THROTTLING_MARKERS: [&str; 2] = ["ThrottlingException", "Rate exceeded"];

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Debug, PartialEq)]
    enum Failure {
        Transient(u32),
        Permanent,
    }

    impl Display for Failure {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{self:?}")
        }
    }

    fn transient(error: &Failure) -> bool {
        matches!(error, Failure::Transient(_))
    }

    /// Runs `script` through `with_retry`, returning the result and the call
    /// count.
    async fn run(
        script: Vec<Result<&'static str, Failure>>,
    ) -> (Result<&'static str, Failure>, usize) {
        let calls = Arc::new(AtomicUsize::new(0));
        let script = Arc::new(Mutex::new(script.into_iter().collect::<VecDeque<_>>()));
        let counter = calls.clone();
        let result = with_retry("test-read", transient, || {
            counter.fetch_add(1, Ordering::SeqCst);
            let next = script.lock().unwrap().pop_front().expect("script ran out");
            async move { next }
        })
        .await;
        (result, calls.load(Ordering::SeqCst))
    }

    #[tokio::test(start_paused = true)]
    async fn two_transient_failures_then_a_success_takes_three_calls() {
        let started = tokio::time::Instant::now();
        let (result, calls) = run(vec![
            Err(Failure::Transient(1)),
            Err(Failure::Transient(2)),
            Ok("value"),
        ])
        .await;
        assert_eq!(result, Ok("value"));
        assert_eq!(calls, 3);
        assert!(started.elapsed() <= BACKOFF_CAPS[0] + BACKOFF_CAPS[1]);
    }

    #[tokio::test(start_paused = true)]
    async fn three_transient_failures_return_the_last_and_stop() {
        let (result, calls) = run(vec![
            Err(Failure::Transient(1)),
            Err(Failure::Transient(2)),
            Err(Failure::Transient(3)),
            Ok("never reached"),
        ])
        .await;
        assert_eq!(result, Err(Failure::Transient(3)));
        assert_eq!(calls, 3, "a fourth attempt was made");
    }

    #[tokio::test(start_paused = true)]
    async fn a_permanent_failure_is_not_retried_and_does_not_wait() {
        let started = tokio::time::Instant::now();
        let (result, calls) = run(vec![Err(Failure::Permanent), Ok("never reached")]).await;
        assert_eq!(result, Err(Failure::Permanent));
        assert_eq!(calls, 1);
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[test]
    fn full_jitter_stays_within_its_cap() {
        for cap in BACKOFF_CAPS {
            for _ in 0..1000 {
                assert!(full_jitter(cap) <= cap);
            }
        }
        assert_eq!(full_jitter(Duration::ZERO), Duration::ZERO);
    }

    /// The schedule as it runs (review IN-05): a read that keeps failing fast
    /// is called exactly [`ATTEMPTS`] times, and the gap before the n-th retry
    /// is never longer than its cap — observed on the paused clock, not read
    /// back from the constants.
    #[tokio::test(start_paused = true)]
    async fn a_read_that_keeps_failing_is_called_three_times_within_the_caps() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let result: Result<(), Failure> = with_retry("test-read", transient, || {
            seen.lock().unwrap().push(tokio::time::Instant::now());
            async { Err(Failure::Transient(0)) }
        })
        .await;
        assert!(result.is_err());
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), ATTEMPTS);
        for (gap, cap) in calls.windows(2).map(|w| w[1] - w[0]).zip(BACKOFF_CAPS) {
            assert!(
                gap <= cap,
                "waited {gap:?} before a retry capped at {cap:?}"
            );
        }
    }

    /// What the module doc claims for the measured failure (review IN-03): a
    /// read that hangs until the client's 2 s timeout gets ONE retry inside
    /// `sources::LOAD_BUDGET`, and that retry is cancelled by the budget
    /// before its own timeout. The 2 s is `EXTENSION_REQUEST_TIMEOUT` in
    /// `prices_clickhouse::mtls`, private there and pinned below.
    #[tokio::test(start_paused = true)]
    async fn a_hung_read_gets_one_retry_inside_the_load_budget() {
        const CLIENT_TIMEOUT: Duration = Duration::from_secs(2);
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let budget = crate::portal::sources::LOAD_BUDGET;
        let outcome = tokio::time::timeout(
            budget,
            with_retry("test-read", transient, || {
                counter.fetch_add(1, Ordering::SeqCst);
                async {
                    tokio::time::sleep(CLIENT_TIMEOUT).await;
                    Err::<(), _>(Failure::Transient(0))
                }
            }),
        )
        .await;
        assert!(outcome.is_err(), "the budget, not the retry, ended it");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "a hung read got more than one retry"
        );
        // And a third attempt could not have started even with no wait at all.
        assert!(CLIENT_TIMEOUT * 2 >= budget);
        assert!(
            include_str!("../../../prices-clickhouse/src/mtls.rs")
                .contains("const EXTENSION_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);"),
            "mtls.rs's client timeout moved; redo this arithmetic and the module doc"
        );
    }

    /// Review WR-01: only a failure the next attempt may not repeat.
    #[test]
    fn only_a_transient_fetch_message_is_retried() {
        // Built exactly as `prices_clickhouse::mtls` builds them.
        let unreachable = "extension at http://localhost:2773/systemsmanager/parameters/get \
             unreachable (verify Parameters and Secrets layer ARN is attached and layer is \
             initialised): error sending request";
        let status = |s: &str| format!("extension returned HTTP {s} (parameter=/prices/x)");

        assert!(is_transient_fetch(unreachable));
        assert!(is_transient_fetch(&status("500 Internal Server Error")));
        assert!(is_transient_fetch(&status("503 Service Unavailable")));
        assert!(is_transient_fetch(&status("429 Too Many Requests")));
        assert!(is_transient_fetch(&format!(
            "{}: ThrottlingException: Rate exceeded",
            status("400 Bad Request")
        )));

        assert!(!is_transient_fetch(&status("400 Bad Request")));
        assert!(!is_transient_fetch(&status("403 Forbidden")));
        assert!(!is_transient_fetch(&status("404 Not Found")));
        assert!(!is_transient_fetch(
            "response missing `Parameter.Value` field"
        ));
        assert!(!is_transient_fetch("response missing `SecretString` field"));
        assert!(!is_transient_fetch("response body parse failed: EOF"));
        assert!(!is_transient_fetch("reqwest client build failed: tls"));
        assert!(!is_transient_fetch("something mtls has never said"));
    }

    /// The shapes [`is_transient_fetch`] reads are the ones `mtls.rs` writes:
    /// a reworded message there must fail here, not silently turn a transient
    /// failure permanent (or the reverse).
    #[test]
    fn the_fetch_messages_this_classifies_are_the_ones_mtls_builds() {
        let mtls = include_str!("../../../prices-clickhouse/src/mtls.rs");
        for shape in [
            "\"extension at {EXTENSION_URL} unreachable (verify",
            "\"extension at {EXTENSION_PARAMETER_URL} unreachable (verify",
            "\"extension returned HTTP {status} (secret={secret_name})\"",
            "\"extension returned HTTP {status} (parameter={name})\"",
            "\"reqwest client build failed: {e}\"",
            "\"response body parse failed: {e}\"",
            "\"response missing `SecretString` field\"",
            "\"response missing `Parameter.Value` field\"",
        ] {
            assert!(mtls.contains(shape), "mtls.rs no longer builds {shape}");
        }
    }

    #[cfg(feature = "aws-mtls")]
    #[test]
    fn only_a_transient_fetch_failure_is_transient() {
        use prices_clickhouse::mtls::MtlsError;
        assert!(is_transient(&MtlsError::Fetch(
            "extension returned HTTP 503 Service Unavailable (parameter=x)".into()
        )));
        assert!(!is_transient(&MtlsError::Fetch(
            "extension returned HTTP 404 Not Found (parameter=x)".into()
        )));
        assert!(!is_transient(&MtlsError::MissingEnv("AWS_SESSION_TOKEN")));
        assert!(!is_transient(&MtlsError::BundleDecode("x".into())));
    }
}
