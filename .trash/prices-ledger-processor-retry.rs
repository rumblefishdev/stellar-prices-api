//! Retry-with-backoff envelope mirroring BE's indexer
//! (`crates/indexer/src/handler/mod.rs:113`).
//!
//! `[50, 200, 800] ms` cadence — three retries, four wire calls total.
//! Only the caller knows which errors are transient; pass a classifier.

use std::time::Duration;

pub const DEFAULT_BACKOFF_MS: [u64; 3] = [50, 200, 800];

/// Returns `Ok(attempts)` where `attempts` is the retry count (0 = first
/// attempt succeeded). Errors classified as non-transient short-circuit.
pub async fn retry_with_backoff<F, Fut, T, E, P>(
    backoff_ms: &[u64],
    is_transient: P,
    mut attempt: F,
) -> Result<(T, u32), E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    P: Fn(&E) -> bool,
{
    let mut tries: u32 = 0;
    loop {
        match attempt().await {
            Ok(v) => return Ok((v, tries)),
            Err(e) => {
                if !is_transient(&e) || tries as usize >= backoff_ms.len() {
                    return Err(e);
                }
                tokio::time::sleep(Duration::from_millis(backoff_ms[tries as usize])).await;
                tries += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Debug, PartialEq)]
    enum FakeErr {
        Transient,
        Permanent,
    }

    #[tokio::test]
    async fn succeeds_first_try() {
        let (v, tries) = retry_with_backoff(
            &[1, 1, 1],
            |_: &FakeErr| true,
            || async { Ok::<u32, FakeErr>(42) },
        )
        .await
        .unwrap();
        assert_eq!(v, 42);
        assert_eq!(tries, 0);
    }

    #[tokio::test]
    async fn retries_transient_then_succeeds() {
        let calls = Cell::new(0u32);
        let (v, tries) = retry_with_backoff(
            &[1, 1, 1],
            |e: &FakeErr| matches!(e, FakeErr::Transient),
            || async {
                let n = calls.get();
                calls.set(n + 1);
                if n < 2 {
                    Err(FakeErr::Transient)
                } else {
                    Ok::<u32, FakeErr>(7)
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(v, 7);
        assert_eq!(tries, 2);
    }

    #[tokio::test]
    async fn permanent_error_short_circuits() {
        let calls = Cell::new(0u32);
        let err = retry_with_backoff(
            &[1, 1, 1],
            |e: &FakeErr| matches!(e, FakeErr::Transient),
            || async {
                calls.set(calls.get() + 1);
                Err::<u32, FakeErr>(FakeErr::Permanent)
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err, FakeErr::Permanent);
        assert_eq!(calls.get(), 1);
    }

    #[tokio::test]
    async fn exhausts_backoff_then_fails() {
        let calls = Cell::new(0u32);
        let err = retry_with_backoff(
            &[1, 1, 1],
            |e: &FakeErr| matches!(e, FakeErr::Transient),
            || async {
                calls.set(calls.get() + 1);
                Err::<u32, FakeErr>(FakeErr::Transient)
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err, FakeErr::Transient);
        assert_eq!(calls.get(), 4); // 1 initial + 3 retries
    }
}
