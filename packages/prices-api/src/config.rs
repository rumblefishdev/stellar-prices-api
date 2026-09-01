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
    /// The free plan's per-key rate limit, requests per second, for the portal
    /// dashboard to state (task 0188).
    ///
    /// Read from `PORTAL_RATE_LIMIT`, which `compute-stack.ts` sets from
    /// `pricingApiFreePlanRateLimit` — the same config value
    /// `api-gateway-stack.ts` feeds to `addUsagePlan`. It travels this way
    /// rather than being read back from `GetUsagePlan` because that would cost
    /// the portal a control-plane grant task 0188 deliberately does not take,
    /// and rather than being a literal in the frontend because that is the one
    /// number on the panel that could then drift from what the gateway
    /// enforces: raise the limit in `infra/envs/production.json`, deploy, and a
    /// dashboard whose stated theme is rendering honestly would keep stating
    /// the old figure.
    ///
    /// `None` — unset, or set to something that is not a positive integer —
    /// means this deployment cannot say what the limit is, and the page omits
    /// the line rather than guessing. A default of 1 here would be the same
    /// silent staleness one layer down.
    pub portal_rate_limit: Option<u32>,
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
    /// Where the eligibility gate's two knobs come from (task 0189): the
    /// Stellar guild id and the minimum account age. `None` means the gate is
    /// not configured, which is the normal state while `portal_enabled` is
    /// false; filled by [`Self::load_portal_eligibility`], which also probes
    /// both values once so a mis-seeded parameter closes the portal at cold
    /// start ([`Self::load_portal_or_close`]) rather than refusing at a
    /// visitor's click.
    pub portal_eligibility: Option<crate::portal::eligibility::EligibilitySettings>,
    /// The origin the portal's bundle is served from, when that is not this
    /// backend's own host (task 0194): `https://sorobanscan.rumblefish.dev`.
    ///
    /// Read from `PORTAL_WEB_ORIGIN`, which `compute-stack.ts` sets from
    /// `portalWebOrigin` — the same value `api-gateway-stack.ts` names in the
    /// preflight, so the two halves of the CORS answer cannot disagree. Two
    /// things hang off it, and only these two: the one origin the portal
    /// routes' `Access-Control-Allow-Origin` names (`portal::cors_layer`),
    /// and the host a sign-in round-trip lands on after the callback
    /// (`auth::AuthState::with_web_origin`) — the callback runs here, where
    /// the session cookie is set, and the page lives there.
    ///
    /// `None` — unset or blank — is the same-origin deployment: every landing
    /// is the bare `PORTAL_HOME` path and no CORS header is ever emitted,
    /// which is what local `serve` and the tests want. Normalised by
    /// [`web_origin_from`]; the shape is checked once, at synth, by
    /// `validateConfig`.
    pub portal_web_origin: Option<String>,
}

