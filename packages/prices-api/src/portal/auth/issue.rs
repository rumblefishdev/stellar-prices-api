//! Completing an `action=issue` round-trip (task 0189).
//!
//! The callback arrives here holding a **fresh** Discord token — the
//! eligibility proof ADR 0010 §8 requires. This module asks Discord, right
//! now, whether the token's owner is a member of the Stellar guild and whether
//! their account is old enough, and only then hands off to the issue path
//! (`super::super::keys::issue_for`). Nothing is remembered: the next issue or
//! rework proves itself again.
//!
//! # Every outcome is a redirect, and every redirect target is a literal
//!
//! The visitor is mid-navigation (Discord sent them here), so the answer is a
//! `303` back to the portal. **Five of the seven are verdicts** — the outcomes
//! of a completed eligibility check, and the only ones this module produces:
//!
//! | query | meaning |
//! | --- | --- |
//! | `?issue=ok` | the key exists and is on the plan — the page reveals it |
//! | `?issue=not_member` | Discord confirmed no such membership (its own 10007/10004), or the member has not cleared screening |
//! | `?issue=too_young&wait_secs=N` | account below the threshold; `N` is the wait, so the page's copy follows the operator's setting |
//! | `?issue=unknown` | membership could **not** be verified (throttle, outage, absent `pending`, unreadable parameter) — refused without accusation |
//! | `?issue=failed` | eligibility passed but the control plane would not produce a key |
//!
//! `unknown` and `failed` are separate on purpose: one says "Discord could not
//! vouch for you, try shortly", the other says "you are fine, our key service
//! was not". Collapsing them would render an AWS incident as a doubt about the
//! visitor's membership.
//!
//! The other two — [`ISSUE_CANCELLED_QUERY`] and [`ISSUE_DENIED_QUERY`] — are
//! **not** verdicts and are not produced here. They belong to the callback,
//! which reaches them when the round-trip ends at Discord before any check
//! runs; they live in this module so that every `?issue=` literal is declared
//! in one place. See their own documentation for why they are two and not one.
//!
//! The **key value never rides in a `Location`** — after `?issue=ok` the page
//! calls the reveal route, which is read-only and session-authorized.
//!
//! # The session is refreshed on every outcome past identity
//!
//! Identity was just proven, so even a refused visitor leaves signed in — a
//! non-member still legitimately holds reveal and usage (the epic's non-goal:
//! leaving the guild never forfeits an existing key). If a session for a
//! *different* account was present, the fresh identity simply replaces it —
//! the same rule the sign-in arm applies, so the key issued and the session
//! shown can never disagree about who the visitor is.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::response::Response;

use super::super::eligibility::{self, Eligibility, EligibilitySettings};
use super::super::keys::{self, IssueOutcome};
use super::super::usage::UsageCache;
use super::discord::{self, AccessToken, MemberLookup};
use super::secret::OauthSecret;
use super::session::{self, Session};
use super::state_token;
use super::{AuthState, PORTAL_HOME, cookies, redirect, refuse_discord};
use crate::portal::keys::gateway::Gateway;

/// See the module table. Literals, like `?signin=…` — the dynamic one is
/// [`too_young_query`], whose only variable part is a `u64` rendered in
/// decimal, so no request-derived byte can reach a `Location` header.
pub(super) const ISSUE_OK_QUERY: &str = "?issue=ok";
pub(super) const ISSUE_NOT_MEMBER_QUERY: &str = "?issue=not_member";
pub(super) const ISSUE_UNKNOWN_QUERY: &str = "?issue=unknown";
pub(super) const ISSUE_FAILED_QUERY: &str = "?issue=failed";

