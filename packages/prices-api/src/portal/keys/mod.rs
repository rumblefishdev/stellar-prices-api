//! Issue a key, and show it (task 0187).
//!
//! Two routes under [`PORTAL_API_PREFIX`](super::PORTAL_API_PREFIX), so they
//! inherit [0183]'s gate exactly as sign-in does: with `PORTAL_ENABLED` false
//! both are an empty `404`, byte-identical to a path that was never deployed.
//!
//! | route | does |
//! | --- | --- |
//! | `POST /api-tokens/api/key` | issue — or adopt the one that already exists |
//! | `GET /api-tokens/api/key` | reveal — the same lookup, plus the value |
//!
//! # There is no database, and that is the design
//!
//! **API Gateway is the source of truth for whether a key exists.** Task 0158
//! designed a registry table and its own "Issue flow" section explains why it is
//! not needed first: a key's name is derived from the Discord user id, so the
//! control plane can be *asked*. The registry buys a hot-path read and a
//! history; neither is required to answer "where is my key", and whether it is
//! ever needed is [0190]'s to justify.
//!
//! What follows from that is the shape of everything here: every request is a
//! **reconciliation**, not a lookup of something we wrote down. The flow is
//! list → filter → rank → converge, and it is the same flow for both routes,
//! which is why the task groups them ("reveal is the same lookup as issue").
//!
//! # Why a `GET` may create
//!
//! Reveal re-enters the issue flow when there is nothing to reveal, per the
//! task: *"a key deleted by hand in the console otherwise leaves the user with a
//! dead id forever"*. Without a registry there is no way to tell "deleted by
//! hand" from "never issued" — that distinction is precisely what the registry
//! would have stored — so honouring that requirement means a `GET` that can
//! create.
//!
//! That is worth stating plainly rather than burying, because `auth/mod.rs`
//! warns against inheriting its CSRF reasoning here. The exposure is a
//! third-party page causing a top-level navigation to this URL, which
//! `SameSite=Lax` does send the session cookie on. What it buys an attacker is
//! bounded to nothing:
//!
//! - The flow is **idempotent**. A visitor who has a key gets that same key; a
//!   visitor who does not gets the one key they were entitled to press a button
//!   for. No amount of forged navigation produces a second key, because the
//!   reconciler deletes all but one.
//! - The response is **not readable cross-origin**. A navigation renders JSON in
//!   the victim's own tab; the attacker's page cannot read it, and no CORS
//!   header here says otherwise.
//! - `POST` is not reachable cross-site at all under `SameSite=Lax`, which only
//!   releases the cookie for top-level `GET`.
//!
//! So the worst outcome is that a visitor ends up holding the key they could
//! have issued themselves — the same reasoning `auth`'s sign-out uses, and it is
//! restated rather than inherited because the conclusion had to be re-derived
//! for a route that creates a production credential.
//!
//! # Never log a key value
//!
//! Enforced by [`gateway::KeyValue`], which has no `Display`, no `Serialize` and
//! a `Debug` that prints `<redacted>` — so a `?value` in a `tracing` macro, a
//! `#[derive(Debug)]` on any wrapper, and a panic message carrying one are all
//! safe by construction rather than by review. The single exit is
//! `KeyValue::expose`, which has exactly one call site: [`KeyResponse`]'s
//! construction. This module also sets no X-Ray annotations of its own; the
//! tracing subscriber's `info` level records the route and the outcome, never
//! the credential.

pub mod gateway;
pub mod naming;

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;

use crate::common::{cache_control, errors};

use super::auth::secret::OauthSecret;
use gateway::{Attachment, Gateway, GatewayError, KeyValue};
use naming::{KeyRecord, choose_winner, exact_matches, key_name, losers};

/// Issue (`POST`) and reveal (`GET`) share one path — they are one resource.
pub const KEY_PATH: &str = "/api-tokens/api/key";