/// Normalise `PORTAL_WEB_ORIGIN`: trimmed, no trailing slash, blank is `None`.
///
/// An origin is compared byte-for-byte by the browser, so a trailing `/` —
/// the most natural typo for a value that looks like a URL — would make every
/// CORS answer a mismatch with nothing in any log to say why. Stripped here
/// rather than refused: the value is a deploy-time literal, and a config error
/// this small should not be a cold-start failure on the function that also
/// serves `/v1`. A value with a path, a query, or no scheme IS refused, loudly,
/// because there is no single right reading of it.
pub fn web_origin_from(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let authority = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"));
    match authority {
        Some(rest) if !rest.is_empty() && !rest.contains(['/', '?', '#']) => {
            Some(trimmed.to_string())
        }
        _ => {
            tracing::error!(
                value = trimmed,
                "PORTAL_WEB_ORIGIN is not a bare origin (scheme://host[:port]); ignoring it"
            );
            None
        }
    }
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
            portal_rate_limit: std::env::var("PORTAL_RATE_LIMIT")
                .ok()
                .and_then(|raw| raw.trim().parse::<u32>().ok())
                .filter(|limit| *limit > 0),
            portal_oauth: None,
            portal_endpoints: crate::portal::auth::discord::Endpoints::from_env(),
            portal_keys: None,
            portal_eligibility: None,
            portal_web_origin: web_origin_from(std::env::var("PORTAL_WEB_ORIGIN").ok().as_deref()),
        }
    }

    /// Fill [`Self::portal_oauth`] from Secrets Manager, or from the local file
    /// named by `PORTAL_OAUTH_SECRET_FILE`.
    ///
    /// Called by both entrypoints after [`Self::from_env`]. It is a separate,
    /// async step because it performs I/O, and it is *conditional* on
    /// [`Self::portal_enabled`], which is the load-bearing part:
    ///
    /// Production ran with `PORTAL_ENABLED=false` for the whole of the portal's
    /// build, until task 0194 flipped it in `compute-stack.ts` — so this read
    /// now happens on every production cold start. The conditionality still
    /// matters for tests and for any environment where the flag is off: if a
    /// cold start read this secret unconditionally it would fail on a deployment
    /// where nobody has created it yet — and that failure is not confined to the portal.
    /// `main.rs` builds one router for every route group (ADR 0008), so a panic
    /// in init takes out `/v1` as well, to protect four routes that answer an
    /// empty `404` either way.
    ///
    /// With the portal **open**, a missing or malformed secret is an error —
    /// and what the Lambda does with it is the decision recorded on
    /// [`Self::load_portal_or_close`]: close the portal in that process
    /// rather than panic, because a panic here is an init failure on the
    /// function that also serves `/v1`. The thing this guards against — a
    /// sign-in button that answers `503` — does not happen either way: with
    /// the portal closed the gate answers before any handler does.
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
    /// With the portal **open** a missing plan id is an error, matching
    /// sign-in — see [`Self::load_portal_or_close`] for what the Lambda does
    /// with it. A portal that renders an "issue key" button which answers
    /// `503` is the thing to avoid, and a closed portal avoids it as surely as
    /// a failed init does, without taking `/v1` down.
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

    /// Fill [`Self::portal_eligibility`] with the sources of the eligibility
    /// gate's two knobs (task 0189).
    ///
    /// Conditional on [`Self::portal_enabled`] for exactly the reasons the two
    /// loaders above are. With the portal **open**, a missing source is an
    /// error — a portal whose "get my key" round-trip can only ever answer
    /// "could not verify" is worse than one that is closed — and so is an
    /// unreadable or malformed *value*: both parameters are **probed once
    /// here**, so `discord-guild-id` seeded with a name instead of a
    /// snowflake, or `min-account-age-minutes` holding "five", is a cold-start
    /// error with the parameter named, not a per-visitor refusal. What the
    /// Lambda does with the error is [`Self::load_portal_or_close`]'s call.
    ///
    /// What is stored is the **source**, not the probed value: every issuance
    /// resolves it again, which is what makes an operator's `put-parameter`
    /// take effect without a redeploy (bounded only by the Parameters and
    /// Secrets extension's ~5 min cache).
    ///
    /// # Where the values come from
    ///
    /// `PORTAL_GUILD_ID_PARAM` and `PORTAL_MIN_ACCOUNT_AGE_PARAM` carry the
    /// **names of SSM parameters** (`/prices/{env}/discord-guild-id`,
    /// `/prices/{env}/min-account-age-minutes`), seeded by the operator at
    /// deploy prep — never created by CDK, because a CloudFormation-managed
    /// parameter is restored to the committed value by the next `cdk deploy`,
    /// which would silently un-flip production back to the test guild after
    /// [0179]. The direct-value overrides are local-only seams, compiled out
    /// of the Lambda like `PORTAL_FREE_PLAN_ID`.
    pub async fn load_portal_eligibility(&mut self) -> Result<(), PortalEligibilityError> {
        if !self.portal_enabled {
            return Ok(());
        }
        let settings = eligibility_settings()?;
        // Probe both values now. The per-action resolve keeps them tunable;
        // this makes a bad seed loud at deploy time.
        settings
            .guild_id()
            .await
            .map_err(PortalEligibilityError::Probe)?;
        settings
            .min_account_age_minutes()
            .await
            .map_err(PortalEligibilityError::Probe)?;
        self.portal_eligibility = Some(settings);
        Ok(())
    }

    /// The three loaders above, in order, stopping at the first error.
    async fn load_portal(&mut self) -> Result<(), PortalLoadError> {
        self.load_portal_oauth().await?;
        self.load_portal_keys().await?;
        self.load_portal_eligibility().await?;
        Ok(())
    }

    /// Load every portal source, or close the portal in this process.
    ///
    /// The three loaders each return their error; this is where the Lambda
    /// decides what an error *means*, and the decision is **closed, not
    /// crashed** (task 0194, PR review finding 1). `main.rs` used to
    /// `expect()` each loader, on the argument that a portal source missing at
    /// deploy should fail loudly in `Init Errors` rather than as a `503` under
    /// a sign-in button. Three things were wrong with that:
    ///
    /// - **The loud failure lands on `/v1`.** One router serves every route
    ///   group (ADR 0008), so an init panic is not "the portal fails to
    ///   deploy" — `cdk deploy` succeeds regardless — it is the next `/v1`
    ///   caller receiving a `502`, over a secret and three SSM parameters the
    ///   data API never uses.
    /// - **It was not only a deploy hazard.** The reads go through the
    ///   Parameters and Secrets extension with a 2 s timeout and no retry
    ///   (`prices_clickhouse::mtls`), and Parameter Store's default throughput
    ///   is 40 TPS for the whole account. A burst of cold starts — the ramp of
    ///   a load test — is three SSM reads per environment against that budget,
    ///   and a throttled one was a `502` on the data API.
    /// - **Nobody is paged by `Init Errors`.** The api-handler has no alarm on
    ///   `Errors` at all; the "loud" failure was loud only to whoever probed.
    ///
    /// So on any error the portal is closed *in this execution environment* —
    /// the flag cleared and all three sources dropped, which restores every
    /// property of a closed portal (the gate answers before any handler, and
    /// there is no control-plane client in the process) — and the error is
    /// returned for the caller to log. `/config` then reports
    /// `enabled: false`, which is the probe the deploy runbook already makes
    /// after every deploy, so a misconfigured deploy is caught by the same step
    /// it always was; `/v1` never notices.
    ///
    /// The cost, stated: an environment that failed a *transient* read stays
    /// closed for its lifetime, where a panic would have discarded it and
    /// retried on the next cold start. That trades a `502` on the data API for
    /// a portal that, in one environment, says it is not open. The alarm on
    /// the log line `main.rs` writes is the follow-up recorded on task 0194.
    ///
    /// `serve.rs` keeps its three `expect()`s on purpose: a developer who asked
    /// for the portal and did not get it wants to know now, and no partner is
    /// behind that process.
    pub async fn load_portal_or_close(&mut self) -> Result<(), PortalLoadError> {
        let loaded = self.load_portal().await;
        if loaded.is_err() {
            self.close_portal();
        }
        loaded
    }

    /// Close the portal in this process: the flag AND the three sources, so a
    /// half-loaded configuration (secret read, plan id not) leaves nothing
    /// behind that a closed portal would not have — in particular, no
    /// control-plane client.
    fn close_portal(&mut self) {
        self.portal_enabled = false;
        self.portal_oauth = None;
        self.portal_keys = None;
        self.portal_eligibility = None;
    }
}

