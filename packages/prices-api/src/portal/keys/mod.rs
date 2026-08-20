//! Reveal a key — and issue one, but only for [0189]'s eligibility-checked
//! callback (task 0187, re-shaped by task 0189).
//!
//! One route under [`PORTAL_API_PREFIX`](super::PORTAL_API_PREFIX), so it
//! inherits [0183]'s gate exactly as sign-in does: with `PORTAL_ENABLED` false
//! it is an empty `404`, byte-identical to a path that was never deployed.
//!
//! | route | does |
//! | --- | --- |
//! | `GET`/`POST /api-tokens/api/key` | reveal — the lookup, plus the value. **Never creates.** |
//!
//! Issuance itself is [`issue_for`], reachable **only** from the OAuth
//! callback completing an `action=issue` round-trip
//! (`super::auth::issue`) — because issuing requires an eligibility proof
//! (Stellar Discord membership + minimum account age, ADR 0010 §8) that only a
//! fresh Discord token can provide, and a session cookie is not one.
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
//! # The route is read-only, and that is [0189]'s invariant
//!
//! Task 0187 shipped this route as one create-capable handler for both verbs
//! ("reveal is the same lookup as issue"), with a documented argument for why
//! a `GET` that creates was safe. [0189] re-derives the question and reverses
//! the answer, because the premise changed: issuing now requires an
//! eligibility proof, so a route reachable with a session cookie alone **must
//! not create** — that is the acceptance criterion "issue is unreachable with
//! a session cookie alone", made structural. The reveal is now a pure lookup:
//! list → filter → rank → read. No create, no attach, no delete — a session
//! cookie can cause **zero control-plane writes**, which also retires 0187's
//! `SameSite=Lax` top-level-navigation concern outright rather than bounding
//! it.
//!
//! What that costs, and why it is fine:
//!
//! - **A key deleted by hand in the console is no longer resurrected by a
//!   reveal.** The caller sees "no key" and the page offers the issue
//!   round-trip; one press re-proves eligibility and recreates it. That is
//!   strictly better than 0187's behaviour — the recreate now happens behind
//!   the gate instead of around it.
//! - **A winner that exists but was never attached** (a crash between create
//!   and attach) reveals as a key that answers `403` on `/v1/`. The heal is
//!   the same press: [`issue_for`] adopts and attaches it. The reveal must
//!   not fix it in place, because attach is a write.
//! - **Duplicates are no longer swept by reveal** — [`issue_for`] still
//!   converges them. Until the owner next issues, the reveal answers with the
//!   same deterministic winner the issue flow would pick
//!   (`naming::choose_winner`), so nothing diverges by waiting.
//!
//! **A user who has left the Stellar Discord keeps their key, and this route
//! keeps working for them.** Reveal consults the session only — never
//! Discord. That is the epic's stated non-goal, not an oversight: membership
//! is proved at the moment of issuance and nothing afterwards (ADR 0010 §7);
//! what a departed member loses is the right to rework ([0191]).
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
use std::time::Duration;

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

/// The reveal, on both verbs — see [`key`] for why `POST` answers identically.
pub const KEY_PATH: &str = "/api-tokens/api/key";

/// Error code for a caller with no valid session.
const NOT_SIGNED_IN: &str = "not_signed_in";
/// Error code for a control plane that would not answer.
const KEY_UNAVAILABLE: &str = "key_unavailable";
/// Error code for a deployment with the portal open and no usage plan wired.
const KEYS_UNCONFIGURED: &str = "keys_unconfigured";
/// Error code for a caller who has no key. The same code (and the same
/// envelope-vs-empty-body distinction from [0183]'s gate) as the usage route's,
/// because the frontend reads both: a `404` is "no key" only when this
/// envelope says so.
const NO_KEY: &str = "no_key";

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

