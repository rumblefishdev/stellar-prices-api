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
    /// The free plan's per-key rate limit, requests per second, served by
    /// `/config` (task 0188).
    ///
    /// Read from `PORTAL_RATE_LIMIT`, which `compute-stack.ts` sets from
    /// `pricingApiFreePlanRateLimit` — the same config value
    /// `api-gateway-stack.ts` feeds to `addUsagePlan`. Since task 0311 the
    /// signed-in dashboard does not state this figure: `/api/usage` reads the
    /// key's OWN plan through `GetUsagePlans` and reports its figures. This
    /// stays for what has no key to ask about — the no-key state and the
    /// landing page; while the usage call is unanswered or failed the
    /// dashboard states no figure at all rather than this one — and it
    /// stays config-fed rather than a literal in the frontend, because a
    /// literal would drift from what the gateway enforces the moment
    /// `infra/envs/production.json` changed.
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
    /// leaves this `None`. In the Lambda it stays `None` and the portal loads
    /// its sources lazily, on the first portal request that needs them
    /// (`crate::portal::sources`). A value here is a pre-supplied source:
    /// `serve.rs` fills it eagerly with [`Self::load_portal_oauth`], and tests
    /// set it directly. Either way the portal then never loads from the
    /// environment.
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
    /// A pre-supplied source, like [`Self::portal_oauth`]: `None` in the
    /// Lambda, where the portal loads it lazily; filled by `serve.rs` through
    /// [`Self::load_portal_keys`], or directly by a test.
    pub portal_keys: Option<crate::portal::keys::gateway::Gateway>,
    /// Where the eligibility gate's two knobs come from (task 0189): the
    /// Stellar guild id and the minimum account age. A pre-supplied source,
    /// like [`Self::portal_oauth`]: `None` in the Lambda, where the portal
    /// loads it lazily; filled by `serve.rs` through
    /// [`Self::load_portal_eligibility`], or directly by a test.
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

    /// Fill [`Self::portal_oauth`] for `serve.rs`, which loads eagerly and
    /// `expect()`s: a developer who asked for the portal and did not get it
    /// wants to know now, and no partner is behind that process.
    ///
    /// A no-op while [`Self::portal_enabled`] is false, so the ordinary local
    /// run of the data API needs none of it. The Lambda never calls this: it
    /// loads through `load_portal_sources` on the first portal request.
    pub async fn load_portal_oauth(
        &mut self,
    ) -> Result<(), crate::portal::auth::secret::SecretError> {
        if !self.portal_enabled {
            return Ok(());
        }
        self.portal_oauth = Some(portal_oauth_from_env().await?);
        Ok(())
    }

    /// Fill [`Self::portal_keys`] for `serve.rs` — see
    /// [`Self::load_portal_oauth`] for why eagerly and why only there.
    ///
    /// The flag guard also keeps a closed portal away from the control plane:
    /// with the portal off there is no client in the process, so no code path
    /// can create or delete a production API key.
    pub async fn load_portal_keys(&mut self) -> Result<(), PortalKeysError> {
        if !self.portal_enabled {
            return Ok(());
        }
        self.portal_keys = Some(portal_keys_from_env().await?);
        Ok(())
    }

    /// Fill [`Self::portal_eligibility`] for `serve.rs` — see
    /// [`Self::load_portal_oauth`].
    pub async fn load_portal_eligibility(&mut self) -> Result<(), PortalEligibilityError> {
        if !self.portal_enabled {
            return Ok(());
        }
        self.portal_eligibility = Some(portal_eligibility_from_env().await?);
        Ok(())
    }
}

/// Read the Discord OAuth secret (task 0186) from Secrets Manager, or from the
/// local file named by `PORTAL_OAUTH_SECRET_FILE`. With the portal open, no
/// source at all is an error.
pub(crate) async fn portal_oauth_from_env()
-> Result<crate::portal::auth::secret::OauthSecret, crate::portal::auth::secret::SecretError> {
    crate::portal::auth::secret::OauthSecret::load()
        .await?
        .ok_or(crate::portal::auth::secret::SecretError::NoSource)
}

