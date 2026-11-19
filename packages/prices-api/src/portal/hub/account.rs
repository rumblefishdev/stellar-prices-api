//! `GET /api/hub/v1/account` (task 0348): the account's key and its use, in
//! one answer the hub's Prices tab renders.
//!
//! ```json
//! {
//!   "key": { "state": "none" | "active" | "revoked", "key_id"?, "created_at"?,
//!            "last_updated_at"?, "revoked_at"?, "next_eligible_at"?, "can_issue": bool },
//!   "details": { "usage": <UsageResponse> | null, "free_rate_limit_per_second": n | null }
//! }
//! ```
//!
//! # One listing, one deadline, no value
//!
//! What the portal's dashboard asked of three routes (`/key`, `/usage`,
//! `/config`), from one `GetApiKeys`: the key's state is
//! [`keys::key_status`] of the listing, and the usage is
//! [`UsageState::answer_from_listing`] of the same listing — its plan and
//! counter, or the usage cache's entry when it agrees with what was listed.
//! Both halves run under one deadline (`HubState::deadline_from`), so a cold
//! call's loads and the control plane together stay inside the invocation.
//!
//! The token's `act` is `read`, and a read never pulls a value it does not
//! need: the key's value is not read here at all. `reveal` reads it.
//!
//! # The states
//!
//! - `none`: no key under the account's name. `can_issue`.
//! - `active`: the live key, with its id and dates. A new one cannot be had:
//!   one live key per account.
//! - `revoked`: every key under the name is off. While the revocation's
//!   period lasts, `next_eligible_at` names when a new one can be had and
//!   `can_issue` is false — the month cap; after it, `can_issue`, and the
//!   usage is `null` as `/api/usage` says "no key" for it.
//!
//! An instant AWS did not date is left out rather than invented.
//!
//! # When the usage half fails
//!
//! The key's state is the answer's point, and the listing that gives it
//! succeeded: a usage read that fails or runs out of time (with nothing
//! cached to fall back on that agrees with the listing — a "no key" cached
//! before the key was issued elsewhere is not one) leaves `usage` `null`
//! beside it, as the portal's
//! dashboard states no figure while `/usage` fails, and logs why. The listing
//! itself failing is the portal's `502` / `503 key_unavailable`.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use tokio::time::Instant;

use super::HubState;
use super::auth::{self, authorize};
use super::token::Act;
use crate::portal::keys::naming::{exact_matches, key_name};
use crate::portal::keys::{self, KeyError, KeyStatus, rfc3339};
use crate::portal::usage::{UsageOutcome, UsageResponse};

/// The whole answer.
#[derive(Serialize)]
struct AccountResponse {
    key: KeyView,
    details: Details,
}

/// The key's state, as the tab renders it.
#[derive(Serialize)]
struct KeyView {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_eligible_at: Option<String>,
    /// Whether `key/issue` would issue now, as far as the key's state says:
    /// no key, or one revoked in an earlier period.
    can_issue: bool,
}

/// Prices' own part of the answer: what `/api/usage` and `/api/config` say.
#[derive(Serialize)]
struct Details {
    usage: Option<Box<UsageResponse>>,
    free_rate_limit_per_second: Option<u32>,
}

impl KeyView {
    fn of(status: &KeyStatus) -> Self {
        let blank = KeyView {
            state: "none",
            key_id: None,
            created_at: None,
            last_updated_at: None,
            revoked_at: None,
            next_eligible_at: None,
            can_issue: true,
        };
        match status {
            KeyStatus::None => blank,
            KeyStatus::Active(record) => KeyView {
                state: "active",
                key_id: Some(record.id.clone()),
                created_at: record.created_at.and_then(rfc3339),
                last_updated_at: record.last_updated_at.and_then(rfc3339),
                can_issue: false,
                ..blank
            },
            KeyStatus::Revoked {
                revoked_at,
                next_eligible_at,
            } => KeyView {
                state: "revoked",
                revoked_at: revoked_at.and_then(rfc3339),
                next_eligible_at: next_eligible_at.clone(),
                can_issue: next_eligible_at.is_none(),
                ..blank
            },
        }
    }
}

/// The route. See the module doc.
pub(crate) async fn account(State(state): State<HubState>, headers: HeaderMap) -> Response {
    let started = Instant::now();
    let authorized = match authorize(&state.sources, &headers, Act::Read).await {
        Ok(authorized) => authorized,
        Err(refused) => return refused,
    };
    let caller = &authorized.caller;
    let jti = caller.jti.as_str();
    let act = caller.act.as_str();
    let Some(gateway) = authorized.loaded.gateway.as_deref() else {
        return keys::unconfigured();
    };
    // `token::verify` let only a snowflake through; this is the last line
    // before a `nameQuery`, as in every key route.
    let Some(name) = key_name(&caller.discord_id) else {
        tracing::warn!(
            jti,
            act,
            "hub call carried an id that names no key; refusing"
        );
        return auth::invalid_token();
    };

    let deadline_at = Instant::now() + state.deadline_from(started);
    // Begun before the listing: the usage cache's epoch has to cover it.
    let usage_start = state.usage.begin(&caller.discord_id);

    let listed = match tokio::time::timeout_at(deadline_at, gateway.list_named(&name)).await {
        Ok(Ok(records)) => exact_matches(records, &name),
        Ok(Err(error)) => {
            // `error` cannot carry a key value — see `gateway::sdk_message`.
            tracing::error!(jti, act, error = %error, "hub account lookup failed");
            return keys::key_unavailable(KeyError::Unavailable);
        }
        Err(_elapsed) => {
            tracing::error!(jti, act, "hub account lookup ran out of time");
            return keys::key_unavailable(KeyError::OutOfTime);
        }
    };

    let status = keys::key_status(&listed);
    let usage = match state
        .usage
        .answer_from_listing(
            gateway,
            &caller.discord_id,
            usage_start,
            &listed,
            deadline_at.saturating_duration_since(Instant::now()),
        )
        .await
    {
        UsageOutcome::Usage(body) => Some(body),
        UsageOutcome::NoKey => None,
        // Logged inside, in the usage lookup's own words; this says what the
        // hub was told.
        UsageOutcome::OutOfTime
        | UsageOutcome::Throttled
        | UsageOutcome::Failed
        | UsageOutcome::NotASnowflake => {
            tracing::warn!(jti, act, "hub account answered without its usage");
            None
        }
    };

    let key = KeyView::of(&status);
    tracing::info!(
        jti,
        act,
        state = key.state,
        // As every other line writes it (`key_id=<id>`), and left out when
        // the state names no key.
        key_id = key.key_id.as_deref().map(tracing::field::display),
        usage = usage.is_some(),
        "hub answered an account"
    );
    Json(AccountResponse {
        key,
        details: Details {
            usage,
            free_rate_limit_per_second: state.rate_limit,
        },
    })
    .into_response()
}
