//! `POST /api/hub/v1/key/{reveal,revoke,issue}` (task 0348): the portal's
//! key services, authorised by the hub's token.
//!
//! | route | answers |
//! | --- | --- |
//! | `reveal` | `{"key_id","name","value","created_at","last_updated_at"}`, or `404 no_key` / `404 key_revoked` with `details` — the portal's reveal |
//! | `revoke` | `{"revoked": true, "next_eligible_at", "revoked_at", "partial"?}`, or `404 no_key` — the portal's revoke |
//! | `issue` | `{"outcome": "issued" \| "has_key" \| "capped" \| "too_young" \| "failed", "next_eligible_at"?}` |
//!
//! A failing or slow control plane on `reveal` and `revoke` is the portal's
//! `502` / `503 key_unavailable` ([`keys::key_unavailable`]). `issue` always
//! answers `200` with an outcome, as the portal's issue always lands on a
//! page: `failed` is "our key service, not you".
//!
//! # Issue
//!
//! The hub proved membership and screening before it signed an `issue`
//! token. What is checked here, first and without a control-plane call, is
//! the account's age, from the snowflake, against Prices' own minimum (the
//! `min-account-age-minutes` parameter the portal's issue reads): a backstop,
//! so a key is never issued below Prices' threshold whatever the hub's is.
//! `too_young` names the instant the account clears it as
//! `next_eligible_at`. Then [`keys::issue_for`], the portal's reconciler,
//! with the `IssuedBy=hub-contract` tag: `issued` for a key it created,
//! `has_key` for one it found (one live key per account), `capped` with the
//! date for an account that revoked this period.
//!
//! # The usage cache
//!
//! A revoke, or an issue that leaves a key, evicts the account's cached usage
//! in this execution environment ([`UsageCache::invalidate`]): the next
//! `account` and the portal's `/api/usage` read the key as it now is, not a
//! "no key" or the revoked key's figures for up to a minute. A reveal evicts
//! a cached "no key", as the portal's reveal does.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use tokio::time::Instant;

use super::HubState;
use super::auth::{self, authorize};
use super::token::Act;
use crate::common::errors;
use crate::portal::eligibility::{self, AccountAge};
use crate::portal::keys::gateway::ISSUED_BY_HUB;
use crate::portal::keys::{
    self, IssueOutcome, KeyError, KeyResponse, KeyState, RevokeOutcome, no_key_response, rfc3339,
};

/// `POST key/reveal`.
pub(crate) async fn reveal(State(state): State<HubState>, headers: HeaderMap) -> Response {
    let started = Instant::now();
    let authorized = match authorize(&state.sources, &headers, Act::Reveal).await {
        Ok(authorized) => authorized,
        Err(refused) => return refused,
    };
    let caller = &authorized.caller;
    let (jti, act) = (caller.jti.as_str(), caller.act.as_str());
    let Some(gateway) = authorized.loaded.gateway.as_deref() else {
        return keys::unconfigured();
    };

    match keys::key_state(gateway, &caller.discord_id, state.deadline_from(started)).await {
        Err(KeyError::NotASnowflake) => {
            tracing::warn!(
                jti,
                act,
                "hub call carried an id that names no key; refusing"
            );
            auth::invalid_token()
        }
        Err(error) => {
            // `key_state` logged the failure; this is what the hub was told.
            tracing::warn!(jti, act, outcome = "unavailable", "hub reveal failed");
            keys::key_unavailable(error)
        }
        Ok(KeyState::Active { record, value }) => {
            tracing::info!(jti, act, key_id = %record.id, outcome = "revealed", "hub revealed an API key");
            // This answer proves a key exists: a cached "no key" is false.
            state
                .usage
                .cache_handle()
                .invalidate_no_key(&caller.discord_id);
            Json(KeyResponse::new(record, value)).into_response()
        }
        Ok(KeyState::Revoked {
            revoked_at,
            next_eligible_at: Some(next_eligible_at),
        }) => {
            tracing::info!(
                jti,
                act,
                outcome = "key_revoked",
                "hub reveal found the key revoked"
            );
            keys::key_revoked_response(revoked_at, &next_eligible_at)
        }
        // Nothing, or revoked in an earlier period: no usable key, as the
        // portal's reveal says.
        Ok(KeyState::Revoked { .. } | KeyState::None) => {
            tracing::info!(jti, act, outcome = "no_key", "hub reveal found no key");
            no_key_response()
        }
    }
}