/// Build the control-plane client key issuance uses (task 0187).
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
///
/// # The API id and stage (task 0311)
///
/// `Gateway::plan_of` keeps only the usage plans on OUR API stage, so the
/// client also needs the REST API id and the stage name. The id arrives
/// exactly as the plan id does — `PORTAL_API_ID_PARAM` names the SSM
/// parameter `ApiGatewayStack` publishes at `/prices/{env}/api-gateway-id`,
/// with a `PORTAL_API_ID` override for a local run compiled out of the
/// Lambda — and the stage is the plain `PORTAL_API_STAGE`, because the
/// stage name is `envName` and needs no lookup. The two reads run
/// concurrently.
pub(crate) async fn portal_keys_from_env()
-> Result<crate::portal::keys::gateway::Gateway, PortalKeysError> {
    let (plan_id, api_id) = tokio::try_join!(free_plan_id(), api_id())?;
    let stage = api_stage()?;
    Ok(crate::portal::keys::gateway::Gateway::from_ambient_config(plan_id, api_id, stage).await)
}

/// The sources of the eligibility gate's two knobs (task 0189), each value
/// **probed once** so a mis-seeded parameter — `discord-guild-id` seeded
/// with a name instead of a snowflake, `min-account-age-minutes` holding
/// "five" — fails the load with the parameter named, rather than refusing
/// every visitor as "could not verify".
///
/// What is kept is the **source**, not the probed value: every issuance
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
pub(crate) async fn portal_eligibility_from_env()
-> Result<crate::portal::eligibility::EligibilitySettings, PortalEligibilityError> {
    let settings = eligibility_settings()?;
    tokio::try_join!(settings.guild_id(), settings.min_account_age_minutes())
        .map_err(PortalEligibilityError::Probe)?;
    Ok(settings)
}

/// The Lambda's portal load (`crate::portal::sources`): all five reads — the
/// OAuth secret, the free-plan id, the API id, the guild id and the minimum
/// account age — in flight at once, failing on the first error.
///
/// Concurrent because the whole load sits inside
/// `crate::portal::sources::LOAD_BUDGET`, in front of the request that asked:
/// five reads one after another could each take the extension client's 2 s.
pub(crate) async fn load_portal_sources() -> Result<crate::portal::sources::Loaded, PortalLoadError>
{
    let (oauth, gateway, settings) = tokio::try_join!(
        async { portal_oauth_from_env().await.map_err(PortalLoadError::from) },
        async { portal_keys_from_env().await.map_err(PortalLoadError::from) },
        async {
            portal_eligibility_from_env()
                .await
                .map_err(PortalLoadError::from)
        },
    )?;
    Ok(crate::portal::sources::Loaded {
        oauth: Some(std::sync::Arc::new(oauth)),
        gateway: Some(std::sync::Arc::new(gateway)),
        settings: Some(std::sync::Arc::new(settings)),
    })
}

