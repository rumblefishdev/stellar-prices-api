//! Small env-var helpers shared by the worker Lambdas: optional vars with a
//! default. The must-be-set case lives in [`crate::mtls`] (`require_env`); this
//! is the optional-with-default companion, hoisted here so each worker stops
//! hand-rolling `std::env::var(..).unwrap_or_else(..)` and the unset/parse-error
//! fallback behaviour stays consistent across crates.

use std::str::FromStr;

/// The value of env var `key`, or `default` if it is unset. An explicitly empty
/// var is returned as-is (matching `std::env::var`).
pub fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Parse env var `key` into `T`, falling back to `default` if it is unset or
/// fails to parse. For numeric / bool knobs (`BATCH_SIZE`, `MAX_BATCHES`,
/// `ENRICHMENT_ONE_SHOT`, …).
pub fn env_parse_or<T: FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_or_falls_back_on_unset_and_garbage() {
        // A var name unlikely to be set in the test env.
        assert_eq!(env_parse_or::<u32>("PRICES_CH_ENV_TEST_UNSET_XYZ", 7), 7);
        assert_eq!(
            env_or("PRICES_CH_ENV_TEST_UNSET_XYZ", "fallback"),
            "fallback"
        );
    }
}
