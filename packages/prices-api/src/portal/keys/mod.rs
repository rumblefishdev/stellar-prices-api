//! Reveal a key — and issue one, but only for [0189]'s eligibility-checked
//! callback (task 0187, re-shaped by task 0189).
//!
//! One route under [`PORTAL_API_PREFIX`](super::PORTAL_API_PREFIX), so it
//! inherits [0183]'s gate exactly as sign-in does: with `PORTAL_ENABLED` false
//! it is an empty `404`, byte-identical to a path that was never deployed.
//!
//! | route | does |
//! | --- | --- |
//! | `GET`/`POST /api-tokens/api/key` | reveal — the lookup, plus the value. **Never creates.** A revoked key answers `404 key_revoked` with the date a new one can be issued |
//! | `POST /api-tokens/api/key/rework` | **revoke** (task 0191): deactivate the caller's key now, issue nothing. Session-authorized, deliberately — see below |
//!
//! Issuance itself is [`issue_for`], reachable **only** from the OAuth
//! callback completing an `action=issue` round-trip
//! (`super::auth::issue`) — because issuing requires an eligibility proof
//! (Stellar Discord membership + minimum account age, ADR 0010 §8) that only a
//! fresh Discord token can provide, and a session cookie is not one. Since
//! task 0191 the issue path also enforces the **re-issue cap**: a key the
//! owner revoked inside the current quota period blocks a new one until the
//! period rolls ([`cap`]), which is what makes "replace my key" a revocation
//! rather than a free counter reset.
//!
//! # Revoke is session-only, on purpose
//!
//! The one write a session cookie can cause is `UpdateApiKey(enabled=false)`
//! on the caller's **own** key — destructive to their own access and to
//! nothing else. It is deliberately not behind a Discord round-trip: a leaked
//! key has to be killable while Discord is down, and re-proving membership
//! buys nothing for an action that issues no credential. The CSRF stance is
//! re-derived as `auth/mod.rs` requires: the route is `POST`-only and the
//! session cookie is `SameSite=Lax`, so a cross-site page cannot make the
//! browser send it with a `POST`; a `GET` on the path is a `405`. The worst
//! outcome of the write is the visitor's own key dying a month early, which
//! is exactly what the confirmation dialog made them type `delete-key` for.
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
//! what a departed member loses is the replacement after a revoke ([0191]),
//! which is an issue and re-proves membership.
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

pub mod cap;
pub mod gateway;
pub mod naming;

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Serialize;

use crate::common::{cache_control, errors};

use super::auth::secret::OauthSecret;
use super::period::Period;
use cap::Cap;
use gateway::{Attachment, Disable, Gateway, GatewayError, KeyValue};
use naming::{
    KeyRecord, choose_winner, current_key, exact_matches, key_name, losers, revocation_instant,
};

/// The reveal, on both verbs — see [`key`] for why `POST` answers identically.
pub const KEY_PATH: &str = "/api-tokens/api/key";

/// The revocation (task 0191). `POST` only — the verb is the CSRF guard, see
/// the module docs — and the path keeps the task's name for the action the
/// dashboard calls "Replace my key": the key is replaced, just not until the
/// next period.
pub const REWORK_PATH: &str = "/api-tokens/api/key/rework";

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
/// Error code for a caller whose key is revoked and whose replacement is not
/// due yet (task 0191). A `404` like `no_key` — there is no usable key — but
/// a distinct code, because the page must not offer the issue round-trip: the
/// envelope's `details` carries `next_eligible_at` (RFC 3339) and
/// `revoked_at`, the slot `ErrorEnvelope` has for exactly this kind of
/// structured context.
const KEY_REVOKED: &str = "key_revoked";
/// Error code for a revoke that did not come from the portal page (task
/// 0191). See [`is_same_origin_write`].
const CROSS_SITE_REQUEST: &str = "cross_site_request";

