//! The portal's five sources, loaded on the first portal request that needs
//! them — never at cold start (task 0311).
//!
//! The five are the Discord OAuth secret (Secrets Manager) and four SSM
//! parameters — the free-plan id, the REST API id, the guild id and the
//! minimum account age — all read through the Parameters and Secrets
//! extension by [`crate::config::load_portal_sources`].
//!
//! # Why lazily
//!
//! They used to be read in `main.rs` before the router existed, on every cold
//! start of a Lambda that also serves `/v1`. Measured 2026-09-24/25 (lore note
//! `R-five-plan-herd-and-capacity-test.md`): a burst of `/v1` cold starts
//! throttles SSM, Init grows from ~420 ms to 2.3 s for sources `/v1` never
//! uses, and one failed read closed the portal in that execution environment
//! for its whole life (7 of 68 environments on 09-24). Now a `/v1` cold start
//! reads only the mTLS bundle, and the portal pays for its own sources on its
//! own first request.
//!
//! # Why a failure is kept for at most a cooldown
//!
//! The lifetime closure was the defect. A failed load answers **that request**
//! as unavailable (`/config` `enabled: false`, `503` on `/key`, `/usage` and
//! `/me`, a failure landing on sign-in); a success is kept for the
//! environment's life. Still closed, not crashed: the load never panics,
//! because a panic here would be a `502` on the function that also serves
//! `/v1`, over sources `/v1` does not use.
//!
//! A failure is remembered for [`LOAD_FAILURE_COOLDOWN`] and no longer (review
//! WR-02). Inside it, a portal request answers as unavailable **without**
//! loading and without a second alarm-prefixed ERROR line; the first request
//! after it loads again. Without the cooldown, `/config` — anonymous, and
//! called on every page view — re-ran all five reads with their retries on
//! every request while SSM was throttling, prolonging the very throttle this
//! task escapes. Two seconds bounds that to one load per environment per
//! cooldown and still leaves nothing broken for longer than a reload.
//!
//! # Single flight
//!
//! [`tokio::sync::OnceCell::get_or_try_init`] runs one init at a time and
//! hands a success to every waiter. A failure goes to its own caller only;
//! the waiters queued behind it then find the cooldown recorded and answer
//! unavailable without a load of their own — the failure is recorded inside
//! the init, before the next waiter can start one. [`LOAD_BUDGET`] wraps the
//! whole wait. Standard Lambda runs one request per environment at a time,
//! so in production queued waiters exist only in `serve` and the tests.
//!
//! The load is awaited inside the handler, never `tokio::spawn`ed: Lambda
//! freezes spawned work after the response. And the loader must never call
//! [`PortalSources::get`] on its own cell — that deadlocks.
//!
//! [`Loaded`] carries the OAuth secret, so it does not derive `Debug` and must
//! never be logged with `{:?}`.
//!
//! # The hub's secret is a source of its own
//!
//! The hub contract's secret (task 0348, `crate::portal::hub::secret`) is
//! not one of the five: [`HubSource`], carried by every [`PortalSources`]
//! handle beside its cell, loads it on the first hub call that needs it and
//! never on a portal request — the same reason the five moved off `/v1`'s
//! cold start. It is the same machinery as the five's, at a smaller scale:
//! one read at a time, a success kept for the environment's life, a failure
//! kept for [`LOAD_FAILURE_COOLDOWN`] and then read again, all inside a
//! budget of its own (`HUB_SECRET_BUDGET`). Its failures close the contract
//! for that request (`503 hub_unconfigured`) and nothing else: they are not
//! the five's alarm line, and no portal route reads them.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::OnceCell;
use tokio::time::Instant;

use crate::config::{AppConfig, PortalLoadError};
use crate::portal::auth::secret::OauthSecret;
use crate::portal::eligibility::EligibilitySettings;
use crate::portal::hub::secret::{HubSecret, SecretError as HubSecretError};
use crate::portal::keys::gateway::Gateway;

/// The whole load's ceiling, the five reads concurrent inside it.
///
/// Each read is bounded at 2 s by the extension client and tried up to three
/// times (`portal::extension`), so a fast failure gets its retries and a hung
/// read about two attempts. Four seconds keeps a load plus the
/// slowest route after it inside the 15 s invocation — pinned below for
/// `/usage` and `/key`, and in `auth::issue` for the callback.
pub(crate) const LOAD_BUDGET: Duration = Duration::from_secs(4);