/// Error code for a caller with no valid session.
const NOT_SIGNED_IN: &str = "not_signed_in";
/// Error code for a control plane that would not answer.
const KEY_UNAVAILABLE: &str = "key_unavailable";
/// Error code for a deployment with the portal open and no usage plan wired.
const KEYS_UNCONFIGURED: &str = "keys_unconfigured";

/// How many times the whole flow is re-run when the key it settled on turns out
/// to have been deleted underneath it.
///
/// Two attempts, not a loop until success. The condition being retried is a
/// concurrent deletion, which is rare and does not persist; a caller that hits
/// it twice is looking at something systematic — a reconciler fighting another
/// process, or a console session deleting keys as fast as they are made — and
/// spending a Lambda's remaining budget on a third attempt would turn that into
/// a timeout instead of an answer.
const MAX_ATTEMPTS: usize = 2;

/// What both handlers need, cloned per request.
///
/// Both fields are `Option` for the same reason `AuthState::oauth` is: the
/// api-handler must boot with the portal closed and nothing provisioned. The
/// routes are mounted regardless, and answer `503` rather than not existing, so
/// that a deployment which opens the portal without wiring the usage plan says
/// so instead of looking like a portal with no key issuance.
#[derive(Clone)]
pub struct KeysState {
    /// Verifies the session cookie. The same secret sign-in issued it with —
    /// there is one signing key and [`super::auth::crypto`]'s domain separation
    /// is what keeps its three token kinds apart.
    oauth: Option<Arc<OauthSecret>>,
    gateway: Option<Arc<Gateway>>,
}

impl KeysState {
    pub fn new(oauth: Option<OauthSecret>, gateway: Option<Gateway>) -> Self {
        Self {
            oauth: oauth.map(Arc::new),
            gateway: gateway.map(Arc::new),
        }
    }
}

/// The two routes, as a `Router` carrying its own state.
///
/// One `.route()` with both verbs, so the path is written once: a second
/// `.route()` for the same path would panic at construction, and that is the
/// only thing keeping issue and reveal from drifting apart in the router.
pub fn routes(state: KeysState) -> Router {
    Router::new()
        .route(KEY_PATH, get(key).post(key))
        .with_state(state)
}

/// What both routes answer with.
///
/// `value` is the credential. It is in the body of a `no-store` response over a
/// path CloudFront is told not to cache and API Gateway is told not to cache —
/// three independent statements of the same rule, because a cached reveal hands
/// one visitor another visitor's key (the gateway's cache has no cache-key
/// parameters, so every caller collapses onto one entry).
///
/// **No `Debug`.** This is the one type in the module that holds the value as a
/// plain `String` — [`KeyValue`]'s protection ends where `expose` is called, and
/// that is here. A derived `Debug` would put the credential back within reach of
/// a `?response` in a `tracing` macro, which is precisely the hole the type
/// exists to close; `Serialize` is the only thing this struct is for.
#[derive(Serialize)]
struct KeyResponse {
    /// The API Gateway key id. Not a secret; useful in a support conversation.
    key_id: String,
    /// The key name, `discord-<userId>-key`.
    name: String,
    /// The key itself — what goes in `X-API-Key`.
    value: String,
    /// Whether this request created the key, as opposed to finding it.
    ///
    /// Display only, and honest about a race: two simultaneous first presses can
    /// both report `true` while converging on one key, because each created one
    /// and one of the two was then reconciled away.
    created: bool,
}

/// Both verbs. `POST` issues, `GET` reveals, and they are **one handler**
/// because they are one operation.
///
/// That is not a shortcut: task 0187 says "reveal is the same lookup as issue",
/// and without a registry it has to be. There is no stored key id to reveal, so
/// a reveal lists, filters, ranks and converges exactly as an issue does, and
/// the only thing left that could differ between them is whether they are
/// allowed to create — which they cannot be, because "deleted by hand" and
/// "never issued" are the same observation. Two functions with identical bodies
/// would have claimed a distinction that does not exist and invited someone to
/// invent one.
async fn key(State(state): State<KeysState>, headers: HeaderMap) -> Response {
    ensure_key(&state, &headers).await
}