/// The header the portal's own `fetch` sets on the revoke, and its value.
///
/// A custom header makes the request *non-simple*: a cross-origin page can
/// only send it after a CORS preflight, and this API answers no preflight —
/// so a browser never delivers such a request from anywhere but the portal's
/// origin. This is the classic CSRF defence that does not depend on what the
/// cookie's `SameSite` considers "the same site", which matters because
/// `SameSite=Lax` is **site**-scoped: today `*.cloudfront.net` makes every
/// distribution its own site, but after the custom-domain cutover ([0195])
/// any sibling host under the registrable domain becomes same-site, and a
/// form `POST` from it would carry the cookie. One request from such a host
/// would then cost the victim their key for the rest of the month.
pub const PORTAL_REQUEST_HEADER: (&str, &str) = ("x-requested-with", "stellar-prices-portal");

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
        .route(REWORK_PATH, post(revoke))
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
        Ok(Lookup::Revoked { revoked_at }) => {
            // The owner killed this key. There is nothing to reveal — the
            // value is a leaked credential by the owner's own statement — and
            // nothing to issue until the period rolls; the page renders the
            // date rather than the "get my API key" link.
            let Cap::Capped {
                next_eligible_at, ..
            } = cap::decide(revoked_at, &Period::now())
            else {
                // Revoked in an earlier period: a new key is due, and the
                // issue round-trip will delete this one and create it. Until
                // that press the honest answer is still "no usable key".
                return no_store(no_key_response());
            };
            no_store(
                (
                    StatusCode::NOT_FOUND,
                    Json(errors::ErrorEnvelope {
                        code: KEY_REVOKED,
                        message: format!(
                            "your API key was revoked; a new one can be issued from \
                             {next_eligible_at}"
                        ),
                        details: Some(serde_json::json!({
                            "next_eligible_at": next_eligible_at,
                            "revoked_at": revoked_at.map(rfc3339),
                        })),
                    }),
                )
                    .into_response(),
            )
        }
        Ok(Lookup::Key(record, value)) => {
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
        Ok(Lookup::None) => no_store(no_key_response()),
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

/// Whether a state-changing request came from the portal's own page.
///
/// Two independent signals, both required to agree:
///
/// 1. [`PORTAL_REQUEST_HEADER`] is present with its value — the non-simple
///    request marker a cross-origin page cannot produce without a preflight.
/// 2. `Sec-Fetch-Site`, when the browser sends it (every current browser
///    does), is `same-origin`. `none` (a typed URL, a bookmark) is accepted
///    too — it cannot be a `POST` from another page. A request carrying
///    `cross-site` or `same-site` is refused even with the header: the
///    header guards against a preflight-less browser, the fetch metadata
///    guards against a misconfigured CORS answer ever appearing in front of
///    this API.
///
/// Absence of `Sec-Fetch-Site` alone does not refuse — `curl` and older
/// browsers omit it — which is why the header is the primary check.
fn is_same_origin_write(headers: &HeaderMap) -> bool {
    let marker = headers
        .get(PORTAL_REQUEST_HEADER.0)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case(PORTAL_REQUEST_HEADER.1));
    let fetch_site_ok = match headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        None => true,
        Some(site) => site.eq_ignore_ascii_case("same-origin") || site.eq_ignore_ascii_case("none"),
    };
    marker && fetch_site_ok
}

/// `404 no_key`, shared by the reveal's two keyless answers.
fn no_key_response() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(errors::ErrorEnvelope {
            code: NO_KEY,
            message: "you have no API key yet; issue one from the portal".into(),
            details: None,
        }),
    )
        .into_response()
}

