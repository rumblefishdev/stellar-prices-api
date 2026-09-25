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
//! # Why a failure is not cached
//!
//! The lifetime closure was the defect. A failed load answers **that request**
//! as unavailable (`/config` `enabled: false`, `503` on `/key`, `/usage` and
//! `/me`, a failure landing on sign-in) and the next portal request loads
//! again; a success is kept for the environment's life. Still closed, not
//! crashed: the load never panics, because a panic here would be a `502` on
//! the function that also serves `/v1`, over sources `/v1` does not use.
//!
//! # Single flight, for success only
//!
//! [`tokio::sync::OnceCell::get_or_try_init`] runs one init at a time and
//! hands a success to every waiter. A failure goes to its own caller only, and
//! the next waiter starts another attempt — so N waiters on a failing load run
//! N loads in turn. [`LOAD_BUDGET`] wraps the whole wait, which bounds each of
//! them. Standard Lambda runs one request per environment at a time, so in
//! production this bites only `serve` and the tests.
//!
//! The load is awaited inside the handler, never `tokio::spawn`ed: Lambda
//! freezes spawned work after the response. And the loader must never call
//! [`PortalSources::get`] on its own cell — that deadlocks.
//!
//! [`Loaded`] carries the OAuth secret, so it does not derive `Debug` and must
//! never be logged with `{:?}`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::OnceCell;

use crate::config::{AppConfig, PortalLoadError};
use crate::portal::auth::secret::OauthSecret;
use crate::portal::eligibility::EligibilitySettings;
use crate::portal::keys::gateway::Gateway;

/// The whole load's ceiling, the five reads concurrent inside it.
///
/// Each read is bounded at 2 s by the extension client, so the budget leaves
/// room for a second attempt at a read that failed fast. Four seconds keeps a load plus the
/// slowest route after it inside the 15 s invocation — pinned below for
/// `/usage` and `/key`, and in `auth::issue` for the callback.
pub(crate) const LOAD_BUDGET: Duration = Duration::from_secs(4);

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
}

impl PortalSources {
    /// Sources already in hand: the cell starts initialised and nothing is
    /// ever loaded.
    pub fn ready(loaded: Loaded) -> PortalSources {
        PortalSources {
            cell: Arc::new(OnceCell::new_with(Some(loaded))),
            loader: None,
        }
    }

    /// The Lambda's loader: the five reads from the environment.
    pub(crate) fn from_env() -> PortalSources {
        PortalSources::with_loader(Arc::new(|| Box::pin(crate::config::load_portal_sources())))
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
        }
    }

    /// The sources, loading them if this is the first ask (or every earlier
    /// ask failed). `None` means this request answers as unavailable; the
    /// failure is logged here, once per failed load, and the next call
    /// retries.
    pub async fn get(&self) -> Option<&Loaded> {
        if let Some(loaded) = self.cell.get() {
            return Some(loaded);
        }
        // A ready cell is always initialised, so only a lazy one gets here.
        let loader = self.loader.as_ref()?;
        let attempt = self.cell.get_or_try_init(|| loader());
        let err = match tokio::time::timeout(LOAD_BUDGET, attempt).await {
            Ok(Ok(loaded)) => return Some(loaded),
            Ok(Err(err)) => err,
            Err(_) => PortalLoadError::TimedOut(LOAD_BUDGET),
        };
        // The alarm's string: `prices-${env}-api-handler-portal-load-failed`
        // matches this literal's prefix, pinned by
        // `tools/scripts/portal-load-failed-filter-guard.test.mjs`.
        tracing::error!(
            error = %err,
            "portal sources failed to load; this request answers as unavailable and the next \
             portal request retries; /v1 is unaffected"
        );
        None
    }
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
    async fn a_failure_is_not_cached_and_a_success_is() {
        let (loader, calls) = scripted(&[Step::Fail, Step::Succeed], Duration::ZERO);
        let sources = PortalSources::lazy(loader);
        assert!(sources.get().await.is_none());
        assert!(sources.get().await.is_some());
        assert!(sources.get().await.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_load_past_the_budget_gives_up_and_the_next_ask_loads_again() {
        let (loader, calls) = scripted(&[Step::Hang, Step::Succeed], Duration::ZERO);
        let sources = PortalSources::lazy(loader);
        let started = tokio::time::Instant::now();
        assert!(sources.get().await.is_none());
        let waited = started.elapsed();
        assert!(
            waited >= LOAD_BUDGET && waited < LOAD_BUDGET + Duration::from_millis(50),
            "{waited:?}"
        );
        assert!(sources.get().await.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn ready_sources_answer_without_a_loader() {
        let sources = PortalSources::ready(Loaded::default());
        assert!(sources.loader.is_none());
        assert!(sources.get().await.is_some());
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
