//! Completing an `action=rework` round-trip (task 0191).
//!
//! The shape of `super::issue`, with two differences that are the whole of the
//! task: the check is **membership only** — an account old enough once is old
//! enough forever, so age is never re-proved on a rework — and what happens
//! after the check is a **swap** rather than a reconcile: the old key is
//! deleted and a new one created in one operation, capped at once per quota
//! period (`keys::cap`).
//!
//! # Why a round-trip, and not a `POST` with the session cookie
//!
//! The eligibility table (`eligibility`, ADR 0010 §8) says a rework re-proves
//! membership, and a proof is a fresh Discord token, which only a top-level
//! navigation can fetch. So "rework is unreachable with a session cookie
//! alone" is structural here exactly as "issue is unreachable with a session
//! cookie alone" is in `issue`: the only route a session cookie reaches is
//! `POST /key/rework`, the read-only pre-check, which writes nothing.
//!
//! # Every outcome is a redirect, every target a literal
//!
//! | query | meaning |
//! | --- | --- |
//! | `?rework=ok` | the old key is gone, the new one is on the plan — the page reveals it |
//! | `?rework=capped&next_eligible_at=YYYY-MM-DD` | the current key was created inside this period; nothing moved. The date is ours (`period`) |
//! | `?rework=no_key` | there was nothing to replace — the page offers the issue round-trip |
//! | `?rework=not_member` | Discord confirmed no such membership, or screening not cleared |
//! | `?rework=unknown` | membership could **not** be verified — refused without accusation |
//! | `?rework=failed` | our key service could not complete the swap. Never a statement about the visitor |
//! | `?rework=cancelled` / `?rework=denied` | the round-trip ended at Discord before any check ran — `issue`'s pair, for the same reasons |
//!
//! `capped` is a *verdict*, not a failure, and it is the one with a date: the
//! wait is weeks, and "try again later" would be a lie by omission. The date
//! travels as `YYYY-MM-DD` — digits and dashes by construction (`keys::cap`)
//! — so no request-derived byte reaches a `Location` header through it.
//!
//! # Precedence: membership, then the cap
//!
//! A departed member is told to rejoin before being told to wait, because the
//! membership answer is already in hand (the token was just exchanged) and
//! the cap needs a control-plane listing. Both are refused; the order decides
//! which fixable thing the visitor hears about first.

use std::time::Instant;

use axum::response::Response;

use super::super::eligibility::{self, Membership};
use super::super::keys::{self, ReworkOutcome};
use super::discord::{self, AccessToken, MemberLookup};
use super::issue::{ISSUE_BUDGET, RECONCILE_FLOOR};
use super::secret::OauthSecret;
use super::session::{self, Session};
use super::state_token;
use super::{AuthState, PORTAL_HOME, cookies, redirect};

/// See the module table. Literals, like `?issue=…`; the dynamic one is
/// [`capped_query`], whose only variable part is a `YYYY-MM-DD` built from a
/// `NaiveDate`.
pub(super) const REWORK_OK_QUERY: &str = "?rework=ok";
pub(super) const REWORK_NO_KEY_QUERY: &str = "?rework=no_key";
pub(super) const REWORK_NOT_MEMBER_QUERY: &str = "?rework=not_member";
pub(super) const REWORK_UNKNOWN_QUERY: &str = "?rework=unknown";
pub(super) const REWORK_FAILED_QUERY: &str = "?rework=failed";
pub(super) const REWORK_CANCELLED_QUERY: &str = "?rework=cancelled";
pub(super) const REWORK_DENIED_QUERY: &str = "?rework=denied";

/// The one parameterised landing state: when the next rework becomes
/// available. `next_eligible_date` is `keys::cap`'s `YYYY-MM-DD`.
pub(super) fn capped_query(next_eligible_date: &str) -> String {
    // Defence in depth on top of the type: the value is built from a
    // `NaiveDate` and cannot contain anything else, but this is a `Location`
    // header, so the shape is asserted where it is used.
    debug_assert!(
        next_eligible_date
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'-')
    );
    format!("?rework=capped&next_eligible_at={next_eligible_date}")
}

/// Refuse to *start* a rework round-trip on a deployment that cannot finish
/// one — `issue::refuse_issue_start`'s twin, landing on `?rework=failed`.
pub(super) fn refuse_rework_start(oauth: bool, gateway: bool, settings: bool) -> Response {
    tracing::error!(
        oauth,
        gateway,
        settings,
        "a rework round-trip was started on a deployment that cannot complete one"
    );
    redirect(&format!("{PORTAL_HOME}{REWORK_FAILED_QUERY}"), Vec::new())
}

/// Land a Discord failure that happened on an `action=rework` round-trip —
/// `issue::refuse_issue_discord`'s twin, with the same fault split:
/// `UnexpectedScope` is our registration drifting (`denied`), anything else
/// is Discord not answering (`unknown`).
pub(super) fn refuse_rework_discord(
    stage: &str,
    error: discord::DiscordError,
    drop_pending: String,
) -> Response {
    let query = match error {
        discord::DiscordError::UnexpectedScope { .. } => REWORK_DENIED_QUERY,
        _ => REWORK_UNKNOWN_QUERY,
    };
    tracing::warn!(
        stage,
        error = %error,
        landing = query,
        "portal rework round-trip could not complete with Discord"
    );
    redirect(&format!("{PORTAL_HOME}{query}"), vec![drop_pending])
}