/// The whole of both routes: authenticate, reconcile, answer.
async fn ensure_key(state: &KeysState, headers: &HeaderMap) -> Response {
    let Some(oauth) = state.oauth.as_ref() else {
        return unconfigured();
    };
    let Some(gateway) = state.gateway.as_ref() else {
        return unconfigured();
    };

    // The session is the authorization for this whole route. There is no API
    // key to present — the caller is here to get one — so the signed cookie is
    // the only thing standing between a stranger and a production credential,
    // and `PORTAL_ENABLED` is the only thing standing in front of that until
    // [0189]'s eligibility gate lands.
    let Some(session) = super::auth::current_session(oauth, headers) else {
        return no_store(errors::unauthorized_with(
            NOT_SIGNED_IN,
            "sign in with Discord before issuing an API key",
        ));
    };

    // A `sub` that is not a snowflake cannot become a key name. Unreachable
    // while the signing key is intact — Discord ids are digits and the cookie is
    // signed — which is exactly why it is checked: this is the last line if that
    // stops being true, and the alternative is an attacker-chosen `nameQuery`
    // aimed at the reconciler's `DeleteApiKey`.
    let Some(name) = key_name(&session.sub) else {
        tracing::warn!("a session carried a user id that is not a snowflake; refusing to issue");
        return no_store(errors::unauthorized_with(
            NOT_SIGNED_IN,
            "sign in with Discord before issuing an API key",
        ));
    };

    match reconcile(gateway, &name).await {
        Ok(Some(outcome)) => {
            tracing::info!(
                key_id = %outcome.record.id,
                created = outcome.created,
                "portal issued or revealed an API key"
            );
            no_store(
                Json(KeyResponse {
                    key_id: outcome.record.id,
                    name: outcome.record.name,
                    // The one call site of `expose`, and the reason the type
                    // exists: everything else in this module can only hold the
                    // value, never read it.
                    value: outcome.value.expose().to_string(),
                    created: outcome.created,
                })
                .into_response(),
            )
        }
        // Every attempt found a key and then lost it before reading its value.
        Ok(None) => {
            tracing::warn!(
                attempts = MAX_ATTEMPTS,
                "a key was deleted underneath every issue attempt"
            );
            no_store(errors::service_unavailable(
                KEY_UNAVAILABLE,
                "your key is being changed by something else right now; try again",
            ))
        }
        Err(error) => {
            // `error` cannot carry a key value — see `gateway::sdk_message`.
            tracing::error!(error = %error, "portal key issuance failed");
            no_store(
                (
                    StatusCode::BAD_GATEWAY,
                    Json(errors::ErrorEnvelope {
                        code: KEY_UNAVAILABLE,
                        message: "could not reach the API key service; try again".into(),
                        details: None,
                    }),
                )
                    .into_response(),
            )
        }
    }
}

/// The result of a successful reconciliation.
struct Outcome {
    record: KeyRecord,
    value: KeyValue,
    created: bool,
}

/// List, filter, rank, converge — the reconciler, run up to [`MAX_ATTEMPTS`]
/// times.
///
/// `Ok(None)` means every attempt settled on a key that was deleted before its
/// value could be read. That is a real, transient state (a console session, or
/// another invocation's reconciler) and it is reported as such rather than as a
/// failure of this service.
async fn reconcile(gateway: &Gateway, name: &str) -> Result<Option<Outcome>, GatewayError> {
    for _ in 0..MAX_ATTEMPTS {
        if let Some(outcome) = attempt(gateway, name).await? {
            return Ok(Some(outcome));
        }
    }
    Ok(None)
}