/// Why the lazy portal load failed (`crate::portal::sources`): which source,
/// with the variable that names it, so the `portal sources failed to load`
/// line points at the runbook step.
#[derive(Debug, thiserror::Error)]
pub enum PortalLoadError {
    #[error("portal sign-in (PORTAL_OAUTH_SECRET_NAME): {0}")]
    Oauth(#[from] crate::portal::auth::secret::SecretError),
    #[error("portal key issuance ({var}): {0}", var = .0.variable())]
    Keys(#[from] PortalKeysError),
    #[error("portal eligibility gate (PORTAL_GUILD_ID_PARAM, PORTAL_MIN_ACCOUNT_AGE_PARAM): {0}")]
    Eligibility(#[from] PortalEligibilityError),
    #[error(
        "the portal's five reads (PORTAL_OAUTH_SECRET_NAME, PORTAL_FREE_PLAN_PARAM, \
         PORTAL_API_ID_PARAM, PORTAL_GUILD_ID_PARAM, PORTAL_MIN_ACCOUNT_AGE_PARAM) did not \
         finish within {0:?}"
    )]
    TimedOut(std::time::Duration),
}

/// Why the eligibility gate could not be configured.
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

/// Why key issuance could not be configured.
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
    #[error(
        "the portal is open but no REST API id is configured; set PORTAL_API_ID_PARAM to the SSM \
         parameter holding it (ApiGatewayStack publishes it at /prices/<env>/api-gateway-id), \
         or PORTAL_API_ID on a local run"
    )]
    ApiIdNoSource,
    #[error(
        "reading the REST API id from SSM parameter `{name}` (PORTAL_API_ID_PARAM) failed: {message}"
    )]
    ApiIdFetch { name: String, message: String },
    #[error("SSM parameter `{name}` (PORTAL_API_ID_PARAM) holds an empty REST API id")]
    ApiIdEmpty { name: String },
    #[error(
        "the portal is open but no API stage is configured; set PORTAL_API_STAGE to the stage \
         name (the environment name, e.g. `production`)"
    )]
    NoStage,
}

impl PortalKeysError {
    /// The variable naming the source that failed — the label on
    /// [`PortalLoadError::Keys`]. Per variant, because the API id and the
    /// stage are read from variables of their own (task 0311).
    pub fn variable(&self) -> &'static str {
        match self {
            PortalKeysError::NoSource
            | PortalKeysError::Fetch { .. }
            | PortalKeysError::Empty { .. } => "PORTAL_FREE_PLAN_PARAM",
            PortalKeysError::ApiIdNoSource
            | PortalKeysError::ApiIdFetch { .. }
            | PortalKeysError::ApiIdEmpty { .. } => "PORTAL_API_ID_PARAM",
            PortalKeysError::NoStage => "PORTAL_API_STAGE",
        }
    }
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
    let id = fetch_parameter(&name)
        .await
        .map_err(|message| PortalKeysError::Fetch {
            name: name.clone(),
            message,
        })?
        .trim()
        .to_string();
    if id.is_empty() {
        return Err(PortalKeysError::Empty { name });
    }
    Ok(id)
}

/// Resolve our REST API id (task 0311) — the same shape as [`free_plan_id`].
async fn api_id() -> Result<String, PortalKeysError> {
    // A direct id, for a local run. **Compiled out of the Lambda** for the
    // reason `PORTAL_FREE_PLAN_ID` is: this value decides which usage plans
    // count as the key's plan, and a configuration change must not be able to
    // point the portal at an API of somebody else's choosing.
    #[cfg(not(feature = "lambda"))]
    if let Ok(id) = std::env::var("PORTAL_API_ID")
        && !id.trim().is_empty()
    {
        return Ok(id.trim().to_string());
    }

    let Ok(name) = std::env::var("PORTAL_API_ID_PARAM") else {
        return Err(PortalKeysError::ApiIdNoSource);
    };
    if name.is_empty() {
        return Err(PortalKeysError::ApiIdNoSource);
    }
    // Trimmed for the reason the plan id is: an `echo | put-parameter`
    // newline would otherwise make every plan fail the stage comparison and
    // report every key as on no plan.
    let id = fetch_parameter(&name)
        .await
        .map_err(|message| PortalKeysError::ApiIdFetch {
            name: name.clone(),
            message,
        })?
        .trim()
        .to_string();
    if id.is_empty() {
        return Err(PortalKeysError::ApiIdEmpty { name });
    }
    Ok(id)
}

/// Our stage name (task 0311), from `PORTAL_API_STAGE`.
fn api_stage() -> Result<String, PortalKeysError> {
    std::env::var("PORTAL_API_STAGE")
        .ok()
        .map(|stage| stage.trim().to_string())
        .filter(|stage| !stage.is_empty())
        .ok_or(PortalKeysError::NoStage)
}