/// Finish an `action=rework` callback: check membership, then swap, then
/// land.
///
/// Runs after `state` verification, the code exchange and the granted-scope
/// check — `token` is the fresh, scope-verified token. The same ordering as
/// `issue::complete_issue`, for the same reasons: the membership call
/// **borrows** the token, the identity read **consumes** it, and the session
/// is refreshed on every outcome past identity.
///
/// `started` is the request's arrival, not this function's — the swap spends
/// the same Lambda budget as the exchange before it (`issue::ISSUE_BUDGET`).
pub(super) async fn complete_rework(
    state: &AuthState,
    oauth: &OauthSecret,
    token: AccessToken,
    drop_pending: String,
    started: Instant,
) -> Response {
    // Only the guild id is needed — the age threshold is deliberately not
    // read. Resolved per action, so an operator's `put-parameter` is
    // honoured without a redeploy; unreadable is `unknown`, never a 5xx.
    let guild_id = match state.issue.settings.as_deref() {
        None => {
            tracing::error!("a rework callback arrived with no eligibility settings wired");
            None
        }
        Some(settings) => match settings.guild_id().await {
            Ok(guild_id) => Some(guild_id),
            Err(error) => {
                tracing::error!(
                    error = %error,
                    "the guild id could not be read; refusing the rework without accusation"
                );
                None
            }
        },
    };

    let member = match &guild_id {
        Some(guild_id) => {
            let looked_up =
                discord::guild_member(&state.http, &state.endpoints, &token, guild_id).await;
            if let MemberLookup::NotMember { code: 10_004 } = looked_up {
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
        Err(error) => return refuse_rework_discord("identity read", error, drop_pending),
    };

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

    // Membership only. `eligibility::membership` is the same `pending` table
    // `decide` uses, minus the age check — see that module's per-action table.
    let membership = match &member {
        Some(member) => eligibility::membership(member),
        None => Membership::Unknown,
    };
    match membership {
        Membership::Member => {}
        Membership::NotMember => {
            tracing::info!(outcome = "not_member", "portal rework refused");
            return land(REWORK_NOT_MEMBER_QUERY);
        }
        Membership::Unknown => {
            tracing::info!(
                outcome = "unknown",
                "portal rework refused without accusation"
            );
            return land(REWORK_UNKNOWN_QUERY);
        }
    }

    let Some(gateway) = state.issue.gateway.as_deref() else {
        tracing::error!("a member's rework callback arrived with no control plane wired");
        return land(REWORK_FAILED_QUERY);
    };

    // The swap is list + create + attach + one delete per old key + read,
    // against what is left of the invocation — same arithmetic as the issue.
    let remaining = ISSUE_BUDGET.saturating_sub(started.elapsed());
    if remaining < RECONCILE_FLOOR {
        tracing::error!(
            remaining_ms = remaining.as_millis() as u64,
            "membership passed but the invocation's budget is spent; \
             refusing to start a swap that cannot finish"
        );
        return land(REWORK_FAILED_QUERY);
    }
    let deadline = remaining.min(state.issue.deadline);

    match keys::rework_for(gateway, &user.id, deadline).await {
        ReworkOutcome::Replaced => {
            // The cached usage (if any) describes a key that no longer
            // exists; the new key starts from a clean counter.
            if let Some(cache) = &state.issue.usage_cache {
                cache.invalidate(&user.id);
            }
            land(REWORK_OK_QUERY)
        }
        ReworkOutcome::NoKey => land(REWORK_NO_KEY_QUERY),
        ReworkOutcome::Capped { next_eligible_date } => land(&capped_query(&next_eligible_date)),
        ReworkOutcome::Failed => land(REWORK_FAILED_QUERY),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every landing state is a distinct literal under the portal home — the
    /// rework half of `the_issue_landing_states_are_distinct_portal_literals`.
    #[test]
    fn the_rework_landing_states_are_distinct_portal_literals() {
        let fixed = [
            REWORK_OK_QUERY,
            REWORK_NO_KEY_QUERY,
            REWORK_NOT_MEMBER_QUERY,
            REWORK_UNKNOWN_QUERY,
            REWORK_FAILED_QUERY,
            REWORK_CANCELLED_QUERY,
            REWORK_DENIED_QUERY,
        ];
        for (i, a) in fixed.iter().enumerate() {
            assert!(a.starts_with("?rework="));
            assert!(format!("{PORTAL_HOME}{a}").starts_with("/api-tokens/?"));
            for b in &fixed[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    /// The one dynamic landing state renders a date and nothing else.
    #[test]
    fn the_capped_query_carries_a_bare_date() {
        assert_eq!(
            capped_query("2026-09-01"),
            "?rework=capped&next_eligible_at=2026-09-01"
        );
    }

    /// `?rework=…` and `?issue=…` never collide: the page reads them as two
    /// separate one-shot params, and a rework landing must never render as
    /// an issue verdict.
    #[test]
    fn rework_and_issue_landings_live_under_different_params() {
        assert!(REWORK_OK_QUERY.starts_with("?rework="));
        assert!(super::super::issue::ISSUE_OK_QUERY.starts_with("?issue="));
    }
}
