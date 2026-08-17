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
    /// Credentials for portal sign-in (task 0186): the Discord client id and
    /// secret, the registered redirect URI, and the key the session and `state`
    /// cookies are signed with.
    ///
    /// **Not read from the environment**, which is the point — ADR 0007 and
    /// Tranche 3 AC 6 forbid a secret value in an env var. [`Self::from_env`]
    /// leaves this `None` and [`Self::load_portal_oauth`] fills it from Secrets
    /// Manager, asynchronously, because the read is an HTTP call.
    ///
    /// `None` means sign-in is not configured on this deployment, which is the
    /// normal state while `portal_enabled` is false.
    pub portal_oauth: Option<crate::portal::auth::secret::OauthSecret>,
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
            portal_oauth: None,
        }
    }

    /// Fill [`Self::portal_oauth`] from Secrets Manager, or from the local file
    /// named by `PORTAL_OAUTH_SECRET_FILE`.
    ///
    /// Called by both entrypoints after [`Self::from_env`]. It is a separate,
    /// async step because it performs I/O, and it is *conditional* on
    /// [`Self::portal_enabled`], which is the load-bearing part:
    ///
    /// Production runs with `PORTAL_ENABLED=false` for the whole of the portal's
    /// build (`compute-stack.ts`, flipped by task 0194). If a cold start read
    /// this secret unconditionally it would fail on a deployment where nobody
    /// has created it yet — and that failure is not confined to the portal.
    /// `main.rs` builds one router for every route group (ADR 0008), so a panic
    /// in init takes out `/v1` as well, to protect four routes that answer an
    /// empty `404` either way.
    ///
    /// With the portal **open**, the opposite stance: a missing or malformed
    /// secret is fatal, because the alternative is a portal that renders a
    /// sign-in button which answers `503`. Fail at deploy, in the Lambda's `Init
    /// Errors` metric, not at a visitor's click.
    pub async fn load_portal_oauth(
        &mut self,
    ) -> Result<(), crate::portal::auth::secret::SecretError> {
        if !self.portal_enabled {
            return Ok(());
        }
        match crate::portal::auth::secret::OauthSecret::load().await? {
            Some(secret) => {
                self.portal_oauth = Some(secret);
                Ok(())
            }
            None => Err(crate::portal::auth::secret::SecretError::NoSource),
        }
    }
}
