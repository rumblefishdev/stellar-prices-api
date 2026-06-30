//! Runtime configuration read from the environment at cold start. Mirrors the
//! shape of BE's `api/src/config.rs`, trimmed to what the Prices API needs
//! today; grows per phase (API keys → Phase 1, cache TTL knobs → Phase 4).

/// Application configuration sourced from environment variables.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Build the mTLS ClickHouse client at cold start when true. Set
    /// `CH_ENABLED=false` (or `0`) for local/plaintext runs and tests that only
    /// exercise CH-free routes (e.g. `/health`). Defaults to true so the live
    /// Lambda always primes its connection pool.
    pub ch_enabled: bool,
    /// Public base URL stamped into the OpenAPI `servers` block (Phase 1).
    /// `None` until configured via `API_BASE_URL`.
    pub base_url: Option<String>,
}

impl AppConfig {
    /// Read configuration from the environment, applying defaults.
    pub fn from_env() -> Self {
        Self {
            ch_enabled: std::env::var("CH_ENABLED")
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            base_url: std::env::var("API_BASE_URL").ok(),
        }
    }
}
