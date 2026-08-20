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
    /// Which Discord to talk to (task 0186). Production always takes the
    /// defaults; the overrides exist for the local round-trip and for the tests.
    ///
    /// Carried on the config rather than read inside [`crate::app`], and that is
    /// not a style choice. Reading it per-router meant `std::env::var` ran on
    /// every `app()` call while the integration suite was calling
    /// `std::env::set_var` to point routers at a mock — on parallel test
    /// threads. Concurrent `getenv`/`setenv` is undefined behaviour (glibc can
    /// realloc `environ` under a reader), which is why `set_var` is `unsafe` in
    /// edition 2024, and in practice it is an intermittent segfault that takes
    /// the whole test binary down. Threading the value through the config
    /// removes the race by construction instead of serialising around it, and
    /// leaves no `unsafe` in the tests at all.
    pub portal_endpoints: crate::portal::auth::discord::Endpoints,
    /// The API Gateway control-plane client the portal issues keys with
    /// (task 0187), already carrying the `pricing-api-free` usage-plan id.
    ///
    /// `None` means key issuance is not configured on this deployment, which is
    /// the normal state while `portal_enabled` is false — and, like
    /// [`Self::portal_oauth`], it is filled by an async step rather than by
    /// [`Self::from_env`], because building it resolves credentials and reading
    /// the plan id is an HTTP call.
    pub portal_keys: Option<crate::portal::keys::gateway::Gateway>,
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
            portal_endpoints: crate::portal::auth::discord::Endpoints::from_env(),
            portal_keys: None,
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

    /// Fill [`Self::portal_keys`] with a control-plane client for task 0187.
    ///
    /// Conditional on [`Self::portal_enabled`] for exactly the reasons
    /// [`Self::load_portal_oauth`] is, and one more of its own:
    ///
    /// - **A closed portal must not pay for this.** Building the client
    ///   resolves credentials and reads an SSM parameter; doing that at every
    ///   cold start would put two avoidable operations in front of the first
    ///   `/v1` request, on one router that serves every route group (ADR 0008),
    ///   for two routes that answer an empty `404` regardless.
    /// - **A closed portal must not be able to reach the control plane at
    ///   all.** With the portal off there is no client in the process, so no
    ///   code path — not a bug, not a stray handler — can create or delete a
    ///   production API key.
    ///
    /// With the portal **open** a missing plan id is fatal, matching sign-in: a
    /// portal that renders an "issue key" button which answers `503` is worse
    /// than a deploy that fails in the Lambda's `Init Errors` metric.
    ///
    /// # Where the plan id comes from
    ///
    /// `PORTAL_FREE_PLAN_PARAM` carries the **name of an SSM parameter**, not
    /// the id — the parameter `ApiGatewayStack` publishes at
    /// `/prices/{env}/pricing-api-free-plan-id` (task 0157). It cannot be a
    /// cross-stack reference: `ComputeStack` is a dependency of
    /// `ApiGatewayStack`, so importing the plan would close a cycle, which is
    /// the same shape of problem `apiBaseUrl` has. And it must not be
    /// hard-coded, because a usage-plan id is generated by AWS and changes if
    /// the plan is ever replaced.
    pub async fn load_portal_keys(&mut self) -> Result<(), PortalKeysError> {
        if !self.portal_enabled {
            return Ok(());
        }
        let plan_id = free_plan_id().await?;
        self.portal_keys =
            Some(crate::portal::keys::gateway::Gateway::from_ambient_config(plan_id).await);
        Ok(())
    }
}

/// Why key issuance could not be configured at cold start.
#[derive(Debug, thiserror::Error)]
pub enum PortalKeysError {
    #[error(
        "the portal is open but no usage plan is configured; set PORTAL_FREE_PLAN_PARAM to the \
         SSM parameter holding the pricing-api-free plan id (ApiGatewayStack publishes it at \
         /prices/<env>/pricing-api-free-plan-id)"
    )]
    NoSource,
    #[error("reading the usage-plan id from SSM parameter `{name}` failed: {message}")]
    Fetch { name: String, message: String },
    #[error("SSM parameter `{name}` holds an empty usage-plan id")]
    Empty { name: String },
}

/// Resolve the `pricing-api-free` usage-plan id.
async fn free_plan_id() -> Result<String, PortalKeysError> {
    // A direct id, for a local run against a real account. Checked first so a
    // developer with both set gets the local one, exactly as
    // `OauthSecret::load` does.
    //
    // **Compiled out of the Lambda**, for the reason `discord.rs`'s endpoint
    // overrides and `PORTAL_OAUTH_SECRET_FILE` are: `lambda:UpdateFunctionConfiguration`
    // is a permission distinct from `UpdateFunctionCode`, and this variable
    // decides which usage plan self-service keys are attached to. Left readable
    // in the Lambda, one configuration change would silently move every new key
    // onto a plan of somebody else's choosing — a different rate limit, a
    // different quota, or a plan on a stage we do not control.
    #[cfg(not(feature = "lambda"))]
    if let Ok(id) = std::env::var("PORTAL_FREE_PLAN_ID")
        && !id.trim().is_empty()
    {
        return Ok(id.trim().to_string());
    }

    let Ok(name) = std::env::var("PORTAL_FREE_PLAN_PARAM") else {
        return Err(PortalKeysError::NoSource);
    };
    if name.is_empty() {
        return Err(PortalKeysError::NoSource);
    }
    // Trimmed, not merely checked for emptiness. A plan id goes straight into
    // an ARN path segment (`/usageplans/{id}/keys`), so a trailing newline —
    // which is exactly what an operator gets from `echo <id> | aws ssm put-parameter`
    // — would produce a malformed request that reports as a control-plane
    // failure rather than as the typo it is.
    let id = fetch_plan_id(&name).await?.trim().to_string();
    if id.is_empty() {
        return Err(PortalKeysError::Empty { name });
    }
    Ok(id)
}

/// Read the parameter through the Parameters and Secrets extension — the same
/// localhost listener, token and in-process cache the mTLS bundle and the OAuth
/// secret already use, so a warm container never calls Systems Manager on the
/// path that issues a key.
#[cfg(feature = "aws-mtls")]
async fn fetch_plan_id(name: &str) -> Result<String, PortalKeysError> {
    prices_clickhouse::mtls::fetch_parameter_string(name)
        .await
        .map_err(|e| PortalKeysError::Fetch {
            name: name.to_string(),
            message: e.to_string(),
        })
}

#[cfg(not(feature = "aws-mtls"))]
async fn fetch_plan_id(name: &str) -> Result<String, PortalKeysError> {
    Err(PortalKeysError::Fetch {
        name: name.to_string(),
        message: "this build has no Parameters and Secrets extension client (build with \
                  `--features lambda`, or set PORTAL_FREE_PLAN_ID for a local run)"
            .into(),
    })
}