/// One read of the hub's secret, with its single-flight wait (task 0348).
///
/// No longer than [`LOAD_BUDGET`], because a hub call's first request in an
/// environment awaits both — concurrently, so the slower of the two and not
/// their sum sits in front of the control plane (pinned below). Three seconds
/// still gives a fast failure its retries and a hung read its first
/// attempt's 2 s (`portal::extension`).
pub(crate) const HUB_SECRET_BUDGET: Duration = Duration::from_secs(3);

/// How long a failed load is remembered: inside it, [`PortalSources::get`]
/// answers unavailable without loading (see the module doc).
///
/// Two seconds: long enough that a burst of page views — `/config` fires on
/// every one — costs one load per environment rather than one each, short
/// enough that a visitor who reloads after a transient fault finds it gone.
pub const LOAD_FAILURE_COOLDOWN: Duration = Duration::from_secs(2);

/// What a load produced.
///
/// The production loader returns all three `Some`, or fails. `Default` — all
/// `None` — is "loaded, nothing provisioned", which only test fixtures (and a
/// closed portal) produce; every handler already answers that shape with its
/// unprovisioned response.
#[derive(Clone, Default)]
pub struct Loaded {
    pub oauth: Option<Arc<OauthSecret>>,
    pub gateway: Option<Arc<Gateway>>,
    pub settings: Option<Arc<EligibilitySettings>>,
}

impl Loaded {
    /// The sources a caller put on the config (`serve.rs`, tests).
    pub(crate) fn from_config(config: &AppConfig) -> Loaded {
        Loaded {
            oauth: config.portal_oauth.clone().map(Arc::new),
            gateway: config.portal_keys.clone().map(Arc::new),
            settings: config.portal_eligibility.clone().map(Arc::new),
        }
    }
}

/// One load attempt.
pub type LoadFuture = Pin<Box<dyn Future<Output = Result<Loaded, PortalLoadError>> + Send>>;

/// Starts a load attempt. Called once per attempt, never concurrently on one
/// cell.
pub type Loader = Arc<dyn Fn() -> LoadFuture + Send + Sync>;

/// A shared handle to the portal's sources. Clones share one cell, so every
/// portal state built from one router loads once per environment.
#[derive(Clone)]
pub struct PortalSources {
    cell: Arc<OnceCell<Loaded>>,
    loader: Option<Loader>,
    /// When the last load failed, for [`LOAD_FAILURE_COOLDOWN`]. Shared by
    /// every clone, like the cell.
    last_failure: Arc<Mutex<Option<Instant>>>,
    /// The hub contract's secret (task 0348): a source of its own, loaded
    /// apart from the cell above (see the module doc).
    hub: HubSource,
}

/// Why one ask came back without the sources.
enum Unavailable {
    /// A load ran and failed.
    Failed(PortalLoadError),
    /// A load failed this long ago, inside the cooldown; none was started.
    CoolingDown(Duration),
}

/// Records a failure when dropped, unless disarmed by a success.
///
/// A drop rather than a line after the `await`, so a load cancelled by
/// [`LOAD_BUDGET`] is recorded too — and recorded while the cell's init
/// permit is still held, before the next waiter can start a load of its own.
/// A load cancelled because its request went away counts as well: it starts
/// a cooldown without an ERROR line, which errs toward loading less.
struct FailureRecorder<'a> {
    last_failure: &'a Mutex<Option<Instant>>,
    armed: bool,
}

impl Drop for FailureRecorder<'_> {
    fn drop(&mut self) {
        if self.armed {
            *lock(self.last_failure) = Some(Instant::now());
        }
    }
}