/// One pass of the reconciler.
async fn attempt(gateway: &Gateway, name: &str) -> Result<Option<Outcome>, GatewayError> {
    // Step 1: everything the control plane has under this name PREFIX, across
    // all pages, narrowed to exact equality here. Both halves matter and both
    // are argued for in `naming`.
    let existing = exact_matches(gateway.list_named(name).await?, name);

    let mut created: Option<(KeyRecord, KeyValue)> = None;
    let mut candidates = existing;

    // Step 2: nothing yet — create one.
    if candidates.is_empty() {
        let (record, value) = gateway.create(name).await?;

        // Re-list rather than returning what we just made. Two simultaneous
        // first presses both find nothing and both create, so the list is the
        // only place the collision is visible — and this is where the
        // acceptance criterion "two concurrent presses converge on one key, and
        // the loser is deleted" is actually met. The rank rule is deterministic
        // (`naming::choose_winner`), so both invocations pick the same survivor
        // from the same list rather than each deleting the other's.
        candidates = exact_matches(gateway.list_named(name).await?, name);
        if candidates.is_empty() {
            // The control plane did not list what it just created. Rather than
            // fail, trust the create — it answered with an id and a value, and
            // the next request will reconcile whatever this leaves behind.
            // `KeyGone` here means the key we just made was deleted between
            // the create and this call. Handing its value out would be handing
            // out a dead id — the one thing the adopt-or-recreate rule exists to
            // prevent — so this re-enters the flow like any other lost race.
            if gateway.attach_to_free_plan(&record.id).await? == Attachment::KeyGone {
                return Ok(None);
            }
            return Ok(Some(Outcome {
                record,
                value,
                created: true,
            }));
        }
        created = Some((record, value));
    }

    // Step 3: one survivor.
    //
    // `candidates` is non-empty on every path that reaches here — the branch
    // above returns early when it is not — so the `else` is unreachable. It is
    // written as a return rather than an `expect` anyway, because task 0186's
    // review found the cost of being wrong about that: a panic in a public
    // handler is not a `500`, it is a dropped connection with no response at
    // all (`curl` reports `000`) and an invocation error on the Lambda's
    // `Errors` metric. Falling through to the retry answers `503` instead.
    let Some(winner) = choose_winner(&candidates).cloned() else {
        return Ok(None);
    };

    // Step 4: the winner is on the plan before anybody is handed it — however
    // it came to exist.
    //
    // **Not only on the create path**, which is where this call used to live and
    // where it left a hole with no way out of it. A key can exist and be on no
    // usage plan for three reasons, and only the first is ours to prevent:
    //
    // 1. `CreateApiKey` succeeded and this call then failed (a throttle, a
    //    timeout — these are control-plane calls), so the attempt answered `502`
    //    with the key already made;
    // 2. `CreateApiKey` timed out *after* the service created the key, so no id
    //    ever reached this process and nothing could have attached it;
    // 3. somebody created a key with exactly this name in the console.
    //
    // In every one of them the next request adopts that key — and, when the
    // attach lived on the create path, adopted it forever: the holder got a
    // `200` with a key that answers `403` on `/v1/`, and no retry could fix it
    // because every retry took the same branch. That is the "issued but does not
    // work" state this call exists to prevent, made permanent.
    //
    // Idempotent (see `Gateway::attach_to_free_plan`), so the common case — a
    // key already on the plan — costs one `409` and no state change. Before the
    // deletions below rather than after: the key the caller is about to receive
    // is made usable first, and the destructive half of reconciliation only runs
    // once that has succeeded.
    //
    // A `404` from it is not a failure but the deletion race — the winner was
    // listed and is gone — and it is reported the way `value_of`'s `None` is:
    // re-run the whole flow, which adopts whatever survived or creates a
    // replacement. Because this call now runs before the read, it is the first
    // place that race can surface, so it has to answer it rather than turn a
    // hand-deleted key back into the dead end this slice exists to remove.
    if gateway.attach_to_free_plan(&winner.id).await? == Attachment::KeyGone {
        return Ok(None);
    }

    // Step 5: everything that is not the winner is deleted.
    for loser in losers(&candidates, &winner) {
        // Re-asserted immediately before the destructive call, on the record
        // itself rather than on the query that produced it. `exact_matches` has
        // already run; this is the guard that survives somebody "simplifying"
        // that away, and it is the difference between deleting a key of ours and
        // deleting whatever a prefix query happened to return.
        if loser.name != name {
            tracing::error!(
                key_id = %loser.id,
                "refusing to delete a key whose name is not the caller's; \
                 the exact-name filter has been bypassed"
            );
            continue;
        }
        tracing::info!(key_id = %loser.id, "reconciling away a duplicate portal key");
        // Logged and stepped over, NOT propagated. By the time this loop runs
        // the winner is created, attached and ready to be handed out, so a `?`
        // here would answer `502` and withhold a key that demonstrably works —
        // housekeeping denying the thing the request was for. The duplicate is
        // left for the next reconciliation, which is deterministic and will
        // pick the same winner, so nothing diverges by waiting.
        //
        // This is not hypothetical: `compute-stack.ts` records that task 0194
        // may put an `aws:ResourceTag/ManagedBy` condition on `DELETE`, and an
        // exact-name duplicate created by hand in the console carries no tag.
        // With a `?` here that key would `AccessDenied` on every request and the
        // user would never get theirs at all.
        if let Err(error) = gateway.delete(&loser.id).await {
            tracing::error!(
                key_id = %loser.id,
                error = %error,
                "could not delete a duplicate portal key; leaving it for the next reconciliation"
            );
        }
    }

    // Step 6: the value. Free if we just created the winner; otherwise a read.
    if let Some((record, value)) = created
        && record.id == winner.id
    {
        return Ok(Some(Outcome {
            record: winner,
            value,
            created: true,
        }));
    }

    // `None` here is the raced deletion: the winner existed when it was listed
    // and does not now. The caller re-runs the whole flow, which will adopt
    // whatever survived or create a replacement — this is the "not returned as a
    // dead id" property, and the reason a reveal is a reconciliation rather than
    // a lookup.
    match gateway.value_of(&winner.id).await? {
        Some(value) => Ok(Some(Outcome {
            record: winner,
            value,
            created: false,
        })),
        None => Ok(None),
    }
}