/// Wall-clock ceiling on the whole reconciliation, deletions and retries
/// included.
///
/// **A deadline, because the per-call timeouts do not compose into one.**
/// `gateway::OPERATION_TIMEOUT` bounds a single control-plane call; one attempt
/// makes up to six of them plus one delete per duplicate, `list_named` alone can
/// walk `gateway::MAX_PAGES` pages each with its own budget, and this runs
/// [`MAX_ATTEMPTS`] times. The arithmetic reaches minutes against an
/// `apiHandler.timeoutSeconds` of 15 — at which point Lambda kills the
/// invocation and the caller gets no response at all: no body, no `no-store`, an
/// entry on the `Errors` metric, and possibly a key created but never attached.
/// That is the failure mode task 0186's F7 and this task's R1 both went out of
/// their way to remove; a slow control plane must not reintroduce it.
///
/// 10s leaves the handler ~5s of the function's budget to serialize an answer
/// and for the runtime to send it, and sits far inside API Gateway's own 29s
/// cap.
pub(crate) const RECONCILE_DEADLINE: Duration = Duration::from_secs(10);

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
    /// [`RECONCILE_DEADLINE`], overridable only outside the Lambda build.
    deadline: Duration,
    /// Task 0188's usage cache, so a successful issue can evict a cached
    /// "no key" answer that it has just made false. `None` costs nothing but
    /// a stale dashboard section for up to that cache's TTL; see
    /// [`super::usage::UsageCache`] for why the eviction is this narrow.
    usage_cache: Option<super::usage::UsageCache>,
}

impl KeysState {
    pub fn new(oauth: Option<OauthSecret>, gateway: Option<Gateway>) -> Self {
        Self {
            oauth: oauth.map(Arc::new),
            gateway: gateway.map(Arc::new),
            deadline: RECONCILE_DEADLINE,
            usage_cache: None,
        }
    }

    /// Wire task 0188's usage cache in, so issue and reveal can evict a
    /// cached "no key". Called by [`super::apply`]; optional so every
    /// existing constructor and test stays valid.
    pub fn with_usage_cache(mut self, cache: super::usage::UsageCache) -> Self {
        self.usage_cache = Some(cache);
        self
    }

    /// Shorten the deadline, so a test can drive the timeout in milliseconds
    /// instead of waiting ten seconds for it.
    ///
    /// **Compiled out of the Lambda**, exactly as `Gateway::against` and the
    /// endpoint overrides are, and for a reason of its own: the deadline is what
    /// keeps a slow control plane from turning into a dropped invocation, and a
    /// deployed build must contain no way to set it to something that does not.
    #[cfg(not(feature = "lambda"))]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }
}

/// The route, as a `Router` carrying its own state.
///
/// One `.route()` with both verbs, so the path is written once: a second
/// `.route()` for the same path would panic at construction, and that is the
/// only thing keeping the two verbs from drifting apart in the router.
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
}

/// Both verbs, one handler, and both are the **reveal**.
///
/// `POST` used to be the issue; issuance now lives behind the eligibility
/// round-trip (`super::auth::issue`), and this route must not create however
/// it is called. Both verbs stay routed and answer identically so that "a
/// `POST` with a session cookie creates nothing" is a *tested property* of a
/// live route rather than a hole left by an unrouted one — and so the
/// gateway's mapped verbs need no change.
async fn key(State(state): State<KeysState>, headers: HeaderMap) -> Response {
    reveal(&state, &headers).await
}