/// The round-trip ended at Discord, before any check could run.
///
/// These two are **not** a sixth and seventh verdict. The five above are the
/// outcomes of a *completed* eligibility check; these are what happens when
/// the visitor never reaches one, and sign-in has had exactly this pair since
/// task 0186 (`?signin=cancelled` / `?signin=failed`). Issue had neither, so
/// its callback borrowed sign-in's — and those banners render only in the
/// signed-out branch, which an issue round-trip has by definition left. A
/// visitor who pressed "Get my API key" and changed their mind at Discord
/// came back to an unchanged dashboard with nothing said at all.
///
/// Kept apart from each other for the reason `failed` and `unknown` are kept
/// apart: "you changed your mind" and "our registration is wrong" are not the
/// same event and do not belong to the same person.
pub(super) const ISSUE_CANCELLED_QUERY: &str = "?issue=cancelled";
pub(super) const ISSUE_DENIED_QUERY: &str = "?issue=denied";

/// The one parameterised landing state: how long until the account clears the
/// threshold. Digits only, by type.
pub(super) fn too_young_query(wait_secs: u64) -> String {
    format!("?issue=too_young&wait_secs={wait_secs}")
}

/// Mirrors `keys`' `KEYS_UNCONFIGURED` — the same deployment fault ("portal
/// open, key issuance not wired") reported under the same code whichever door
/// it is noticed at. `/auth/login?action=issue` refuses with this instead of
/// starting a round-trip that cannot end in a key.
pub(super) const KEYS_UNCONFIGURED: &str = "keys_unconfigured";

/// The whole callback's I/O budget, measured from the moment the request
/// arrived — **not** from the moment this module takes over.
///
/// `keys::RECONCILE_DEADLINE` is 10s and was sized for 0187, where the
/// reconciliation was essentially the entire request: "10s leaves the handler
/// ~5s of the function's budget". This path puts four more network calls in
/// front of it — the token exchange (5s), two Parameters and Secrets reads
/// (2s each) and two Discord reads (5s each) — so reusing that constant
/// unchanged would let the worst case reach ~29s against
/// `apiHandler.timeoutSeconds` of **15**. Lambda would kill the invocation
/// with no response at all: the visitor gets API Gateway's bare `502` instead
/// of the designed `?issue=failed` redirect, an `Errors` datapoint is
/// recorded, and a key may exist that was never attached — precisely the
/// failure mode 0186's F7 and 0187's R1 both removed.
///
/// 12s of a 15s function leaves ~3s to serialize a redirect and for the
/// runtime to send it. The reconciler gets whatever survives; see
/// [`RECONCILE_FLOOR`] for what happens when that is not enough.
const ISSUE_BUDGET: Duration = Duration::from_secs(12);

/// The least time worth starting a reconciliation with.
///
/// A reconciliation needs at least a list, a create and an attach. Beginning
/// one with less than this cannot end in a key, and starting control-plane
/// writes that the invocation will be killed part-way through is how an
/// unattached orphan is made. Below this the path lands on `?issue=failed` —
/// which is the honest answer: eligibility passed, our key service did not
/// have time.
const RECONCILE_FLOOR: Duration = Duration::from_secs(2);

/// Everything the issue arm needs beyond what sign-in already carries.
///
/// All optional, like `AuthState::oauth` and `KeysState::gateway`, and for the
/// same reason: the api-handler boots with the portal closed and nothing
/// provisioned. `config::load_portal_eligibility` fails the cold start when
/// the portal is *open* with these missing, so a `None` here in production
/// means the portal is closed and the gate answers before any handler does.
#[derive(Clone)]
pub struct IssueDeps {
    pub(super) gateway: Option<Arc<Gateway>>,
    pub(super) usage_cache: Option<UsageCache>,
    pub(super) settings: Option<Arc<EligibilitySettings>>,
    /// The same wall-clock ceiling 0187's handler put on the reconciliation.
    pub(super) deadline: Duration,
}

impl Default for IssueDeps {
    fn default() -> Self {
        Self {
            gateway: None,
            usage_cache: None,
            settings: None,
            deadline: keys::RECONCILE_DEADLINE,
        }
    }
}

impl IssueDeps {
    pub fn new(
        gateway: Option<Gateway>,
        usage_cache: Option<UsageCache>,
        settings: Option<EligibilitySettings>,
    ) -> Self {
        Self {
            gateway: gateway.map(Arc::new),
            usage_cache,
            settings: settings.map(Arc::new),
            deadline: keys::RECONCILE_DEADLINE,
        }
    }

