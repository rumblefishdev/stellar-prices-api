//! Who may call the contract, and for what (task 0348).
//!
//! Every route starts with [`authorize`], which settles the call or answers
//! it, in this order:
//!
//! 1. **The bearer.** `Authorization: Bearer <token>`, the scheme in any case
//!    (RFC 9110 §11.1), one space, a token with nothing after it. Anything
//!    else is `401 invalid_token`, before a source is loaded.
//! 2. **The two loads, together.** The portal's sources and the hub's secret
//!    are awaited with `tokio::join!`, never one after the other and never
//!    `select!`: in sequence the first call in an execution environment would
//!    pay `LOAD_BUDGET` plus `HUB_SECRET_BUDGET` before the control plane,
//!    and a `select!` that dropped the portal's load when the secret came
//!    back first would count as a failed load — closing `/api/config` and the
//!    portal's routes for the cooldown (`sources::FailureRecorder`).
//! 3. **The secret.** None — not configured, or not readable now — is `503
//!    hub_unconfigured`: the contract is closed, the portal is not.
//! 4. **The token.** [`token::verify`], which answers every refusal alike:
//!    `401 invalid_token`.
//! 5. **The act.** A token whose `act` is not the route's is `403
//!    action_not_allowed` — a `read` token cannot revoke, a `reveal` token
//!    cannot issue. One action per token, but not one call: `jti` is logged,
//!    never checked, so a captured token can be replayed for its own action
//!    until it expires — an accepted risk (ADR 0010 §8a).
//! 6. **The portal's sources.** A failed load is `503 keys_unconfigured`, the
//!    portal's own answer for it.
//!
//! Nothing here calls the control plane: a call refused at 1 to 5 has asked
//! AWS for nothing (the portal's own sources may load at 2 — the cached SSM
//! and Secrets Manager reads every portal route shares — but no API Gateway
//! call is made). The tests pin it with the mock's call counts.
//!
//! # What is logged
//!
//! A refusal at 4 or 5 is a WARN: the hub signs only good tokens, so one that
//! fails is a fault between the two backends (a secret changed on one side
//! only, a clock adrift) or a forgery, and either is worth seeing. A call with
//! no bearer at all is a DEBUG: the hub never makes one. It names the call by `jti` and `act` where the token got far
//! enough to have them — never the token, never the Discord id, never the
//! username (`token::HubCaller`'s `Debug` already hides the id).

use std::time::SystemTime;

use axum::Json;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use super::token::{self, Act, HubCaller};
use crate::common::errors;
use crate::portal::keys;
use crate::portal::sources::{Loaded, PortalSources};

/// A missing, malformed or refused token.
pub(crate) const INVALID_TOKEN: &str = "invalid_token";
/// A token for another action than the route's.
pub(crate) const ACTION_NOT_ALLOWED: &str = "action_not_allowed";
/// No hub secret to verify a token with.
pub(crate) const HUB_UNCONFIGURED: &str = "hub_unconfigured";

/// A call [`authorize`] let through: who, and the portal's sources.
pub(crate) struct Authorized<'a> {
    pub caller: HubCaller,
    pub loaded: &'a Loaded,
}

/// Settle who is calling and whether they may do `needs` — or the answer
/// that refuses them. See the module doc for the order.
pub(crate) async fn authorize<'a>(
    sources: &'a PortalSources,
    headers: &HeaderMap,
    needs: Act,
) -> Result<Authorized<'a>, Response> {
    let Some(bearer) = bearer(headers) else {
        // DEBUG, not WARN: the hub always sends a token, so a call without
        // one is a stranger at a public URL, and one line each is noise.
        tracing::debug!(needs = needs.as_str(), "hub call refused: no bearer token");
        return Err(invalid_token());
    };

    let (loaded, secret) = tokio::join!(sources.get(), sources.hub().get());

    let Some(secret) = secret else {
        // `HubSource::get` logged why.
        return Err(errors::service_unavailable(
            HUB_UNCONFIGURED,
            "the hub contract is not configured on this deployment",
        ));
    };
    let Some(caller) = token::verify(bearer, &secret, SystemTime::now()) else {
        tracing::warn!(
            needs = needs.as_str(),
            "hub call refused: its token did not verify"
        );
        return Err(invalid_token());
    };
    if caller.act != needs {
        tracing::warn!(
            jti = %caller.jti,
            act = caller.act.as_str(),
            needs = needs.as_str(),
            "hub call refused: its token is for another action"
        );
        return Err((
            StatusCode::FORBIDDEN,
            Json(errors::ErrorEnvelope {
                code: ACTION_NOT_ALLOWED,
                message: format!("this call needs a token for `{}`", needs.as_str()),
                details: None,
            }),
        )
            .into_response());
    }
    let Some(loaded) = loaded else {
        // `PortalSources::get` logged why, under the portal's own alarm.
        return Err(keys::unconfigured());
    };
    Ok(Authorized { caller, loaded })
}

/// The token in `Authorization: Bearer <token>`, or `None` for anything else
/// — no header, another scheme, no token, or more than one `Authorization`.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all(AUTHORIZATION).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// `401 invalid_token`, one answer for every refusal: telling them apart
/// would be a probing oracle (`token` says the same).
pub(crate) fn invalid_token() -> Response {
    errors::unauthorized_with(
        INVALID_TOKEN,
        "the hub token is missing, malformed, expired or not ours",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(values: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for value in values {
            map.append(AUTHORIZATION, value.parse().unwrap());
        }
        map
    }

    #[test]
    fn the_bearer_is_the_token_after_the_scheme_and_nothing_else() {
        assert_eq!(bearer(&headers(&["Bearer v1.a.b"])), Some("v1.a.b"));
        assert_eq!(bearer(&headers(&["bearer v1.a.b"])), Some("v1.a.b"));
        assert_eq!(bearer(&headers(&["BEARER v1.a.b"])), Some("v1.a.b"));
        for refused in [
            &[][..],
            &["Bearer"],
            &["Bearer "],
            &["Basic v1.a.b"],
            &["v1.a.b"],
            &["Bearer v1.a.b", "Bearer v1.a.b"],
        ] {
            assert_eq!(bearer(&headers(refused)), None, "{refused:?}");
        }
        // What follows the one space is the token, whatever it is: a second
        // space or a suffix is the verifier's to refuse.
        assert_eq!(bearer(&headers(&["Bearer  v1.a.b"])), Some(" v1.a.b"));
    }
}