/// Unix seconds → RFC 3339, for the envelopes.
fn rfc3339(secs: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs as i64, 0)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// What the revoke answers.
#[derive(Serialize)]
struct RevokeResponse {
    /// Always `true` on a `200`; a failure is a status, never a `false`.
    revoked: bool,
    /// When a new key can be issued under our period rule, RFC 3339 —
    /// **decided by [`cap::decide`]**, not by the period alone, so this route
    /// is not a reader of the cap that disagrees with the other four
    /// (decision #22). For a revocation whose period has already rolled it is
    /// now: an idempotent revoke must not tell a stale tab to wait another
    /// month for a key the issue round-trip would hand over today.
    next_eligible_at: String,
    /// When the key was revoked, RFC 3339 — now, or earlier if it already
    /// was. `null` when no record under the name carries a date: a made-up
    /// epoch instant would render as "deactivated on 1 January 1970".
    revoked_at: Option<String>,
    /// `true` when at least one key under the name could **not** be disabled
    /// while another was (task 0191's partial revocation). The page must not
    /// render a plain "revoked" for it: a duplicate that still answers on
    /// `/v1/` is exactly the state the dialog's docs forbid calling revoked.
    /// Omitted from the JSON in the ordinary case, so the common answer keeps
    /// the shape it had.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    partial: bool,
}

