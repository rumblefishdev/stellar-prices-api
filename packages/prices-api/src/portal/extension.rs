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
//! A fourth attempt never fits: at 2 s per call, three hung calls already
//! exceed `portal::sources::LOAD_BUDGET` (4 s), which is what bounds the whole
//! load. So a fast failure — an immediate non-2xx — gets all three attempts,
//! and a hung one about two. Jitter spreads a herd of environments retrying at
//! once; it comes from `getrandom`, already a dependency.
//!
//! **Only the raw fetch is retried**, and only its transient failure
//! (`MtlsError::Fetch`: unreachable, timed out, a non-2xx, an unreadable
//! envelope). A missing env var is permanent, and parsing and validation (a
//! malformed secret, an empty or non-snowflake parameter) happen outside this
//! wrapper, so they are never retried.
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

/// Worth retrying: the fetch itself failed. Everything else — a missing env
/// var above all — fails the same way every time.
#[cfg(feature = "aws-mtls")]
fn is_transient(error: &prices_clickhouse::mtls::MtlsError) -> bool {
    matches!(error, prices_clickhouse::mtls::MtlsError::Fetch(_))
}

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

    /// Two gaps, and the attempts fit the load budget only when failures are
    /// fast: the arithmetic the module docs state.
    #[test]
    fn three_attempts_with_two_rising_caps() {
        assert_eq!(ATTEMPTS, 3);
        assert!(BACKOFF_CAPS[0] < BACKOFF_CAPS[1]);
    }

    #[cfg(feature = "aws-mtls")]
    #[test]
    fn only_a_fetch_failure_is_transient() {
        use prices_clickhouse::mtls::MtlsError;
        assert!(is_transient(&MtlsError::Fetch("unreachable".into())));
        assert!(!is_transient(&MtlsError::MissingEnv("AWS_SESSION_TOKEN")));
        assert!(!is_transient(&MtlsError::BundleDecode("x".into())));
    }
}