/// Which portal source failed to load at cold start — the value
/// [`AppConfig::load_portal_or_close`] hands back, with the variable that
/// names the source, so the log line points at the runbook step.
#[derive(Debug, thiserror::Error)]
pub enum PortalLoadError {
    #[error("portal sign-in (PORTAL_OAUTH_SECRET_NAME): {0}")]
    Oauth(#[from] crate::portal::auth::secret::SecretError),
    #[error("portal key issuance (PORTAL_FREE_PLAN_PARAM): {0}")]
    Keys(#[from] PortalKeysError),
    #[error("portal eligibility gate (PORTAL_GUILD_ID_PARAM, PORTAL_MIN_ACCOUNT_AGE_PARAM): {0}")]
    Eligibility(#[from] PortalEligibilityError),
}

/// Why the eligibility gate could not be configured at cold start.
#[derive(Debug, thiserror::Error)]
pub enum PortalEligibilityError {
    #[error(
        "the portal is open but the eligibility gate has no sources; set PORTAL_GUILD_ID_PARAM \
         and PORTAL_MIN_ACCOUNT_AGE_PARAM to the SSM parameters the operator seeds at \
         /prices/<env>/discord-guild-id and /prices/<env>/min-account-age-minutes (see the \
         deploy-prep runbook)"
    )]
    NoSource,
    #[error("probing the eligibility parameters failed: {0}")]
    Probe(crate::portal::eligibility::EligibilityError),
}

/// Build the eligibility sources from the environment.
fn eligibility_settings()
-> Result<crate::portal::eligibility::EligibilitySettings, PortalEligibilityError> {
    use crate::portal::eligibility::{EligibilitySettings, ParamSource};

    let source = |direct_var: &str, param_var: &str| -> Option<ParamSource> {
        // A direct value, for a local run. Checked first, and **compiled out
        // of the Lambda**, exactly as `PORTAL_FREE_PLAN_ID` is and for the
        // same reason: `lambda:UpdateFunctionConfiguration` is a permission
        // distinct from `UpdateFunctionCode`, and these two values decide
        // which guild gates issuance and how old an account must be — left
        // readable in the Lambda, one configuration change would silently
        // point the gate at a guild of somebody else's choosing.
        #[cfg(not(feature = "lambda"))]
        if let Ok(value) = std::env::var(direct_var)
            && !value.trim().is_empty()
        {
            return Some(ParamSource::Direct(value));
        }
        #[cfg(feature = "lambda")]
        let _ = direct_var;

        let name = std::env::var(param_var).ok()?;
        if name.trim().is_empty() {
            return None;
        }
        Some(ParamSource::Ssm(name.trim().to_string()))
    };

    let guild_id = source("PORTAL_GUILD_ID", "PORTAL_GUILD_ID_PARAM")
        .ok_or(PortalEligibilityError::NoSource)?;
    let min_account_age = source(
        "PORTAL_MIN_ACCOUNT_AGE_MINUTES",
        "PORTAL_MIN_ACCOUNT_AGE_PARAM",
    )
    .ok_or(PortalEligibilityError::NoSource)?;
    Ok(EligibilitySettings {
        guild_id,
        min_account_age,
    })
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

#[cfg(test)]
mod web_origin_tests {
    use super::web_origin_from;

    #[test]
    fn a_bare_origin_is_kept_and_a_trailing_slash_is_dropped() {
        for raw in [
            "https://sorobanscan.rumblefish.dev",
            "https://sorobanscan.rumblefish.dev/",
            "  https://sorobanscan.rumblefish.dev//  ",
        ] {
            assert_eq!(
                web_origin_from(Some(raw)).as_deref(),
                Some("https://sorobanscan.rumblefish.dev"),
                "{raw:?}"
            );
        }
        assert_eq!(
            web_origin_from(Some("http://localhost:4200")).as_deref(),
            Some("http://localhost:4200")
        );
    }

    #[test]
    fn unset_blank_and_malformed_values_are_none() {
        for raw in [
            None,
            Some(""),
            Some("   "),
            Some("/"),
            Some("sorobanscan.rumblefish.dev"),
            Some("https://sorobanscan.rumblefish.dev/api/"),
            Some("https://sorobanscan.rumblefish.dev?x=1"),
            Some("https://"),
        ] {
            assert_eq!(web_origin_from(raw), None, "{raw:?}");
        }
    }
}

#[cfg(test)]
mod portal_load_tests {
    use super::AppConfig;

    /// The unit under test is the decision, not the loaders: with the portal
    /// open and no source configured, a loader fails and the config must come
    /// back CLOSED — flag and all three sources — rather than half-open. No
    /// environment variable is set here on purpose (`set_var` races the other
    /// test threads, see `AppConfig::portal_endpoints`): the loaders read
    /// `PORTAL_OAUTH_SECRET_FILE`/`_NAME`, `PORTAL_FREE_PLAN_ID`/`_PARAM` and
    /// the eligibility seams, and a developer's shell exporting one of them
    /// only moves which loader fails, not the outcome asserted.
    #[tokio::test]
    async fn a_failed_load_closes_the_portal_and_keeps_none_of_its_sources() {
        let mut config = AppConfig {
            portal_enabled: true,
            ..AppConfig::from_env()
        };

        let err = config
            .load_portal_or_close()
            .await
            .expect_err("no portal source is configured in a unit test");

        assert!(
            !config.portal_enabled,
            "the portal must be closed after a failed load ({err})"
        );
        assert!(config.portal_oauth.is_none());
        assert!(config.portal_keys.is_none());
        assert!(config.portal_eligibility.is_none());
        // The message names the variable the runbook step sets.
        assert!(err.to_string().starts_with("portal "), "{err}");
    }

    #[tokio::test]
    async fn a_closed_portal_loads_nothing_and_stays_closed() {
        let mut config = AppConfig {
            portal_enabled: false,
            ..AppConfig::from_env()
        };

        config
            .load_portal_or_close()
            .await
            .expect("a closed portal has nothing to load and nothing to fail");

        assert!(!config.portal_enabled);
        assert!(config.portal_oauth.is_none());
        assert!(config.portal_keys.is_none());
        assert!(config.portal_eligibility.is_none());
    }
}
