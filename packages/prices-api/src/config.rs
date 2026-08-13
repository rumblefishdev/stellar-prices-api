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
    /// Public base URL stamped into the OpenAPI `servers` block.
    /// `None` until configured via `API_BASE_URL`.
    pub base_url: Option<String>,
    /// Accepted `X-API-Key` values, parsed from comma-separated `API_KEYS`.
    /// When empty the in-app auth gate is **disarmed** (open) — so local/dev and
    /// the early Phase 2 load test work before keys are provisioned. The
    /// per-key rate limit and monthly quota are enforced at the API Gateway
    /// usage-plan regardless (ADR 0008; sized by task 0157 — not the design
    /// doc's 100 req/s). Mirrors BE's deploy-dark gating.
    pub api_keys: Vec<String>,
    /// Whether the onboarding portal's backend routes are served
    /// (`crate::portal`). **Defaults to `false`** — there is one environment and
    /// it is production, so an unfinished portal slice is publicly reachable the
    /// moment it deploys unless something says otherwise. Set `PORTAL_ENABLED=1`
    /// (or `true`) to work on it locally. Flipped in production by task 0194,
    /// after 0189's eligibility gate passes.
    ///
    /// Note the polarity is the opposite of `ch_enabled` above, and that is on
    /// purpose: a missing `CH_ENABLED` should still give the live Lambda its
    /// connection pool, while a missing `PORTAL_ENABLED` must never open a
    /// half-built portal to the internet. Defaults are chosen per flag by what
    /// goes wrong when the variable is forgotten.
    pub portal_enabled: bool,
}

impl AppConfig {
    /// Read configuration from the environment, applying defaults.
    pub fn from_env() -> Self {
        Self {
            ch_enabled: std::env::var("CH_ENABLED")
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            base_url: std::env::var("API_BASE_URL").ok(),
            api_keys: std::env::var("API_KEYS")
                .map(|raw| {
                    raw.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            portal_enabled: std::env::var("PORTAL_ENABLED")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
        }
    }
}