/// The whole of the route: authenticate, look up, answer. Read-only — see the
/// module docs for why that is [0189]'s invariant, not an optimisation.
async fn reveal(state: &KeysState, headers: &HeaderMap) -> Response {
    let Some(oauth) = state.oauth.as_ref() else {
        return unconfigured();
    };
    let Some(gateway) = state.gateway.as_ref() else {
        return unconfigured();
    };

    // The session is the authorization for this route — and for this route it
    // is enough, because a reveal only shows the caller what already belongs
    // to them. Creating is what needs more (an eligibility proof via the
    // issue round-trip), which is why nothing below can create.
    let Some(session) = super::auth::current_session(oauth, headers) else {
        return no_store(errors::unauthorized_with(
            NOT_SIGNED_IN,
            "sign in with Discord before asking for your API key",
        ));
    };

    // A `sub` that is not a snowflake cannot become a key name. Unreachable
    // while the signing key is intact — Discord ids are digits and the cookie is
    // signed — which is exactly why it is checked: this is the last line if that
    // stops being true, and the alternative is an attacker-chosen `nameQuery`
    // aimed at the control plane.
    let Some(name) = key_name(&session.sub) else {
        tracing::warn!("a session carried a user id that is not a snowflake; refusing");
        return no_store(errors::unauthorized_with(
            NOT_SIGNED_IN,
            "sign in with Discord before asking for your API key",
        ));
    };

    // The deadline wraps the whole lookup — `list_named` alone can walk pages,
    // each with its own per-call budget. Elapsing is a `503` rather than a
    // Lambda-killed invocation with no response at all.
    let looked_up = match tokio::time::timeout(state.deadline, lookup(gateway, &name)).await {
        Ok(looked_up) => looked_up,
        Err(_elapsed) => {
            tracing::error!(
                deadline_secs = state.deadline.as_secs_f32(),
                "portal key lookup ran out of time"
            );
            return no_store(errors::service_unavailable(
                KEY_UNAVAILABLE,
                "the API key service is taking too long to answer; try again",
            ));
        }
    };

    match looked_up {
        Ok(Some((record, value))) => {
            tracing::info!(key_id = %record.id, "portal revealed an API key");
            // This response proves a key exists, so a cached "no key" on the
            // usage route is now false. An in-process eviction, not a
            // control-plane write — the read-only invariant is about what a
            // session cookie can make AWS do.
            if let Some(cache) = &state.usage_cache {
                cache.invalidate_no_key(&session.sub);
            }
            no_store(
                Json(KeyResponse {
                    key_id: record.id,
                    name: record.name,
                    // The one call site of `expose`, and the reason the type
                    // exists: everything else in this module can only hold the
                    // value, never read it.
                    value: value.expose().to_string(),
                })
                .into_response(),
            )
        }
        // Nothing under this name — never issued, deleted by hand, or (rarely)
        // deleted between the list and the read. All three answer the same
        // envelope, because without a registry they are the same observation,
        // and all three are fixed the same way: the issue round-trip.
        Ok(None) => no_store(
            (
                StatusCode::NOT_FOUND,
                Json(errors::ErrorEnvelope {
                    code: NO_KEY,
                    message: "you have no API key yet; issue one from the portal".into(),
                    details: None,
                }),
            )
                .into_response(),
        ),
        Err(error) => {
            // `error` cannot carry a key value — see `gateway::sdk_message`.
            tracing::error!(error = %error, "portal key reveal failed");
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

/// The read-only lookup: list, filter, rank, read. Nothing here mutates.
///
/// `Ok(None)` is "no key to reveal" — including the raced case where the
/// winner was listed and deleted before its value could be read. The reveal
/// answers `no_key` for it rather than retrying into a create, because
/// retrying into a create is exactly what this route gave up.
async fn lookup(
    gateway: &Gateway,
    name: &str,
) -> Result<Option<(KeyRecord, KeyValue)>, GatewayError> {
    let candidates = exact_matches(gateway.list_named(name).await?, name);
    let Some(winner) = choose_winner(&candidates).cloned() else {
        return Ok(None);
    };
    match gateway.value_of(&winner.id).await? {
        Some(value) => Ok(Some((winner, value))),
        None => Ok(None),
    }
}

/// What the eligibility-checked issue round-trip needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IssueOutcome {
    /// A key exists, is on the free plan, and its value is readable — created
    /// now or adopted. The value is deliberately not carried: the callback
    /// that consumes this answers with a redirect, and a credential must
    /// never ride in a `Location`.
    Issued,
    /// The control plane would not produce a usable key — an error, a
    /// deadline, or a key deleted underneath every attempt. Logged inside
    /// with the same distinctions 0187's handler drew; the visitor is told to
    /// try again either way.
    Failed,
}

/// Issue (or adopt) the key for `sub` — the create-capable flow, reachable
/// only from `super::auth::issue` after an eligibility proof.
///
/// This is 0187's reconciler unchanged — list → filter → rank → create if
/// missing → attach → sweep duplicates → read — behind the same deadline its
/// handler used, so the callback cannot outlive the Lambda's budget either.
pub(crate) async fn issue_for(gateway: &Gateway, sub: &str, deadline: Duration) -> IssueOutcome {
    // Same guard as the reveal, for the same reason — and here it also stands
    // in front of `DeleteApiKey`.
    let Some(name) = key_name(sub) else {
        tracing::warn!("an issue round-trip carried a user id that is not a snowflake; refusing");
        return IssueOutcome::Failed;
    };

    match tokio::time::timeout(deadline, reconcile(gateway, &name)).await {
        Ok(Ok(Some(outcome))) => {
            tracing::info!(
                key_id = %outcome.record.id,
                created = outcome.created,
                "portal issued an API key"
            );
            IssueOutcome::Issued
        }
        // Every attempt found a key and then lost it before reading its value.
        Ok(Ok(None)) => {
            tracing::warn!(
                attempts = MAX_ATTEMPTS,
                "a key was deleted underneath every issue attempt"
            );
            IssueOutcome::Failed
        }
        Ok(Err(error)) => {
            // `error` cannot carry a key value — see `gateway::sdk_message`.
            tracing::error!(error = %error, "portal key issuance failed");
            IssueOutcome::Failed
        }
        Err(_elapsed) => {
            tracing::error!(
                deadline_secs = deadline.as_secs_f32(),
                "portal key issuance ran out of time"
            );
            IssueOutcome::Failed
        }
    }
}

/// The result of a successful reconciliation.
///
/// Deliberately does **not** carry the key value. The issue flow's caller is a
/// redirect (`auth::issue`), and a value it cannot receive is a value that
/// cannot leak into a `Location` or a log by later mistake; the reveal reads
/// the value through its own read-only [`lookup`]. The reconciler still calls
/// `value_of` where readability is what is being verified.
struct Outcome {
    record: KeyRecord,
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

    let mut created: Option<KeyRecord> = None;
    let mut candidates = existing;

    // Step 2: nothing yet — create one. The value the create answers with is
    // dropped on purpose — see `Outcome`; a successful create is itself the
    // proof the key is readable.
    if candidates.is_empty() {
        let (record, _value) = gateway.create(name).await?;

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
                created: true,
            }));
        }
        // `GetApiKeys` is eventually consistent, so the listing can come back
        // NON-empty and still not contain the key just created — another
        // invocation's key is visible, ours is not yet. Without this union the
        // record is neither ranked (it is not in `candidates`) nor deleted (it
        // is not in `losers`), and what is left behind is a live, enabled
        // production key carrying this user's exact name, attached to no plan,
        // until that user happens to come back — indefinitely if they do not.
        //
        // The branch above states its own case explicitly; this one used to fall
        // through in silence. Unioned rather than special-cased, so the key we
        // made competes under the same rule as every other candidate: it wins
        // and is handed out, or it loses and is deleted like any duplicate.
        // `DeleteApiKey` addresses by id, so a key the listing has not caught up
        // with can still be removed.
        if !candidates.iter().any(|c| c.id == record.id) {
            candidates.push(record.clone());
        }
        created = Some(record);
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

    // Step 6: readability. Free if we just created the winner; otherwise a
    // read — not to hand the value out (see `Outcome`), but to prove the key
    // the caller will be sent to reveal actually answers.
    if let Some(record) = created
        && record.id == winner.id
    {
        return Ok(Some(Outcome {
            record: winner,
            created: true,
        }));
    }

    // `None` here is the raced deletion: the winner existed when it was listed
    // and does not now. The caller re-runs the whole flow, which will adopt
    // whatever survived or create a replacement — this is the "not handed out
    // as a dead id" property.
    match gateway.value_of(&winner.id).await? {
        Some(_value) => Ok(Some(Outcome {
            record: winner,
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