/// The failure clock's lock. A poisoned one still holds a valid instant: the
/// only writer stores one value and cannot panic half-way.
fn lock(m: &Mutex<Option<Instant>>) -> std::sync::MutexGuard<'_, Option<Instant>> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl PortalSources {
    /// Sources already in hand: the cell starts initialised and nothing is
    /// ever loaded.
    pub fn ready(loaded: Loaded) -> PortalSources {
        PortalSources {
            cell: Arc::new(OnceCell::new_with(Some(loaded))),
            loader: None,
            last_failure: Arc::default(),
            hub: HubSource::ready(None),
        }
    }

    /// The Lambda's loader: the five reads from the environment, and the
    /// hub's secret from the environment, each loaded when first asked for.
    pub(crate) fn from_env() -> PortalSources {
        PortalSources {
            hub: HubSource::from_env(),
            ..PortalSources::with_loader(Arc::new(
                || Box::pin(crate::config::load_portal_sources()),
            ))
        }
    }

    /// A loader of the caller's choosing — a test seam, compiled out of the
    /// Lambda so the deployed build has exactly one loader, the env one.
    #[cfg(not(feature = "lambda"))]
    pub fn lazy(loader: Loader) -> PortalSources {
        PortalSources::with_loader(loader)
    }

    fn with_loader(loader: Loader) -> PortalSources {
        PortalSources {
            cell: Arc::new(OnceCell::new()),
            loader: Some(loader),
            last_failure: Arc::default(),
            hub: HubSource::ready(None),
        }
    }

    /// These sources with `hub` as the hub's secret — a test seam, compiled
    /// out of the Lambda like [`PortalSources::lazy`]: the deployed build
    /// gets its hub source from `PortalSources::from_env` or none at all.
    #[cfg(not(feature = "lambda"))]
    pub fn with_hub(self, hub: HubSource) -> PortalSources {
        PortalSources { hub, ..self }
    }

    /// The hub contract's secret (task 0348). Unconfigured unless these
    /// sources are the Lambda's own (or a test gave it one): a closed portal,
    /// and one whose sources the caller supplied (`serve.rs`), get none.
    pub fn hub(&self) -> &HubSource {
        &self.hub
    }

    /// The sources, loading them if this is the first ask (or every earlier
    /// ask failed). `None` means this request answers as unavailable.
    ///
    /// A failed load logs one alarm-prefixed ERROR here. An ask inside the
    /// [`LOAD_FAILURE_COOLDOWN`] after it starts no load and logs one WARN
    /// without the prefix, so the alarm counts loads, not requests.
    pub async fn get(&self) -> Option<&Loaded> {
        if let Some(loaded) = self.cell.get() {
            return Some(loaded);
        }
        // A ready cell is always initialised, so only a lazy one gets here.
        let loader = self.loader.as_ref()?;
        let attempt = self.cell.get_or_try_init(|| async {
            // Checked inside the init, not before it: a request queued behind
            // a failing load gets here after that failure was recorded.
            if let Some(ago) = self.failed_within_cooldown() {
                return Err(Unavailable::CoolingDown(ago));
            }
            let mut recorder = FailureRecorder {
                last_failure: &self.last_failure,
                armed: true,
            };
            let loaded = loader().await.map_err(Unavailable::Failed)?;
            recorder.armed = false;
            Ok(loaded)
        });
        let err = match tokio::time::timeout(LOAD_BUDGET, attempt).await {
            Ok(Ok(loaded)) => return Some(loaded),
            Ok(Err(Unavailable::Failed(err))) => err,
            Ok(Err(Unavailable::CoolingDown(ago))) => {
                // Not the alarm's prefix: this request started no load.
                tracing::warn!(
                    failed_ms_ago = ago.as_millis() as u64,
                    cooldown_ms = LOAD_FAILURE_COOLDOWN.as_millis() as u64,
                    "portal request answered as unavailable without a load: the last load \
                     failed inside the cooldown"
                );
                return None;
            }
            Err(_) => PortalLoadError::TimedOut(LOAD_BUDGET),
        };
        // The alarm's string: `prices-${env}-api-handler-portal-load-failed`
        // matches this literal's prefix, pinned by
        // `tools/scripts/portal-load-failed-filter-guard.test.mjs`.
        tracing::error!(
            error = %err,
            "portal sources failed to load; this request answers as unavailable and the first \
             portal request after a short cooldown retries; /v1 is unaffected"
        );
        None
    }

    /// How long ago the last load failed, if that is inside the cooldown.
    fn failed_within_cooldown(&self) -> Option<Duration> {
        failed_within_cooldown(&self.last_failure)
    }
}

/// How long ago `last_failure` was, if that is inside [`LOAD_FAILURE_COOLDOWN`].
fn failed_within_cooldown(last_failure: &Mutex<Option<Instant>>) -> Option<Duration> {
    let failed_at = (*lock(last_failure))?;
    let ago = failed_at.elapsed();
    (ago < LOAD_FAILURE_COOLDOWN).then_some(ago)
}

/// Which handle `portal::apply` builds for `config`.
///
/// A closed portal, or one whose caller already supplied a source
/// (`serve.rs`, which loads eagerly, and every test fixture), gets an
/// initialised cell from the config. Only an open portal with nothing supplied
/// — the Lambda — loads lazily from the environment.
pub(crate) fn sources_for(config: &AppConfig) -> PortalSources {
    let supplied = config.portal_oauth.is_some()
        || config.portal_keys.is_some()
        || config.portal_eligibility.is_some();
    if !config.portal_enabled || supplied {
        PortalSources::ready(Loaded::from_config(config))
    } else {
        PortalSources::from_env()
    }
}

