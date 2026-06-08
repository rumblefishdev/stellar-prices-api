//! Retry-with-backoff envelope — duplicated from 0038's crate.
//! Consolidation pending a shared utility crate (see G-note).
//!
//! `[50, 200, 800] ms` cadence — three retries, four wire calls
//! total. Mirrors BE indexer's convention.

use std::time::Duration;

pub const DEFAULT_BACKOFF_MS: [u64; 3] = [50, 200, 800];

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
    async fn permanent_error_short_circuits() {
        let calls = Cell::new(0u32);
        let err = retry_with_backoff(
            &[1, 1, 1],
            |_: &FakeErr| false,
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
}
