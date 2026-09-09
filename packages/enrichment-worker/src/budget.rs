//! Lambda invocation-deadline arithmetic, shared by every entrypoint.
//!
//! Lives in the library rather than in a `main.rs` because two callers
//! computing the same deadline slightly differently is how one of them ends up
//! without a margin — which is precisely what happened when the coarse sweep was
//! split into its own binary (task 0218) and re-inlined a 30 s margin against
//! the enrichment worker's 60 s.

/// Safety margin held back from the Lambda deadline.
///
/// A function timeout is an **invocation error, not a Rust `Err`**, so it
/// escapes every best-effort handler and fails the whole invocation. Stopping
/// this far short leaves room to finish the in-flight statement, publish
/// metrics and return cleanly.
pub const MARGIN_MS: u64 = 60_000;

/// Milliseconds of useful work left in this invocation: the Lambda deadline
/// (epoch ms, from `event.context.deadline`) minus [`MARGIN_MS`].
///
/// Saturating throughout, so a deadline already past yields `0` — meaning "do
/// nothing this run" rather than a wrapped, enormous budget.
pub fn remaining_budget_ms(deadline_ms: u64) -> u64 {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    deadline_ms.saturating_sub(now_ms).saturating_sub(MARGIN_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deadline already in the past must yield zero, not a wrapped huge
    /// number — the difference between "skip this run" and "run unbounded".
    #[test]
    fn a_past_deadline_saturates_to_zero() {
        assert_eq!(remaining_budget_ms(0), 0);
    }

    /// A deadline inside the margin also yields zero: there is time on the
    /// clock, but not enough to finish and return cleanly.
    #[test]
    fn a_deadline_inside_the_margin_yields_zero() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap();
        assert_eq!(remaining_budget_ms(now_ms + MARGIN_MS / 2), 0);
    }

    #[test]
    fn budget_is_the_deadline_minus_the_margin() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap();
        let got = remaining_budget_ms(now_ms + MARGIN_MS + 120_000);
        // Allow a little slack for the clock read inside the call.
        assert!(
            (119_000..=120_000).contains(&got),
            "expected ~120000, got {got}"
        );
    }
}