    /// Shorten the deadline for tests — compiled out of the Lambda for the
    /// reason `KeysState::with_deadline` is: a deployed build must contain no
    /// way to set this to something that lets a slow control plane outlive
    /// the invocation.
    #[cfg(not(feature = "lambda"))]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// Whether an issue round-trip could complete on this deployment.
    pub(super) fn is_wired(&self) -> bool {
        self.gateway.is_some() && self.settings.is_some()
    }
}

/// Finish an `action=issue` callback: check, then issue, then land.
///
/// Runs after `state` verification, the code exchange and the granted-scope
/// check — `token` is the fresh, scope-verified token those produced. The
/// order below is load-bearing: the membership call borrows the token, the
/// identity read consumes it, and everything after holds no token at all.
///
/// `started` is stamped when the **request** arrived, not when this function
/// did: the exchange that ran before it spent from the same Lambda budget, so
/// a clock started here would not see the largest single call on the path.
/// See [`ISSUE_BUDGET`].
pub(super) async fn complete_issue(
    state: &AuthState,
    oauth: &OauthSecret,
    token: AccessToken,
    drop_pending: String,
    started: Instant,
) -> Response {
    // Resolve the two operator knobs first — per action, so an SSM change is
    // honoured without a redeploy. Failure is `unknown`, not a 5xx: the
    // visitor is mid-navigation, the fault is ours, and "could not verify"
    // is the honest refusal that does not accuse them of anything.
    let verified = match state.issue.settings.as_deref() {
        None => {
            // `login` refuses `action=issue` on an unwired deployment, so
            // arriving here means the deployment changed under an in-flight
            // round-trip. Log it as ours; tell the visitor to retry.
            tracing::error!("an issue callback arrived with no eligibility settings wired");
            None
        }
        Some(settings) => match (
            settings.guild_id().await,
            settings.min_account_age_minutes().await,
        ) {
            (Ok(guild_id), Ok(min_age)) => Some((guild_id, min_age)),
            (guild, age) => {
                for error in [guild.err(), age.err()].into_iter().flatten() {
                    tracing::error!(
                        error = %error,
                        "eligibility parameters could not be read; refusing without accusation"
                    );
                }
                None
            }
        },
    };

    // The membership call BORROWS the token; the identity read then consumes
    // it. Asked in this order so that one round-trip serves both questions —
    // Discord does not re-prompt for consent on repeat authorisation, so this
    // whole detour cost the visitor a redirect, not a login.
    let member = match &verified {
        Some((guild_id, _)) => {
            let looked_up =
                discord::guild_member(&state.http, &state.endpoints, &token, guild_id).await;
            if let MemberLookup::NotMember { code: 10_004 } = looked_up {
                // "Unknown Guild" is far more likely to be OUR mis-seeded
                // parameter than the visitor's standing. Still rendered as
                // the spec says (0180 item 1 will settle the real shape),
                // but loud in CloudWatch so a config fault is visible on the
                // first refusal rather than after a support thread.
                tracing::warn!(
                    guild_id = %guild_id,
                    "membership check answered Unknown Guild (10004) — \
                     is the discord-guild-id parameter right?"
                );
            }
            Some(looked_up)
        }
        None => None,
    };

    let user = match discord::current_user(&state.http, &state.endpoints, token).await {
        Ok(user) => user,
        // No identity, no session, no verdict — the same 502 the sign-in arm
        // answers when Discord will not say who the visitor is.
        Err(error) => return refuse_discord("identity read", error, drop_pending),
    };

    // Identity is proven: from here on every outcome carries a fresh session,
    // replacing whatever was there. See the module docs.
    let session = Session::issue(&user.id, &user.username, state_token::now_secs());
    let session_cookie = cookies::set(
        cookies::SESSION_COOKIE,
        &session.encode(&oauth.signing_key),
        cookies::SESSION_PATH,
        session::SESSION_TTL_SECS,
    );
    let land = |query: &str| {
        redirect(
            &format!("{PORTAL_HOME}{query}"),
            vec![drop_pending.clone(), session_cookie.clone()],
        )
    };

    let verdict = match (&verified, &member) {
        (Some((_, min_age)), Some(member)) => {
            eligibility::decide(member, &user.id, *min_age, eligibility::now_ms())
        }
        // Parameters unreadable — the membership question was never asked.
        _ => Eligibility::Unknown,
    };

    match verdict {
        Eligibility::NotMember => {
            tracing::info!(outcome = "not_member", "portal issue refused");
            land(ISSUE_NOT_MEMBER_QUERY)
        }
        Eligibility::TooYoung { wait_secs } => {
            tracing::info!(outcome = "too_young", wait_secs, "portal issue refused");
            land(&too_young_query(wait_secs))
        }
        Eligibility::Unknown => {
            // The load-bearing warns (which check could not answer, and why)
            // fired where the answer was known; this line is the one that
            // says what the visitor was told.
            tracing::info!(
                outcome = "unknown",
                "portal issue refused without accusation"
            );
            land(ISSUE_UNKNOWN_QUERY)
        }
        Eligibility::Eligible => {
            let Some(gateway) = state.issue.gateway.as_deref() else {
                tracing::error!("an eligible issue callback arrived with no control plane wired");
                return land(ISSUE_FAILED_QUERY);
            };

            // What is left of the invocation, not a constant. Everything above
            // — the exchange, both parameter reads, both Discord reads — has
            // already been paid for out of the same budget.
            let remaining = ISSUE_BUDGET.saturating_sub(started.elapsed());
            if remaining < RECONCILE_FLOOR {
                tracing::error!(
                    remaining_ms = remaining.as_millis() as u64,
                    "eligibility passed but the invocation's budget is spent; \
                     refusing to start a reconciliation that cannot finish"
                );
                return land(ISSUE_FAILED_QUERY);
            }
            // `min`, so the configured deadline stays an upper bound and the
            // `with_deadline` test seam keeps working.
            let deadline = remaining.min(state.issue.deadline);

            match keys::issue_for(gateway, &user.id, deadline).await {
                IssueOutcome::Issued => {
                    // A key now exists, so a cached "no key" on the usage
                    // route is false — same eviction the reveal performs,
                    // for the page this redirect is about to land on.
                    if let Some(cache) = &state.issue.usage_cache {
                        cache.invalidate_no_key(&user.id);
                    }
                    land(ISSUE_OK_QUERY)
                }
                IssueOutcome::Failed => land(ISSUE_FAILED_QUERY),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every landing state is a distinct literal under the portal home,
    /// like `?signin=…` — extending `the_only_redirect_targets_are_the_portal
    /// _itself` to the issue flow.
    #[test]
    fn the_issue_landing_states_are_distinct_portal_literals() {
        let fixed = [
            ISSUE_OK_QUERY,
            ISSUE_NOT_MEMBER_QUERY,
            ISSUE_UNKNOWN_QUERY,
            ISSUE_FAILED_QUERY,
            // Pre-check landings. Distinct from each other and from every
            // verdict above — a cancelled press must never read as a refusal.
            ISSUE_CANCELLED_QUERY,
            ISSUE_DENIED_QUERY,
        ];
        for (i, a) in fixed.iter().enumerate() {
            assert!(a.starts_with("?issue="));
            assert!(format!("{PORTAL_HOME}{a}").starts_with("/api-tokens/?"));
            for b in &fixed[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    /// The one dynamic landing state renders a `u64` and nothing else — no
    /// request-derived byte can reach the `Location` header through it.
    #[test]
    fn the_too_young_query_is_digits_only() {
        assert_eq!(too_young_query(173), "?issue=too_young&wait_secs=173");
        let rendered = too_young_query(u64::MAX);
        let (prefix, value) = rendered.split_once("wait_secs=").unwrap();
        assert_eq!(prefix, "?issue=too_young&");
        assert!(value.bytes().all(|b| b.is_ascii_digit()));
    }

    #[test]
    fn issue_deps_default_to_unwired_with_the_production_deadline() {
        let deps = IssueDeps::default();
        assert!(!deps.is_wired());
        assert_eq!(deps.deadline, keys::RECONCILE_DEADLINE);
    }
}