/// One read of the hub's secret: `Ok(None)` is "none configured".
pub type HubLoadFuture =
    Pin<Box<dyn Future<Output = Result<Option<HubSecret>, HubSecretError>> + Send>>;

/// Starts a read of the hub's secret. Called once per attempt, never
/// concurrently on one source.
pub type HubLoader = Arc<dyn Fn() -> HubLoadFuture + Send + Sync>;

/// The hub contract's secret (task 0348), loaded on the first hub call that
/// asks for it — [`PortalSources`]'s machinery for one value.
///
/// - **A success is kept for the environment's life**: the secret, or the
///   fact that none is configured (`PORTAL_HUB_SECRET_NAME` unset), which no
///   later read could change — a Lambda's environment variables are fixed for
///   its life.
/// - **A failure is kept for [`LOAD_FAILURE_COOLDOWN`] and no longer**: a
///   fetch that failed, a secret that did not parse, or a read cut at
///   `HUB_SECRET_BUDGET`. That call answers `503 hub_unconfigured`, an ask
///   inside the cooldown answers the same without a read, and the first ask
///   after it reads again — so a throttled or briefly missing secret costs
///   one read per environment per cooldown, not one per call, and a secret
///   the operator fixes is picked up without a recycle. This is the
///   lifetime closure task 0311 took off the portal, kept off the contract.
/// - **One read at a time**, as the five: [`OnceCell::get_or_try_init`], with
///   the failure recorded inside the init (by drop, so a read cut by the
///   budget counts) before the next waiter can start one.
///
/// Clones share one source. Holds the secret, so it does not derive `Debug`.
#[derive(Clone)]
pub struct HubSource {
    cell: Arc<OnceCell<Option<Arc<HubSecret>>>>,
    loader: Option<HubLoader>,
    last_failure: Arc<Mutex<Option<Instant>>>,
}

/// Why one ask came back without the hub's secret.
enum HubUnavailable {
    /// A read ran and failed.
    Failed(HubSecretError),
    /// A read failed this long ago, inside the cooldown; none was started.
    CoolingDown(Duration),
}

impl HubSource {
    /// A source already settled: `Some` is the secret, `None` the contract
    /// unconfigured. Nothing is ever read.
    pub fn ready(secret: Option<HubSecret>) -> HubSource {
        HubSource {
            cell: Arc::new(OnceCell::new_with(Some(secret.map(Arc::new)))),
            loader: None,
            last_failure: Arc::default(),
        }
    }

    /// The Lambda's: `crate::config::hub_secret_from_env`.
    pub(crate) fn from_env() -> HubSource {
        HubSource::with_loader(Arc::new(|| Box::pin(crate::config::hub_secret_from_env())))
    }

    /// A loader of the caller's choosing — a test seam, compiled out of the
    /// Lambda like [`PortalSources::lazy`].
    #[cfg(not(feature = "lambda"))]
    pub fn lazy(loader: HubLoader) -> HubSource {
        HubSource::with_loader(loader)
    }

    fn with_loader(loader: HubLoader) -> HubSource {
        HubSource {
            cell: Arc::new(OnceCell::new()),
            loader: Some(loader),
            last_failure: Arc::default(),
        }
    }

