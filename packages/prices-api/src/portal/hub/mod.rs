//! The API hub's dashboard contract (task 0348).
//!
//! The hub (`rumblefishdev/sorobanscan-api-portal`) holds the developer's
//! Discord session and calls Prices server-to-server, one call per action,
//! each with a token of its own: `v1.<payload>.<mac>`, an HMAC-SHA256 under a
//! secret the two sides share through AWS Secrets Manager (the shared sign-in
//! spec's decisions 12 and 15). This module verifies that token
//! ([`token::verify`]) and parses the secret ([`secret::HubSecret`]). The
//! secret is loaded by [`crate::portal::sources::HubSource`], on the first hub
//! call that needs it and apart from the portal's own sources: a missing or
//! bad one closes the contract only, never the portal, and a failed read is
//! tried again after a short cooldown.
//!
//! # The four calls
//!
//! | route | `act` | answers |
//! | --- | --- | --- |
//! | `GET /api/hub/v1/account` | `read` | the key's state, whether a new one can be had and when, and Prices' `details`: the usage `/api/usage` reports and the free plan's rate limit `/api/config` reports |
//! | `POST /api/hub/v1/key/reveal` | `reveal` | the portal's reveal: the key and its value, or `404 no_key` / `key_revoked` |
//! | `POST /api/hub/v1/key/revoke` | `revoke` | the portal's revoke: the key off now, and when a new one can be had |
//! | `POST /api/hub/v1/key/issue` | `issue` | `issued`, `has_key`, `capped`, `too_young` or `failed` |
//!
//! Each is the portal's own service under another authorisation: the key
//! routes' `keys::key_state`, `keys::revoke_for` and `keys::issue_for`, and
//! the usage route's cache. So
//! Prices' rules hold unchanged — one live key per account
//! (`discord-<id>-key`), the month cap after a revoke, the plan a key keeps,
//! `ManagedBy=prices-portal` — and a key the hub asks for is tagged
//! `IssuedBy=hub-contract`.
//!
//! # Who may call
//!
//! The token, and nothing else: no session cookie, no `Origin`, no marker
//! header, and no CORS — no browser calls these routes (`auth`). The
//! hub proves membership and screening before it asks for an issue (its
//! `act=issue` is that proof, an amendment to ADR 0010 §8); the account's
//! age is checked here again, from the snowflake, against Prices' own
//! threshold.
//!
//! # Answers
//!
//! Prices' error envelope (`{"code","message","details"?}`) with the
//! portal's codes, and `Cache-Control: no-store` on every answer, a `405`
//! included — set once, by the router's own layer. Never in the OpenAPI
//! document: mounted by `portal::apply_with`, inside the `PORTAL_ENABLED`
//! gate.
//!
//! # Logs
//!
//! A call is named by its `jti` and `act`, with its outcome and `key_id`;
//! never the token, the key's value, the Discord id or the username.

pub(crate) mod account;
pub(crate) mod auth;
pub(crate) mod key;
pub mod secret;
pub mod token;

use std::time::Duration;

use axum::Router;
use axum::response::Response;
use axum::routing::{get, post};
use tokio::time::Instant;

use crate::common::cache_control;
use crate::portal::keys::RECONCILE_DEADLINE;
use crate::portal::sources::{LOAD_BUDGET, PortalSources};
use crate::portal::usage::UsageState;

/// The account's key and its use.
pub const ACCOUNT_PATH: &str = "/api/hub/v1/account";
/// The key itself.
pub const REVEAL_PATH: &str = "/api/hub/v1/key/reveal";
/// Revoke the key.
pub const REVOKE_PATH: &str = "/api/hub/v1/key/revoke";
/// Issue a key.
pub const ISSUE_PATH: &str = "/api/hub/v1/key/issue";

/// How long a call may take from its arrival to its answer's being ready.
///
/// The slower of the two loads in front of every call (they run together,
/// see [`auth`]) and then the control plane's own deadline: 4 s + 10 s. Each
/// route's control-plane deadline is what is left of this, never more than
/// [`RECONCILE_DEADLINE`] — so `issue`, which reads the minimum account age
/// before the control plane, takes that read out of its own deadline rather
/// than adding it to the invocation's. The second left of the 15 s
/// invocation serialises the answer; pinned below.
pub(crate) const CALL_BUDGET: Duration =
    Duration::from_secs(LOAD_BUDGET.as_secs() + RECONCILE_DEADLINE.as_secs());

