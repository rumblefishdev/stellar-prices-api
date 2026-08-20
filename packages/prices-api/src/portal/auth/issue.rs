//! Completing an `action=issue` round-trip (task 0189).
//!
//! The callback arrives here holding a **fresh** Discord token — the
//! eligibility proof ADR 0010 §8 requires. This module asks Discord, right
//! now, whether the token's owner is a member of the Stellar guild and whether
//! their account is old enough, and only then hands off to the issue path
//! (`super::super::keys::issue_for`). Nothing is remembered: the next issue or
//! rework proves itself again.
//!
//! # Every outcome is a redirect, and the outcomes are five literals
//!
//! The visitor is mid-navigation (Discord sent them here), so the answer is a
//! `303` back to the portal with one of:
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
use std::time::Duration;

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
pub(super) async fn complete_issue(
    state: &AuthState,
    oauth: &OauthSecret,
    token: AccessToken,
    drop_pending: String,
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
            match keys::issue_for(gateway, &user.id, state.issue.deadline).await {
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

    /// The five landing states are distinct literals under the portal home,
    /// like `?signin=…` — extending `the_only_redirect_targets_are_the_portal
    /// _itself` to the issue flow.
    #[test]
    fn the_issue_landing_states_are_distinct_portal_literals() {
        let fixed = [
            ISSUE_OK_QUERY,
            ISSUE_NOT_MEMBER_QUERY,
            ISSUE_UNKNOWN_QUERY,
            ISSUE_FAILED_QUERY,
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