    /// The secret, reading it if no read has succeeded yet. `None` means this
    /// call answers `503 hub_unconfigured`: none is configured, or the read
    /// failed now or inside the cooldown before.
    ///
    /// Logs as the five's load does, without its alarm's line: one ERROR per
    /// failed read, one WARN per ask inside the cooldown, and one WARN when
    /// the read finds none configured.
    pub async fn get(&self) -> Option<Arc<HubSecret>> {
        if let Some(secret) = self.cell.get() {
            return secret.clone();
        }
        // A ready source is always initialised, so only a lazy one gets here.
        let loader = self.loader.as_ref()?;
        let attempt = self.cell.get_or_try_init(|| async {
            // Inside the init, as in `PortalSources::get`: a call queued
            // behind a failing read gets here after that failure was recorded.
            if let Some(ago) = failed_within_cooldown(&self.last_failure) {
                return Err(HubUnavailable::CoolingDown(ago));
            }
            let mut recorder = FailureRecorder {
                last_failure: &self.last_failure,
                armed: true,
            };
            let secret = loader().await.map_err(HubUnavailable::Failed)?;
            recorder.armed = false;
            if secret.is_none() {
                tracing::warn!(
                    "the hub contract has no secret (PORTAL_HUB_SECRET_NAME is unset): its calls \
                     answer 503 hub_unconfigured; the portal is unaffected"
                );
            }
            Ok(secret.map(Arc::new))
        });
        match tokio::time::timeout(HUB_SECRET_BUDGET, attempt).await {
            Ok(Ok(secret)) => secret.clone(),
            Ok(Err(HubUnavailable::CoolingDown(ago))) => {
                tracing::warn!(
                    failed_ms_ago = ago.as_millis() as u64,
                    cooldown_ms = LOAD_FAILURE_COOLDOWN.as_millis() as u64,
                    "hub contract call answered as unconfigured without a read: the last read \
                     of its secret failed inside the cooldown"
                );
                None
            }
            Ok(Err(HubUnavailable::Failed(err))) => {
                tracing::error!(
                    error = %err,
                    "the hub contract's secret did not load (PORTAL_HUB_SECRET_NAME): this call \
                     answers 503 hub_unconfigured and the first hub call after a short cooldown \
                     reads it again; the portal is unaffected"
                );
                None
            }
            Err(_) => {
                tracing::error!(
                    budget_ms = HUB_SECRET_BUDGET.as_millis() as u64,
                    "the hub contract's secret did not load within its budget \
                     (PORTAL_HUB_SECRET_NAME): this call answers 503 hub_unconfigured and the \
                     first hub call after a short cooldown reads it again; the portal is \
                     unaffected"
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::portal::auth::secret::SecretError;

    /// What one scripted attempt does.
    #[derive(Clone)]
    enum Step {
        Fail,
        Succeed,
        Hang,
    }

    /// A loader that plays `script` (repeating the last step) after `delay`,
    /// counting its calls.
    fn scripted(script: &[Step], delay: Duration) -> (Loader, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let steps = Arc::new(Mutex::new(script.iter().cloned().collect::<VecDeque<_>>()));
        let last = Arc::new(Mutex::new(Step::Fail));
        let counter = calls.clone();
        let loader: Loader = Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            let step = {
                let mut last = last.lock().unwrap();
                if let Some(step) = steps.lock().unwrap().pop_front() {
                    *last = step;
                }
                last.clone()
            };
            Box::pin(async move {
                tokio::time::sleep(delay).await;
                match step {
                    Step::Fail => Err(PortalLoadError::Oauth(SecretError::NoSource)),
                    Step::Succeed => Ok(Loaded::default()),
                    Step::Hang => std::future::pending().await,
                }
            })
        });
        (loader, calls)
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_first_asks_share_one_successful_load() {
        let (loader, calls) = scripted(&[Step::Succeed], Duration::from_millis(50));
        let sources = PortalSources::lazy(loader);
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let sources = sources.clone();
            set.spawn(async move { sources.get().await.is_some() });
        }
        while let Some(loaded) = set.join_next().await {
            assert!(loaded.unwrap());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_is_kept_only_for_the_cooldown_and_a_success_for_good() {
        let (loader, calls) = scripted(&[Step::Fail, Step::Succeed], Duration::ZERO);
        let sources = PortalSources::lazy(loader);
        assert!(sources.get().await.is_none());
        tokio::time::advance(LOAD_FAILURE_COOLDOWN).await;
        assert!(sources.get().await.is_some());
        assert!(sources.get().await.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_load_past_the_budget_gives_up_and_the_first_ask_after_the_cooldown_loads_again() {
        let (loader, calls) = scripted(&[Step::Hang, Step::Succeed], Duration::ZERO);
        let sources = PortalSources::lazy(loader);
        let started = tokio::time::Instant::now();
        assert!(sources.get().await.is_none());
        let waited = started.elapsed();
        assert!(
            waited >= LOAD_BUDGET && waited < LOAD_BUDGET + Duration::from_millis(50),
            "{waited:?}"
        );
        // A timed-out load is a failed one: it starts the cooldown too.
        assert!(sources.get().await.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        tokio::time::advance(LOAD_FAILURE_COOLDOWN).await;
        assert!(sources.get().await.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// Review WR-02. Inside the cooldown an ask starts no load — the retry
    /// storm against a throttled SSM is bounded to one load per cooldown —
    /// and the first ask after it loads. That it adds no alarm-prefixed
    /// ERROR line is asserted in `tests/portal_load_logs.rs`, a binary of its
    /// own: `tracing`'s callsite cache makes a log capture beside parallel
    /// tests unreliable (see that file).
    #[tokio::test(start_paused = true)]
    async fn inside_the_cooldown_an_ask_does_not_load() {
        let (loader, calls) = scripted(&[Step::Fail, Step::Succeed], Duration::ZERO);
        let sources = PortalSources::lazy(loader);

        assert!(sources.get().await.is_none());
        for _ in 0..5 {
            tokio::time::advance(LOAD_FAILURE_COOLDOWN / 10).await;
            assert!(sources.get().await.is_none());
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a load ran inside the cooldown"
        );

        tokio::time::advance(LOAD_FAILURE_COOLDOWN).await;
        assert!(sources.get().await.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// Requests queued behind a failing load do not each run one of their
    /// own: the failure is recorded before the next waiter's init starts.
    #[tokio::test(start_paused = true)]
    async fn waiters_behind_a_failing_load_do_not_load_again() {
        let (loader, calls) = scripted(&[Step::Fail], Duration::from_millis(50));
        let sources = PortalSources::lazy(loader);
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let sources = sources.clone();
            set.spawn(async move { sources.get().await.is_some() });
        }
        while let Some(loaded) = set.join_next().await {
            assert!(!loaded.unwrap());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ready_sources_answer_without_a_loader() {
        let sources = PortalSources::ready(Loaded::default());
        assert!(sources.loader.is_none());
        assert!(sources.get().await.is_some());
    }

    fn config(portal_enabled: bool) -> AppConfig {
        AppConfig {
            ch_enabled: false,
            base_url: None,
            api_keys: vec![],
            portal_enabled,
            portal_oauth: None,
            portal_endpoints: Default::default(),
            portal_keys: None,
            portal_eligibility: None,
            portal_rate_limit: None,
            portal_web_origin: None,
        }
    }

    /// Review WR-05. What keeps a closed Lambda from ever building a
    /// control-plane client (`Gateway::from_ambient_config`) is that
    /// `sources_for` gives it no loader at all — the gate's `404` is the
    /// second line, not the first. Only an open portal with nothing supplied
    /// gets the environment loader.
    #[tokio::test]
    async fn a_closed_portal_never_gets_a_loader() {
        let closed = sources_for(&config(false));
        assert!(closed.loader.is_none(), "a closed portal holds a loader");
        let loaded = closed.get().await.expect("a closed portal's cell is ready");
        assert!(loaded.oauth.is_none() && loaded.gateway.is_none() && loaded.settings.is_none());

        assert!(sources_for(&config(true)).loader.is_some());
    }

    /// A load in front of the slowest routes must still leave the answer
    /// inside the invocation. The 15 is `apiHandler.timeoutSeconds` in
    /// `infra/envs/production.json`; the callback's share is pinned in
    /// `auth::issue`.
    #[test]
    fn the_load_budget_fits_in_front_of_every_route() {
        const LAMBDA_TIMEOUT: Duration = Duration::from_secs(15);
        assert!(LOAD_BUDGET < LAMBDA_TIMEOUT);
        assert!(LOAD_BUDGET + crate::portal::usage::USAGE_DEADLINE < LAMBDA_TIMEOUT);
        assert!(LOAD_BUDGET + crate::portal::keys::RECONCILE_DEADLINE < LAMBDA_TIMEOUT);
    }
}

#[cfg(test)]
mod hub_tests {
    //! The hub's secret as a source of its own (task 0348): a failed read
    //! closes the contract for its call only, and the portal never notices.

    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, UNIX_EPOCH};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;
    use crate::portal::hub::token::{self, Act};

    /// The spec's test vector (decision 15): its secret, its token, and an
    /// instant it holds at.
    const VECTOR_SECRET: &str = r#"{"current":"AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA="}"#;
    const VECTOR_TOKEN: &str = "v1.eyJpc3MiOiJodHRwczovL3Nvcm9iYW5zY2FuLmNvbS9hcGkvaHViIiwiYXVkIjoicHJpY2VzIiwic3ViIjoiZGlzY29yZDo4MDM1MTExMDIyNDY3ODkxMiIsIm5hbWUiOiJuZWxseSIsImlhdCI6MTc5NTAwMDAwMCwiZXhwIjoxNzk1MDAwMDYwLCJqdGkiOiJBQUVDQXdRRkJnY0lDUW9MREEwT0R3IiwiYWN0IjoicmV2ZWFsIn0.aeS9_nACshTGuf2Afv4wbQhvLR-f64XCX0OVmmngofE";
    const VECTOR_VALID_AT: u64 = 1_795_000_030;

    /// What one scripted read does.
    #[derive(Clone)]
    enum Read {
        /// The extension could not fetch it.
        FetchFails,
        /// It came back, and is not the secret's JSON.
        Malformed,
        /// It never answers.
        Hangs,
        /// No secret is configured.
        Absent,
        /// The vector's secret.
        Succeeds,
    }

    /// A hub loader that plays `script` (repeating the last step), counting
    /// its reads.
    fn scripted(script: &[Read]) -> (HubSource, Arc<AtomicUsize>) {
        let reads = Arc::new(AtomicUsize::new(0));
        let steps = Arc::new(Mutex::new(script.iter().cloned().collect::<VecDeque<_>>()));
        let last = Arc::new(Mutex::new(Read::FetchFails));
        let counter = reads.clone();
        let loader: HubLoader = Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            let step = {
                let mut last = last.lock().unwrap();
                if let Some(step) = steps.lock().unwrap().pop_front() {
                    *last = step;
                }
                last.clone()
            };
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                match step {
                    Read::FetchFails => Err(HubSecretError::Fetch {
                        name: "sorobanscan-hub/production/prices-token".into(),
                        message: "extension returned HTTP 503 Service Unavailable".into(),
                    }),
                    Read::Malformed => HubSecret::parse("{}").map(Some),
                    Read::Hangs => std::future::pending().await,
                    Read::Absent => Ok(None),
                    Read::Succeeds => HubSecret::parse(VECTOR_SECRET).map(Some),
                }
            })
        });
        (HubSource::lazy(loader), reads)
    }

    /// The vector's token verifies under `secret`.
    fn verifies_the_vector(secret: &HubSecret) -> bool {
        let now = UNIX_EPOCH + Duration::from_secs(VECTOR_VALID_AT);
        token::verify(VECTOR_TOKEN, secret, now).is_some_and(|caller| {
            caller.discord_id == "80351110224678912" && caller.act == Act::Reveal
        })
    }

    fn open_portal() -> AppConfig {
        AppConfig {
            ch_enabled: false,
            base_url: None,
            api_keys: vec![],
            portal_enabled: true,
            portal_oauth: None,
            portal_endpoints: Default::default(),
            portal_keys: None,
            portal_eligibility: None,
            portal_rate_limit: None,
            portal_web_origin: None,
        }
    }

    /// The portal's own sources, lazily loaded and always successful,
    /// counting their loads.
    fn portal_sources() -> (PortalSources, Arc<AtomicUsize>) {
        let loads = Arc::new(AtomicUsize::new(0));
        let counter = loads.clone();
        let sources = PortalSources::lazy(Arc::new(move || -> LoadFuture {
            counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(Loaded::default()) })
        }));
        (sources, loads)
    }

    /// `GET /api/config`'s `enabled`.
    async fn config_enabled(router: &axum::Router) -> bool {
        let resp = router
            .clone()
            .oneshot(
                Request::get(crate::portal::CONFIG_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["enabled"] == true
    }

    /// The ruling's case, end to end: the first read fails, the contract is
    /// closed for that call and through the cooldown, the first call after it
    /// reads again and the secret verifies the vector's token — and the
    /// portal is open and loads once throughout, never reading the hub's.
    #[tokio::test(start_paused = true)]
    async fn a_failed_read_is_retried_after_the_cooldown_and_then_verifies_tokens() {
        let (hub, reads) = scripted(&[Read::FetchFails, Read::Succeeds]);
        let (sources, loads) = portal_sources();
        let sources = sources.with_hub(hub);
        let router = crate::app_with_portal(
            &open_portal(),
            crate::AppState::without_ch(),
            sources.clone(),
        );

        assert!(config_enabled(&router).await);
        assert_eq!(
            reads.load(Ordering::SeqCst),
            0,
            "a portal request read the hub's"
        );

        assert!(
            sources.hub().get().await.is_none(),
            "a failed read gave a secret"
        );
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert!(
            config_enabled(&router).await,
            "a failed hub read closed the portal"
        );

        tokio::time::advance(LOAD_FAILURE_COOLDOWN / 2).await;
        assert!(sources.hub().get().await.is_none());
        assert_eq!(
            reads.load(Ordering::SeqCst),
            1,
            "a read ran inside the cooldown"
        );

        tokio::time::advance(LOAD_FAILURE_COOLDOWN).await;
        let secret = sources
            .hub()
            .get()
            .await
            .expect("the read after the cooldown");
        assert!(verifies_the_vector(&secret));
        assert_eq!(reads.load(Ordering::SeqCst), 2);

        // A success is kept: no read again, however long after.
        tokio::time::advance(LOAD_FAILURE_COOLDOWN * 100).await;
        let again = sources.hub().get().await.expect("the kept secret");
        assert!(verifies_the_vector(&again));
        assert_eq!(reads.load(Ordering::SeqCst), 2);

        assert!(config_enabled(&router).await);
        assert_eq!(
            loads.load(Ordering::SeqCst),
            1,
            "the portal loaded more than once"
        );
        assert_eq!(
            reads.load(Ordering::SeqCst),
            2,
            "a portal request read the hub's"
        );
    }

    /// A secret that does not parse is a failure like any other: retried
    /// after the cooldown, so the operator's fix is picked up without a
    /// recycle.
    #[tokio::test(start_paused = true)]
    async fn a_malformed_secret_is_read_again_after_the_cooldown() {
        let (hub, reads) = scripted(&[Read::Malformed, Read::Succeeds]);
        assert!(hub.get().await.is_none());
        tokio::time::advance(LOAD_FAILURE_COOLDOWN).await;
        assert!(verifies_the_vector(&hub.get().await.unwrap()));
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    /// A hung read is cut at the hub's own budget, starts the cooldown, and
    /// the first ask after it reads again.
    #[tokio::test(start_paused = true)]
    async fn a_hung_read_is_cut_at_its_budget_and_retried_after_the_cooldown() {
        let (hub, reads) = scripted(&[Read::Hangs, Read::Succeeds]);
        let started = tokio::time::Instant::now();
        assert!(hub.get().await.is_none());
        assert_eq!(started.elapsed(), HUB_SECRET_BUDGET);

        assert!(hub.get().await.is_none());
        assert_eq!(
            reads.load(Ordering::SeqCst),
            1,
            "a read ran inside the cooldown"
        );

        tokio::time::advance(LOAD_FAILURE_COOLDOWN).await;
        assert!(verifies_the_vector(&hub.get().await.unwrap()));
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    /// No secret configured is a settled answer, not a failure: the
    /// environment's variables cannot change, so it is never read again.
    #[tokio::test(start_paused = true)]
    async fn none_configured_is_kept_and_never_read_again() {
        let (hub, reads) = scripted(&[Read::Absent, Read::Succeeds]);
        assert!(hub.get().await.is_none());
        for _ in 0..5 {
            tokio::time::advance(LOAD_FAILURE_COOLDOWN * 2).await;
            assert!(hub.get().await.is_none());
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_first_asks_share_one_read() {
        let (hub, reads) = scripted(&[Read::Succeeds]);
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let hub = hub.clone();
            set.spawn(async move { hub.get().await.is_some() });
        }
        while let Some(got) = set.join_next().await {
            assert!(got.unwrap());
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    /// Calls queued behind a failing read do not each run one of their own.
    #[tokio::test(start_paused = true)]
    async fn waiters_behind_a_failing_read_do_not_read_again() {
        let (hub, reads) = scripted(&[Read::FetchFails]);
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let hub = hub.clone();
            set.spawn(async move { hub.get().await.is_some() });
        }
        while let Some(got) = set.join_next().await {
            assert!(!got.unwrap());
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_ready_source_answers_without_a_loader() {
        let configured = HubSource::ready(Some(HubSecret::parse(VECTOR_SECRET).unwrap()));
        assert!(configured.loader.is_none());
        assert!(verifies_the_vector(&configured.get().await.unwrap()));

        let unconfigured = HubSource::ready(None);
        assert!(unconfigured.loader.is_none());
        assert!(unconfigured.get().await.is_none());
    }

    /// Only the Lambda's own sources read the hub's secret: a closed portal
    /// and sources the caller supplied (`serve.rs`, the fixtures) get none.
    #[test]
    fn only_the_lambdas_sources_get_a_hub_loader() {
        let closed = AppConfig {
            portal_enabled: false,
            ..open_portal()
        };
        assert!(sources_for(&closed).hub().loader.is_none());
        assert!(
            PortalSources::ready(Loaded::default())
                .hub()
                .loader
                .is_none()
        );
        assert!(portal_sources().0.hub().loader.is_none());
        assert!(sources_for(&open_portal()).hub().loader.is_some());
    }

    /// A hub call's first request in an environment waits on the portal's
    /// load and the hub's read, concurrently, and then on the control plane.
    /// Either way it has to finish inside the invocation: the 15 is
    /// `apiHandler.timeoutSeconds` in `infra/envs/production.json`.
    #[test]
    fn the_hub_budget_fits_beside_the_load_in_front_of_every_route() {
        const LAMBDA_TIMEOUT: Duration = Duration::from_secs(15);
        assert!(HUB_SECRET_BUDGET <= LOAD_BUDGET);
        let first = LOAD_BUDGET.max(HUB_SECRET_BUDGET);
        assert!(first + crate::portal::usage::USAGE_DEADLINE < LAMBDA_TIMEOUT);
        assert!(first + crate::portal::keys::RECONCILE_DEADLINE < LAMBDA_TIMEOUT);
    }
}