/// What the four routes need, cloned per request.
///
/// `sources` is the portal's one cell — the control plane, the eligibility
/// settings and, beside them, the hub's secret. `usage` is a clone of the
/// portal's usage state: its cache is the portal's, so an account's usage is
/// one entry whichever route asks, and a revoke or issue here evicts it for
/// `/api/usage` too.
#[derive(Clone)]
pub struct HubState {
    sources: PortalSources,
    usage: UsageState,
    /// The free plan's per-key rate limit (`AppConfig::portal_rate_limit`),
    /// as `/api/config` reports it.
    rate_limit: Option<u32>,
    /// [`RECONCILE_DEADLINE`], overridable only outside the Lambda build.
    deadline: Duration,
}

impl HubState {
    pub fn new(sources: PortalSources, usage: UsageState, rate_limit: Option<u32>) -> Self {
        Self {
            sources,
            usage,
            rate_limit,
            deadline: RECONCILE_DEADLINE,
        }
    }

    /// Shorten the control plane's deadline, so a test can drive the timeout
    /// in milliseconds. **Compiled out of the Lambda**, as
    /// `KeysState::with_deadline` is: a deployed build must contain no way
    /// to set it to something that outlives the invocation.
    #[cfg(not(feature = "lambda"))]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// The control plane's deadline for a call that arrived at `started`:
    /// what is left of [`CALL_BUDGET`], at most the configured deadline.
    fn deadline_from(&self, started: Instant) -> Duration {
        self.deadline
            .min(CALL_BUDGET.saturating_sub(started.elapsed()))
    }
}

/// The four routes, as a `Router` carrying its own state.
///
/// `no-store` is a layer here rather than each handler's statement, because
/// it must also cover what no handler writes: the `405` for a wrong verb.
/// Every answer of this contract is about one account — a cached one would
/// serve it to another.
pub fn routes(state: HubState) -> Router {
    Router::new()
        .route(ACCOUNT_PATH, get(account::account))
        .route(REVEAL_PATH, post(key::reveal))
        .route(REVOKE_PATH, post(key::revoke))
        .route(ISSUE_PATH, post(key::issue))
        .layer(axum::middleware::map_response(no_store))
        .with_state(state)
}

async fn no_store(mut response: Response) -> Response {
    cache_control::attach(&mut response, cache_control::NO_STORE);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal::sources::HUB_SECRET_BUDGET;

    /// Every route is under the portal's prefix, so the gate covers it.
    #[test]
    fn the_routes_live_under_the_gated_prefix() {
        for path in [ACCOUNT_PATH, REVEAL_PATH, REVOKE_PATH, ISSUE_PATH] {
            assert!(path.starts_with(crate::portal::PORTAL_API_PREFIX), "{path}");
            assert!(path.starts_with("/api/hub/v1/"), "{path}");
        }
    }

    /// The whole of a call fits the invocation: the two loads together,
    /// then the control plane — and for `issue`, the age threshold's read
    /// too, taken out of the control plane's share. The 15 is
    /// `apiHandler.timeoutSeconds` in `infra/envs/production.json`.
    #[test]
    fn a_call_fits_the_invocation() {
        const LAMBDA_TIMEOUT: Duration = Duration::from_secs(15);
        const ANSWER_MARGIN: Duration = Duration::from_secs(1);
        let loads = LOAD_BUDGET.max(HUB_SECRET_BUDGET);
        assert!(loads + RECONCILE_DEADLINE <= CALL_BUDGET);
        assert!(CALL_BUDGET + ANSWER_MARGIN <= LAMBDA_TIMEOUT);
        // `issue`: after the loads and the threshold's read, a create still
        // has room to start (`keys::CREATE_FLOOR`).
        let left = CALL_BUDGET - loads - crate::portal::eligibility::PARAMETER_TIMEOUT;
        assert!(left >= crate::portal::keys::CREATE_FLOOR);
    }

    #[tokio::test(start_paused = true)]
    async fn the_deadline_is_what_is_left_of_the_budget() {
        let state = HubState::new(
            PortalSources::ready(Default::default()),
            UsageState::new(None, None),
            None,
        );
        let started = Instant::now();
        assert_eq!(state.deadline_from(started), RECONCILE_DEADLINE);
        tokio::time::advance(LOAD_BUDGET).await;
        assert_eq!(state.deadline_from(started), RECONCILE_DEADLINE);
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(
            state.deadline_from(started),
            CALL_BUDGET - LOAD_BUDGET - Duration::from_secs(2)
        );
        tokio::time::advance(CALL_BUDGET).await;
        assert_eq!(state.deadline_from(started), Duration::ZERO);
        let short = state.with_deadline(Duration::from_millis(50));
        assert_eq!(
            short.deadline_from(Instant::now()),
            Duration::from_millis(50)
        );
    }
}