/// Read the parameter through the Parameters and Secrets extension — the same
/// localhost listener, token and in-process cache the mTLS bundle and the OAuth
/// secret already use, so a warm container never calls Systems Manager on the
/// path that issues a key.
///
/// Retried on a transient failure (`crate::portal::extension`). The error is
/// the message alone; the caller wraps it in the variant naming which
/// parameter it was reading.
#[cfg(feature = "aws-mtls")]
async fn fetch_parameter(name: &str) -> Result<String, String> {
    crate::portal::extension::parameter_string(name)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "aws-mtls"))]
async fn fetch_parameter(_name: &str) -> Result<String, String> {
    Err(
        "this build has no Parameters and Secrets extension client (build with \
         `--features lambda`, or set PORTAL_FREE_PLAN_ID and PORTAL_API_ID for a local run)"
            .into(),
    )
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
    use super::{AppConfig, PortalKeysError, PortalLoadError, load_portal_sources};

    /// With no source configured, the lazy load fails, and says which
    /// source. No environment variable is set here on purpose (`set_var`
    /// races the other test threads, see `AppConfig::portal_endpoints`): the
    /// loaders read `PORTAL_OAUTH_SECRET_FILE`/`_NAME`,
    /// `PORTAL_FREE_PLAN_ID`/`_PARAM`, `PORTAL_API_ID`/`_PARAM`,
    /// `PORTAL_API_STAGE` and the eligibility seams, and a developer's shell
    /// exporting one of them only moves which source fails, not the outcome
    /// asserted.
    #[tokio::test]
    async fn a_load_with_no_source_fails_and_names_the_source() {
        let err = match load_portal_sources().await {
            Ok(_) => panic!("no portal source is configured in a unit test"),
            Err(err) => err,
        };
        // The message names the variable the runbook step sets.
        assert!(err.to_string().starts_with("portal "), "{err}");
    }

    /// The label names the variable of the read that failed — not
    /// `PORTAL_FREE_PLAN_PARAM` for all seven, which it used to.
    #[test]
    fn a_key_issuance_failure_names_its_own_variable() {
        let name = || "/prices/production/x".to_string();
        let cases = [
            (PortalKeysError::NoSource, "PORTAL_FREE_PLAN_PARAM"),
            (
                PortalKeysError::Fetch {
                    name: name(),
                    message: "m".into(),
                },
                "PORTAL_FREE_PLAN_PARAM",
            ),
            (
                PortalKeysError::Empty { name: name() },
                "PORTAL_FREE_PLAN_PARAM",
            ),
            (PortalKeysError::ApiIdNoSource, "PORTAL_API_ID_PARAM"),
            (
                PortalKeysError::ApiIdFetch {
                    name: name(),
                    message: "m".into(),
                },
                "PORTAL_API_ID_PARAM",
            ),
            (
                PortalKeysError::ApiIdEmpty { name: name() },
                "PORTAL_API_ID_PARAM",
            ),
            (PortalKeysError::NoStage, "PORTAL_API_STAGE"),
        ];
        for (err, var) in cases {
            let line = PortalLoadError::Keys(err).to_string();
            assert!(
                line.starts_with(&format!("portal key issuance ({var}): ")),
                "{line}"
            );
        }
    }

    /// `serve.rs`'s eager loaders are no-ops on a closed portal: nothing is
    /// read, nothing fails, nothing is filled.
    #[tokio::test]
    async fn a_closed_portal_loads_nothing() {
        let mut config = AppConfig {
            portal_enabled: false,
            ..AppConfig::from_env()
        };

        config.load_portal_oauth().await.expect("nothing to load");
        config.load_portal_keys().await.expect("nothing to load");
        config
            .load_portal_eligibility()
            .await
            .expect("nothing to load");

        assert!(config.portal_oauth.is_none());
        assert!(config.portal_keys.is_none());
        assert!(config.portal_eligibility.is_none());
    }
}