/// `no-store` on every response this module produces.
///
/// The handler's own statement of the rule, alongside the two configurations
/// that state it as well (`CACHING_DISABLED` on CloudFront's API behaviour,
/// `cachingEnabled: false` in `portalSettings`). Three layers because the
/// failure they prevent is one visitor being handed another visitor's
/// credential, and configuration drifts.
fn no_store(mut response: Response) -> Response {
    cache_control::attach(&mut response, cache_control::NO_STORE);
    response
}

/// `503` for a deployment that reached these routes with nothing wired.
fn unconfigured() -> Response {
    no_store(errors::service_unavailable(
        KEYS_UNCONFIGURED,
        "API key issuance is not configured on this deployment",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both verbs, one path — and the path is under the portal prefix, so
    /// [0183]'s gate covers it without knowing it exists.
    #[test]
    fn the_route_lives_under_the_gated_prefix() {
        assert!(KEY_PATH.starts_with(super::super::PORTAL_API_PREFIX));
    }

    /// Depth 3 under `/api-tokens/api/`, like sign-in: the currently-deployed
    /// gateway maps depth 1-2 only, which is [0205]'s to fix. Asserted so that
    /// nobody moves this route deeper without noticing it makes the release
    /// dependency worse.
    #[test]
    fn the_route_is_one_segment_under_the_prefix() {
        let rest = KEY_PATH.trim_start_matches(super::super::PORTAL_API_PREFIX);
        assert_eq!(rest, "key");
        assert!(!rest.contains('/'));
    }
}