/// `POST key/revoke`.
pub(crate) async fn revoke(State(state): State<HubState>, headers: HeaderMap) -> Response {
    let started = Instant::now();
    let authorized = match authorize(&state.sources, &headers, Act::Revoke).await {
        Ok(authorized) => authorized,
        Err(refused) => return refused,
    };
    let caller = &authorized.caller;
    let (jti, act) = (caller.jti.as_str(), caller.act.as_str());
    let Some(gateway) = authorized.loaded.gateway.as_deref() else {
        return keys::unconfigured();
    };

    match keys::revoke_for(gateway, &caller.discord_id, state.deadline_from(started)).await {
        Err(KeyError::NotASnowflake) => {
            tracing::warn!(
                jti,
                act,
                "hub call carried an id that names no key; refusing"
            );
            auth::invalid_token()
        }
        Err(error) => {
            // `revoke_for` logged the failure, with the key ids it touched.
            tracing::warn!(jti, act, outcome = "unavailable", "hub revoke failed");
            keys::key_unavailable(error)
        }
        Ok(RevokeOutcome::Revoked { answer, key_id }) => {
            // Whatever is cached describes a key that is now off.
            state.usage.cache_handle().invalidate(&caller.discord_id);
            tracing::info!(
                jti,
                act,
                key_id = %key_id,
                outcome = "revoked",
                "hub revoked an API key"
            );
            Json(answer).into_response()
        }
        Ok(RevokeOutcome::NoKey) => {
            tracing::info!(jti, act, outcome = "no_key", "hub revoke found no key");
            (
                StatusCode::NOT_FOUND,
                Json(errors::ErrorEnvelope {
                    code: keys::NO_KEY,
                    message: "this account has no API key to revoke".into(),
                    details: None,
                }),
            )
                .into_response()
        }
    }
}

/// What `key/issue` answers: always `200`, an outcome the hub's tab words.
#[derive(Serialize)]
struct IssueAnswer {
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_eligible_at: Option<String>,
}

fn outcome(outcome: &'static str, next_eligible_at: Option<String>) -> Response {
    Json(IssueAnswer {
        outcome,
        next_eligible_at,
    })
    .into_response()
}

/// `POST key/issue`.
pub(crate) async fn issue(State(state): State<HubState>, headers: HeaderMap) -> Response {
    let started = Instant::now();
    let authorized = match authorize(&state.sources, &headers, Act::Issue).await {
        Ok(authorized) => authorized,
        Err(refused) => return refused,
    };
    let caller = &authorized.caller;
    let (jti, act) = (caller.jti.as_str(), caller.act.as_str());
    let (Some(gateway), Some(settings)) = (
        authorized.loaded.gateway.as_deref(),
        authorized.loaded.settings.as_deref(),
    ) else {
        return keys::unconfigured();
    };

    // The age backstop, before anything is asked of the control plane.
    let min_age = match settings.min_account_age_minutes().await {
        Ok(min_age) => min_age,
        Err(error) => {
            tracing::error!(
                jti,
                act,
                error = %error,
                outcome = "failed",
                "hub issue refused: the minimum account age could not be read"
            );
            return outcome("failed", None);
        }
    };
    match eligibility::account_age(&caller.discord_id, min_age, eligibility::now_ms()) {
        Some(AccountAge::OldEnough) => {}
        Some(AccountAge::TooYoung {
            wait_secs,
            old_enough_at_ms,
        }) => {
            tracing::info!(
                jti,
                act,
                outcome = "too_young",
                wait_secs,
                "hub issue refused"
            );
            return outcome("too_young", rfc3339(old_enough_at_ms.div_ceil(1000)));
        }
        // `token::verify` let only a snowflake through.
        None => {
            tracing::warn!(
                jti,
                act,
                "hub call carried an id with no account age; refusing"
            );
            return auth::invalid_token();
        }
    }

    match keys::issue_for(
        gateway,
        &caller.discord_id,
        state.deadline_from(started),
        ISSUED_BY_HUB,
    )
    .await
    {
        IssueOutcome::Issued { created, key_id } => {
            // A key exists now — perhaps a new one with a fresh counter in
            // place of a revoked one: whatever is cached is not about it.
            state.usage.cache_handle().invalidate(&caller.discord_id);
            let said = if created { "issued" } else { "has_key" };
            tracing::info!(
                jti,
                act,
                key_id = %key_id,
                outcome = said,
                "hub issued an API key"
            );
            outcome(said, None)
        }
        IssueOutcome::Capped {
            next_eligible_at, ..
        } => {
            tracing::info!(jti, act, outcome = "capped", "hub issue refused");
            outcome("capped", Some(next_eligible_at))
        }
        IssueOutcome::Failed => {
            // `issue_for` logged why.
            tracing::warn!(jti, act, outcome = "failed", "hub issue failed");
            outcome("failed", None)
        }
    }
}