/// `POST /key/rework` — revoke the caller's key now, issue nothing
/// (task 0191).
///
/// **Deactivates, immediately, and that is the whole operation.** Every
/// enabled key under the caller's name is disabled (`UpdateApiKey`); the
/// answer carries the date a replacement can be issued — the 1st of the next
/// month under our period rule — and the page renders it. Nothing is created:
/// the cap on re-issue is what makes this a revocation rather than a free
/// counter reset, and the wait is the visitor's own access, which the
/// confirmation dialog made them type `delete-key` to accept.
///
/// Idempotent: a key already revoked answers `200` again with the same dates,
/// so a retry after a dropped response is safe. A caller with no key is a
/// `404 no_key`. Session-authorized — see the module docs for why that is
/// deliberate and how the `POST`-only shape stands in for a CSRF token.
///
/// | answer | meaning |
/// | --- | --- |
/// | `200 {revoked: true, next_eligible_at, revoked_at}` | the key is off (the data plane follows within tens of seconds — 0180 item 8) |
/// | `404 no_key` | nothing to revoke |
/// | `401` / `502` / `503` | as the reveal |
async fn revoke(State(state): State<KeysState>, headers: HeaderMap) -> Response {
    let Some(oauth) = state.oauth.as_ref() else {
        return unconfigured();
    };
    let Some(gateway) = state.gateway.as_ref() else {
        return unconfigured();
    };
    // Before the session is even read: the one write a session can cause
    // must come from the portal's own page. See `is_same_origin_write`.
    if !is_same_origin_write(&headers) {
        tracing::warn!("a revoke arrived without the same-origin markers; refusing");
        return no_store(
            (
                StatusCode::FORBIDDEN,
                Json(errors::ErrorEnvelope {
                    code: CROSS_SITE_REQUEST,
                    message: "this action can only be taken from the portal page".into(),
                    details: None,
                }),
            )
                .into_response(),
        );
    }
    let Some(session) = super::auth::current_session(oauth, &headers) else {
        return no_store(errors::unauthorized_with(
            NOT_SIGNED_IN,
            "sign in with Discord before asking to replace your API key",
        ));
    };
    // Same guard as the reveal — and here it stands in front of a write.
    let Some(name) = key_name(&session.sub) else {
        tracing::warn!("a session carried a user id that is not a snowflake; refusing");
        return no_store(errors::unauthorized_with(
            NOT_SIGNED_IN,
            "sign in with Discord before asking to replace your API key",
        ));
    };

    let revoked = match tokio::time::timeout(state.deadline, disable_all(gateway, &name)).await {
        Ok(revoked) => revoked,
        Err(_elapsed) => {
            tracing::error!(
                deadline_secs = state.deadline.as_secs_f32(),
                "portal key revocation ran out of time"
            );
            return no_store(errors::service_unavailable(
                KEY_UNAVAILABLE,
                "the API key service is taking too long to answer; try again",
            ));
        }
    };

    // Read before the match consumes `revoked`; both arms below answer `200`
    // with the same dates and differ only in this flag.
    let partial = matches!(&revoked, Ok(Revocation::Partial { .. }));
    match revoked {
        Ok(Revocation::Done { at }) | Ok(Revocation::Partial { at }) => {
            if partial {
                tracing::error!(
                    "portal revoked an API key only in part; a duplicate may still answer"
                );
            } else {
                tracing::info!("portal revoked an API key");
            }
            // The cached usage describes a key that is now off. Its counter is
            // preserved (0180 item 8), so the numbers would still be right —
            // but the page's state changed underneath them, and a fresh read
            // is one call.
            if let Some(cache) = &state.usage_cache {
                cache.invalidate(&session.sub);
            }
            // The same decision function the reveal, the usage route and the
            // issue path read, from the same instant — never `Period::now()`
            // straight, which would answer "next month" for a revocation that
            // happened two periods ago and hide the issue link the round-trip
            // would have honoured.
            let next_eligible_at = match cap::decide(at, &Period::now()) {
                Cap::Capped {
                    next_eligible_at, ..
                } => next_eligible_at,
                Cap::Allowed => rfc3339(now_secs()),
            };
            no_store(
                Json(RevokeResponse {
                    revoked: true,
                    next_eligible_at,
                    revoked_at: at.map(rfc3339),
                    partial,
                })
                .into_response(),
            )
        }
        Ok(Revocation::NoKey) => no_store(
            (
                StatusCode::NOT_FOUND,
                Json(errors::ErrorEnvelope {
                    code: NO_KEY,
                    message: "you have no API key to replace; issue one first".into(),
                    details: None,
                }),
            )
                .into_response(),
        ),
        Err(error) => {
            tracing::error!(error = %error, "portal key revocation failed");
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

/// Disable every key under `name`. [`Revocation::NoKey`] means there was
/// none left to disable; [`Revocation::Done`] and [`Revocation::Partial`]
/// carry the revocation instant — the control plane's own `lastUpdatedDate`
/// for a key this call disabled, the recorded one for a key that already was,
/// and `None` for the shape where no record under the name is dated.
///
/// **Every** enabled key, not only the current one: a duplicate left by an
/// earlier double-submit is a working credential the visitor was just told
/// had died.
///
/// **A failure mid-loop is logged and stepped over, not propagated** — the
/// reverse of what this function did when it shipped, and the reason is the
/// sentence at the other end of it. Propagating the first error answered
/// `502`, and the dialog's `502` copy said the key "is still active" while the
/// keys disabled before the error were off. There is no rollback for a
/// revocation (re-enabling a leaked key is the wrong direction to fail in), so
/// the state after a partial failure is real and has to be *reported* rather
/// than denied:
///
/// - at least one disable applied, none failed → [`Revocation::Done`];
/// - at least one applied **and** at least one failed → [`Revocation::Partial`],
///   which the page renders as "your key is off, a duplicate may still work" —
///   never as a plain "revoked", because that is the claim `app.tsx`'s dialog
///   docs forbid making about a key that still answers;
/// - nothing applied and something failed → the error, and the `502` stands:
///   here "still active" is the truth, and no write of ours landed.
///
/// A key that answers `NotFound` is not a failure but the deletion race, and
/// counts as neither: if every enabled key raced away there is nothing to
/// revoke and no record to date, which is [`Revocation::NoKey`] — the same
/// answer the very next `?action=issue` press will act on when it finds the
/// name empty and creates a key outright.
async fn disable_all(gateway: &Gateway, name: &str) -> Result<Revocation, GatewayError> {
    let keys = exact_matches(gateway.list_named(name).await?, name);
    let Some(current) = current_key(&keys) else {
        return Ok(Revocation::NoKey);
    };
    if !current.enabled {
        // Already revoked — answer with the recorded instant (the latest,
        // the same one the cap reads), write nothing. Undatable stays
        // `None` all the way to the JSON: the page has copy for "we cannot
        // date it", and none for the epoch.
        return Ok(Revocation::Done {
            at: revocation_instant(&keys),
        });
    }
    // The LATEST instant among the disables that landed, which is what
    // `naming::revocation_instant` will read back off the records afterwards
    // and what `cap::decide` compares against the period. Never
    // `SystemTime::now()`: that is a different clock from the control plane's,
    // and on the 1st the two can fall either side of the period boundary — the
    // page would then name a `next_eligible_at` a month before the issue
    // round-trip would honour it.
    let mut applied: Option<Option<u64>> = None;
    let mut failure: Option<GatewayError> = None;
    for key in keys.iter().filter(|k| k.enabled) {
        // The same last-line guard the reconciler keeps before its deletes:
        // on the record, immediately before the write.
        if key.name != name {
            tracing::error!(
                key_id = %key.id,
                "refusing to disable a key whose name is not the caller's; \
                 the exact-name filter has been bypassed"
            );
            continue;
        }
        match gateway.disable(&key.id).await {
            Ok(Disable::Applied(at)) => {
                tracing::info!(key_id = %key.id, "portal disabled a revoked API key");
                applied = Some(match applied {
                    Some(previous) => previous.max(at),
                    None => at,
                });
            }
            Ok(Disable::KeyGone) => {
                tracing::info!(
                    key_id = %key.id,
                    "a key listed for revocation was already gone; nothing to disable"
                );
            }
            Err(error) => {
                tracing::error!(
                    key_id = %key.id,
                    error = %error,
                    "could not disable a key during a revocation; continuing with the rest"
                );
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
    }
    match (applied, failure) {
        (Some(at), None) => Ok(Revocation::Done { at }),
        (Some(at), Some(_)) => Ok(Revocation::Partial { at }),
        // Nothing of ours landed, so the `502` is honest and the dialog's
        // "we could not deactivate it" is the right thing to say.
        (None, Some(error)) => Err(error),
        // Every enabled key raced away between the listing and the patch.
        // Not `Done`: there is no disabled record in the account, so telling
        // the visitor "deactivated, next eligible on the 1st" would be
        // contradicted by the next issue press, which finds the name empty
        // and creates a key on the spot.
        (None, None) => Ok(Revocation::NoKey),
    }
}

/// What [`disable_all`] found and did.
enum Revocation {
    /// Nothing under the name left to revoke.
    NoKey,
    /// Every key is off. `at` is the revocation instant, `None` only when no
    /// record under the name carries a `lastUpdatedDate`.
    Done { at: Option<u64> },
    /// Some keys are off and at least one could not be disabled — so a
    /// credential under this name may still answer on `/v1/`. `at` dates the
    /// disables that did land, and the cap is decided from it exactly as for
    /// [`Self::Done`]: the owner is capped either way, because a revocation
    /// they cannot fully complete must not also cost them the refusal.
    Partial { at: Option<u64> },
}

/// Seconds since the Unix epoch, saturating.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// What the read-only lookup found.
enum Lookup {
    /// A live key and its value.
    Key(KeyRecord, KeyValue),
    /// The owner's key, revoked — no value is read for it. Carries the
    /// instant the cap is decided from (`naming::revocation_instant`, the
    /// latest revocation), which is what the issue path reads too.
    Revoked { revoked_at: Option<u64> },
    /// Nothing to reveal.
    None,
}

/// The read-only lookup: list, filter, rank, read. Nothing here mutates.
///
/// `Lookup::None` is "no key to reveal" — including the raced case where the
/// winner was listed and deleted before its value could be read. The reveal
/// answers `no_key` for it rather than retrying into a create, because
/// retrying into a create is exactly what this route gave up.
async fn lookup(gateway: &Gateway, name: &str) -> Result<Lookup, GatewayError> {
    let candidates = exact_matches(gateway.list_named(name).await?, name);
    let Some(current) = current_key(&candidates).cloned() else {
        return Ok(Lookup::None);
    };
    if !current.enabled {
        return Ok(Lookup::Revoked {
            revoked_at: revocation_instant(&candidates),
        });
    }
    match gateway.value_of(&current.id).await? {
        Some(value) => Ok(Lookup::Key(current, value)),
        None => Ok(Lookup::None),
    }
}

/// What the eligibility-checked issue round-trip needs to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IssueOutcome {
    /// A key exists, is on the free plan, and its value is readable — created
    /// now or adopted. The value is deliberately not carried: the callback
    /// that consumes this answers with a redirect, and a credential must
    /// never ride in a `Location`.
    Issued,
    /// The owner revoked their key inside the current quota period, so no
    /// new one is issued until it rolls (task 0191). Nothing was written.
    Capped {
        /// `YYYY-MM-DD`, for the landing query.
        next_eligible_date: String,
    },
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

    let deadline_at = std::time::Instant::now() + deadline;
    match tokio::time::timeout(deadline, reconcile(gateway, &name, deadline_at)).await {
        Ok(Ok(Reconciled::Issued(outcome))) => {
            tracing::info!(
                key_id = %outcome.record.id,
                created = outcome.created,
                "portal issued an API key"
            );
            IssueOutcome::Issued
        }
        Ok(Ok(Reconciled::Capped { next_eligible_date })) => {
            tracing::info!(
                outcome = "capped",
                "portal issue refused: the key was revoked this quota period"
            );
            IssueOutcome::Capped { next_eligible_date }
        }
        Ok(Ok(Reconciled::OutOfTime)) => {
            tracing::error!(
                deadline_secs = deadline.as_secs_f32(),
                "portal key issuance refused to start a create it could not finish"
            );
            IssueOutcome::Failed
        }
        // Every attempt found a key and then lost it before reading its value.
        Ok(Ok(Reconciled::Lost)) => {
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

/// What the reconciler settled on.
enum Reconciled {
    Issued(Outcome),
    /// A revoked key inside the current period stands in the way.
    Capped {
        next_eligible_date: String,
    },
    /// Every attempt settled on a key that was deleted before its value could
    /// be read.
    Lost,
    /// Too little time left to create and attach. Nothing was written.
    OutOfTime,
}

/// One attempt's verdict; `Retry` re-enters the flow.
enum Attempt {
    Done(Outcome),
    Capped {
        next_eligible_date: String,
    },
    /// Too little of the deadline left to create and attach — nothing was
    /// written. Not retried: the next attempt would have even less.
    OutOfTime,
    Retry,
}

/// The least time a create is started with. `CreateApiKey` and
/// `CreateUsagePlanKey` each get up to `gateway::OPERATION_TIMEOUT` (5s) in
/// the worst case; in practice each is a few hundred milliseconds. 4s is
/// enough for both at ordinary latency and refuses to start them when the
/// invocation is about to be killed — which is the one way this flow can
/// leave an enabled, unattached key behind.
///
/// Above `auth::issue::RECONCILE_FLOOR` (2s) on purpose: between the two a
/// reconciliation can still adopt an existing key but will not start a
/// create. The note on that constant is where the trade-off is argued.
pub(crate) const CREATE_FLOOR: Duration = Duration::from_secs(4);

/// List, filter, rank, converge — the reconciler, run up to [`MAX_ATTEMPTS`]
/// times.
///
/// `Lost` means every attempt settled on a key that was deleted before its
/// value could be read. That is a real, transient state (a console session, or
/// another invocation's reconciler) and it is reported as such rather than as a
/// failure of this service.
async fn reconcile(
    gateway: &Gateway,
    name: &str,
    deadline: std::time::Instant,
) -> Result<Reconciled, GatewayError> {
    for _ in 0..MAX_ATTEMPTS {
        match attempt(gateway, name, deadline).await? {
            Attempt::Done(outcome) => return Ok(Reconciled::Issued(outcome)),
            Attempt::Capped { next_eligible_date } => {
                return Ok(Reconciled::Capped { next_eligible_date });
            }
            Attempt::OutOfTime => return Ok(Reconciled::OutOfTime),
            Attempt::Retry => {}
        }
    }
    Ok(Reconciled::Lost)
}

/// One pass of the reconciler. `deadline` is the instant the caller's
/// `timeout` fires; a create is not started without room for itself.
async fn attempt(
    gateway: &Gateway,
    name: &str,
    deadline: std::time::Instant,
) -> Result<Attempt, GatewayError> {
    // Step 1: everything the control plane has under this name PREFIX, across
    // all pages, narrowed to exact equality here. Both halves matter and both
    // are argued for in `naming`.
    let existing = exact_matches(gateway.list_named(name).await?, name);

    // Step 1b (task 0191): a revoked key. If every key under the name is
    // disabled, the owner revoked it, and the re-issue cap decides: inside the
    // period of the revocation → refused with the date, nothing written;
    // after it → the revocation records are deleted and the flow continues
    // into a create, exactly as for a first issue. Deleting BEFORE creating
    // is safe here, unlike a swap, because a disabled key is not a credential
    // anybody can lose; and it keeps the next listing from ranking a dead key
    // ahead of the live one.
    let (live, mut revoked): (Vec<KeyRecord>, Vec<KeyRecord>) =
        existing.into_iter().partition(|k| k.enabled);
    // Ids this attempt deleted. `GetApiKeys` is eventually consistent, so the
    // re-listing after a create can still show them; ranked, a deleted
    // earlier-created record wins, its attach 404s, and the single retry is
    // spent on a phantom — leaving the key just created unattached. Filtered
    // out of the re-listing instead, along with anything disabled.
    let mut deleted_here: Vec<String> = Vec::new();
    if live.is_empty() && !revoked.is_empty() {
        // The LATEST revocation governs: a duplicate revoked last month must
        // not open a door a revocation this month closed.
        let revoked_at = revocation_instant(&revoked);
        if let Cap::Capped {
            next_eligible_date, ..
        } = cap::decide(revoked_at, &Period::now())
        {
            return Ok(Attempt::Capped { next_eligible_date });
        }
        // Logged and stepped over, NOT propagated — the same rule, for the
        // same reason, as the loser sweep at the end of this function. A `?`
        // here makes a delete that cannot succeed permanent: this branch is
        // reached on EVERY press once the name holds nothing but revoked keys,
        // so one undeletable record (an untagged exact-name key made by hand in
        // the console, against the tag condition task 0194 may add to `DELETE`)
        // would answer `?issue=failed` forever, with no in-product recovery.
        // Housekeeping must not be able to withhold the key the request is for.
        //
        // The create below is safe with the record still there: the re-listing
        // drops everything disabled, so a survivor is neither ranked nor
        // adopted, and the sweep at the end tries it again.
        let mut undeleted: Vec<KeyRecord> = Vec::new();
        for dead in &revoked {
            if dead.name != name {
                tracing::error!(
                    key_id = %dead.id,
                    "refusing to delete a key whose name is not the caller's; \
                     the exact-name filter has been bypassed"
                );
                continue;
            }
            tracing::info!(key_id = %dead.id, "deleting a revoked key whose period has rolled");
            match gateway.delete(&dead.id).await {
                Ok(()) => deleted_here.push(dead.id.clone()),
                Err(error) => {
                    tracing::error!(
                        key_id = %dead.id,
                        error = %error,
                        "could not delete a revoked key whose period has rolled; continuing \
                         into the create and leaving it for the sweep"
                    );
                    undeleted.push(dead.clone());
                }
            }
        }
        // Whatever was deleted here is gone, so the sweep below has nothing of
        // theirs left to do; whatever failed stays in the list for it to retry.
        revoked = undeleted;
    }

    let mut created: Option<KeyRecord> = None;
    // Live keys only: a revoked key that survived the branch above (one live
    // key beside it — a console re-enable, or a duplicate) is neither ranked
    // nor adopted; it is swept like any other loser below.
    let mut candidates = live;

    // Step 2: nothing yet — create one. The value the create answers with is
    // dropped on purpose — see `Outcome`; a successful create is itself the
    // proof the key is readable.
    if candidates.is_empty() {
        // A create is the one write that cannot be undone by a timeout: if
        // the deadline cancels this future after the request left, the key
        // exists, attached to nothing, revealed to its owner as a key that
        // answers `403`. So the create is started only with room for itself
        // and the attach behind it — see `CREATE_FLOOR`.
        if deadline.saturating_duration_since(std::time::Instant::now()) < CREATE_FLOOR {
            tracing::error!(
                "not enough of the deadline left to create AND attach a key; refusing to \
                 start a create that could be cut off"
            );
            return Ok(Attempt::OutOfTime);
        }
        let (record, _value) = gateway.create(name).await?;

        // Re-list rather than returning what we just made. Two simultaneous
        // first presses both find nothing and both create, so the list is the
        // only place the collision is visible — and this is where the
        // acceptance criterion "two concurrent presses converge on one key, and
        // the loser is deleted" is actually met. The rank rule is deterministic
        // (`naming::choose_winner`), so both invocations pick the same survivor
        // from the same list rather than each deleting the other's.
        candidates = exact_matches(gateway.list_named(name).await?, name)
            .into_iter()
            .filter(|k| k.enabled && !deleted_here.contains(&k.id))
            .collect();
        if candidates.is_empty() {
            // The control plane did not list what it just created. Rather than
            // fail, trust the create — it answered with an id and a value, and
            // the next request will reconcile whatever this leaves behind.
            // `KeyGone` here means the key we just made was deleted between
            // the create and this call. Handing its value out would be handing
            // out a dead id — the one thing the adopt-or-recreate rule exists to
            // prevent — so this re-enters the flow like any other lost race.
            if gateway.attach_to_free_plan(&record.id).await? == Attachment::KeyGone {
                return Ok(Attempt::Retry);
            }
            return Ok(Attempt::Done(Outcome {
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
        return Ok(Attempt::Retry);
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
        return Ok(Attempt::Retry);
    }

    // Step 5: everything that is not the winner is deleted — duplicates, and
    // (task 0191) any revoked key left beside a live one.
    for loser in losers(&candidates, &winner)
        .into_iter()
        .chain(revoked.iter())
    {
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
        return Ok(Attempt::Done(Outcome {
            record: winner,
            created: true,
        }));
    }

    // `None` here is the raced deletion: the winner existed when it was listed
    // and does not now. The caller re-runs the whole flow, which will adopt
    // whatever survived or create a replacement — this is the "not handed out
    // as a dead id" property.
    match gateway.value_of(&winner.id).await? {
        Some(_value) => Ok(Attempt::Done(Outcome {
            record: winner,
            created: false,
        })),
        None => Ok(Attempt::Retry),
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

    /// The revoke route is one segment deeper — `key/rework` — which is
    /// still inside the depth the currently-deployed gateway maps (0205's
    /// note: depth 1–2 under the prefix work today, depth 3 answers `403`
    /// until the greedy proxy deploys). Pinned so it is not moved deeper.
    #[test]
    fn the_rework_route_is_under_the_key_route_and_no_deeper() {
        assert!(REWORK_PATH.starts_with(super::super::PORTAL_API_PREFIX));
        assert_eq!(REWORK_PATH, format!("{KEY_PATH}/rework"));
        let rest = REWORK_PATH.trim_start_matches(super::super::PORTAL_API_PREFIX);
        assert_eq!(rest.split('/').count(), 2);
    }
}
