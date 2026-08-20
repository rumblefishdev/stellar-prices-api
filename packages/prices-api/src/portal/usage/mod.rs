//! Usage against quota, rendered honestly (task 0188).
//!
//! One route under [`PORTAL_API_PREFIX`](super::PORTAL_API_PREFIX), so it
//! inherits [0183]'s gate exactly as sign-in and the key routes do: with
//! `PORTAL_ENABLED` false it is an empty `404`, byte-identical to a path that
//! was never deployed.
//!
//! | route | does |
//! | --- | --- |
//! | `GET /api-tokens/api/usage` | the caller's used / remaining / limit for the current period, from `GetUsage` |
//!
//! # This route is read-only, and that is a safety property
//!
//! The key id comes from the same list → exact-filter → rank flow as [0187]'s
//! reveal — API Gateway is the source of truth, there is no registry row to
//! look up — but it stops there. **No create, no attach, no delete.** [0187]'s
//! decision 14 keeps issuance behind an explicit press; a dashboard that ran
//! the full reconciliation on load would mint a production key for anyone who
//! merely opened the page. A caller with no key is told so (`404 no_key`), and
//! duplicates are ranked with the same deterministic rule the reveal uses —
//! so the usage shown belongs to the key the reveal would hand out — but the
//! losers are left for the next issue/reveal to sweep.
//!
//! The CSRF stance is re-derived rather than inherited, as `auth/mod.rs`
//! requires of every route on this prefix: `SameSite=Lax` does send the session
//! cookie on a top-level `GET` navigation here, and it buys an attacker
//! nothing — the handler changes no state at all, and the JSON it renders in
//! the victim's own tab is not readable cross-origin. The worst outcome is the
//! visitor seeing their own dashboard.
//!
//! # The numbers are AWS's; the period boundary is ours
//!
//! `used`, `remaining` and the reconstructed `limit` come from `GetUsage`,
//! scoped to `(usagePlanId, apiKeyId)` — no accounting of our own. The period
//! rendered around them does **not** come from AWS, because AWS documents
//! neither the reset instant nor its timezone (ADR 0010, correction #2, still
//! open — the only statement anywhere is an example caption). "The 1st of the
//! month, 00:00 UTC" is **our stated product rule**, the same one the rework
//! cap in [0191] is defined by.
//!
//! If AWS's counter turns out to roll at a different instant, the LABEL is a UX
//! wrinkle to word around — but the NUMBERS under it are not, and that is worth
//! being precise about. The query runs from our calendar 1st while `remaining`
//! is the last day's balance, so a range spanning two of AWS's periods would sum
//! last period's traffic into `used` and reconstruct `limit = used + remaining`
//! above the real quota: a 100 000 plan rendered as "150 000". `summarize_days`
//! in `keys/gateway.rs` is what keeps that from reaching the page — it spots the
//! reset (a `remaining` that rises can only be one) and counts from it, so both
//! figures come out of a single AWS period whatever instant that period began.
//! It also logs the sighting, which is the only evidence this system can produce
//! about the instant ADR 0010 is still open on.
//!
//! # `GetUsage` lags, and the response says so
//!
//! Measured 2026-08-12 (archived `0180/notes/R-apigw-namequery-quota-and-disable.md`):
//! the reported counters can trail enforcement by minutes — a key that just
//! served 80 requests read `[0, 100000]` on the day. So every answer carries
//! `as_of`, the instant the `GetUsage` behind it was actually made, and the
//! page renders it as a "last updated" line. **The wording is decided here,
//! once** (see the frontend), and [0193] restyles it without re-deciding it: a
//! dashboard that admits a lag beats one that looks broken.
//!
//! # The cache, and why it exists
//!
//! These are control-plane calls, throttled per **account** — the same budget
//! our CDK deploys draw on — and gateway caching is forbidden on portal methods
//! (a shared cache entry on an authenticated route serves one visitor another
//! visitor's data; [0187]). So the caching lives here, in process, keyed by the
//! caller: a fresh entry short-circuits both the key lookup and the `GetUsage`,
//! which is what keeps a dashboard refresh loop from competing with CI. When
//! AWS throttles anyway, the last good answer is served with its **original**
//! `as_of` — stale and honest — rather than an error page.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::{Datelike, NaiveDate, SecondsFormat, Utc};
use serde::Serialize;

use crate::common::{cache_control, errors};

use super::auth::secret::OauthSecret;
use super::keys::gateway::{Gateway, GatewayError};
use super::keys::naming::{choose_winner, exact_matches, key_name};

/// The one route. `GET` only — reading a counter must not share a path shape
/// with anything that writes.
pub const USAGE_PATH: &str = "/api-tokens/api/usage";

/// Error code for a caller with no valid session. Same constant as the key
/// routes use, for the same audience.
const NOT_SIGNED_IN: &str = "not_signed_in";
/// Error code for a signed-in caller who has no key to have usage for.
const NO_KEY: &str = "no_key";
/// Error code for a control plane that would not answer.
const USAGE_UNAVAILABLE: &str = "usage_unavailable";
/// Error code for a deployment with the portal open and no usage plan wired.
const USAGE_UNCONFIGURED: &str = "usage_unconfigured";

/// How long a cached answer is served without asking AWS again.
///
/// One dashboard load is one `GetApiKeys` + one `GetUsage`; within this window
/// every further load by the same caller is neither. 60 seconds is far inside
/// `GetUsage`'s own reporting lag (minutes — see the module docs), so the
/// cache costs the viewer no freshness AWS was offering, while a refresh
/// loop at any human rate collapses to one control-plane call a minute.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// How long an expired entry is kept for the throttle fallback before it is
/// pruned.
///
/// An entry past [`CACHE_TTL`] is not served normally, but while AWS is
/// rate-limiting us it is the best answer available — and the response's
/// `as_of` says exactly how old it is, so serving it is honest. Fifteen
/// minutes bounds both the staleness of that fallback and the map's memory:
/// a warm Lambda accumulates one entry per signed-in viewer, and pruning on
/// insert keeps that set to callers seen recently rather than ever.
const STALE_KEEP: Duration = Duration::from_secs(15 * 60);

/// Wall-clock ceiling on the whole lookup, both control-plane calls included.
///
/// The same shape and the same 10s as the key routes' `RECONCILE_DEADLINE`,
/// for the same reason: the per-call timeouts do not compose into a
/// request-level bound (`list_named` and `usage_of` each page with a budget
/// per call), and the alternative to answering `503` is Lambda killing the
/// invocation with no response at all.
const USAGE_DEADLINE: Duration = Duration::from_secs(10);

/// What the usage route needs, cloned per request. The `Arc`s are shared
/// across clones, which is what makes the cache one cache.
#[derive(Clone)]
pub struct UsageState {
    /// Verifies the session cookie — the same secret sign-in issued it with.
    oauth: Option<Arc<OauthSecret>>,
    /// The control-plane client, carrying the free plan id. `None` while the
    /// portal is closed, exactly as `KeysState` holds it.
    gateway: Option<Arc<Gateway>>,
    /// The last good answer per caller (session `sub`), plus the per-caller
    /// eviction epochs. See the module docs and [`CacheInner`].
    cache: Arc<Mutex<CacheInner>>,
    /// [`CACHE_TTL`], overridable only outside the Lambda build.
    ttl: Duration,
    /// [`USAGE_DEADLINE`], overridable only outside the Lambda build.
    deadline: Duration,
}

/// The cache map and the guard that keeps eviction effective under
/// concurrency.
///
/// `epochs` exists for one race: a `GET /usage` snapshots a keyless listing,
/// the same user's `POST /key` completes and calls
/// [`UsageCache::invalidate_no_key`] — a no-op, nothing is cached yet — and
/// the usage request then finishes and would write its now-false `NoKey` with
/// a fresh timestamp, resurrecting for a full TTL exactly the stale answer the
/// eviction was for. So an eviction also bumps the caller's epoch, and a
/// `NoKey` computed under an older epoch is **discarded instead of stored**
/// ([`remember`]). Real usage answers are exempt: a listing that saw the key
/// cannot be falsified by the key coming into existence.
#[derive(Default)]
struct CacheInner {
    entries: HashMap<String, CacheEntry>,
    epochs: HashMap<String, EpochMark>,
}

/// One caller's eviction epoch. `bumped_at` exists only so stale marks can be
/// pruned with the entries — a mark must outlive the in-flight lookup it
/// guards against (bounded by [`USAGE_DEADLINE`]), and [`STALE_KEEP`] is
/// comfortably past that.
#[derive(Clone, Copy)]
struct EpochMark {
    value: u64,
    bumped_at: Instant,
}

/// A handle to the usage cache for the one writer outside this module: the
/// key routes.
///
/// A successful issue makes a cached "no key" answer false — and without this,
/// provably wrong for a whole [`CACHE_TTL`]: the page's own refetch after the
/// press, and any reload inside the window, would be served the stale `NoKey`
/// and tell a key-holder they have no key. The handle can evict **only** that
/// answer, nothing else: real usage entries stay cached (a reveal changes no
/// counter), and nothing outside this module can read or write anything.
#[derive(Clone)]
pub struct UsageCache(Arc<Mutex<CacheInner>>);

impl UsageCache {
    /// Drop a cached "no key" answer for `sub`, if that is what is cached —
    /// and bump the caller's epoch either way, so an in-flight lookup that
    /// snapshotted the keyless state cannot write it back afterwards (see
    /// [`CacheInner`]). The unconditional bump is the point: at the moment the
    /// race matters there is nothing cached to remove.
    pub fn invalidate_no_key(&self, sub: &str) {
        let mut cache = self.0.lock().expect("the usage cache lock is not poisoned");
        if let Some(entry) = cache.entries.get(sub)
            && matches!(entry.answer, CachedAnswer::NoKey)
        {
            cache.entries.remove(sub);
        }
        let mark = cache.epochs.entry(sub.to_string()).or_insert(EpochMark {
            value: 0,
            bumped_at: Instant::now(),
        });
        mark.value = mark.value.wrapping_add(1);
        mark.bumped_at = Instant::now();
    }
}

impl UsageState {
    pub fn new(oauth: Option<OauthSecret>, gateway: Option<Gateway>) -> Self {
        Self {
            oauth: oauth.map(Arc::new),
            gateway: gateway.map(Arc::new),
            cache: Arc::new(Mutex::new(CacheInner::default())),
            ttl: CACHE_TTL,
            deadline: USAGE_DEADLINE,
        }
    }

    /// Shorten the cache TTL, so a test can drive expiry without waiting a
    /// minute for it.
    ///
    /// **Compiled out of the Lambda**, like `KeysState::with_deadline`: the TTL
    /// is what keeps a refresh loop off the account's control-plane budget, and
    /// a deployed build must contain no way to set it to something that does
    /// not.
    #[cfg(not(feature = "lambda"))]
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Shorten the deadline, for the timeout test. Same stance as above.
    #[cfg(not(feature = "lambda"))]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// The handle the key routes hold — see [`UsageCache`].
    pub fn cache_handle(&self) -> UsageCache {
        UsageCache(self.cache.clone())
    }
}

/// The route, as a `Router` carrying its own state.
pub fn routes(state: UsageState) -> Router {
    Router::new()
        .route(USAGE_PATH, get(usage))
        .with_state(state)
}

/// What the route answers with. Everything the dashboard renders, nothing it
/// has to compute.
///
/// The three counters are one `Option` each and go absent **together**: when
/// AWS has no row for the key yet (see `Gateway::usage_of` — common for a key
/// issued minutes ago), inventing `used: 0` would be defensible but inventing
/// `remaining` and `limit` would not, and a response that is honest about two
/// fields and guessing on the third is worse than one that says "nothing
/// recorded yet". The period and `as_of` are always present — they are ours.
#[derive(Clone, Serialize)]
struct UsageResponse {
    /// Requests counted against the quota this period, per AWS.
    used: Option<u64>,
    /// Requests left, as of the latest day AWS has data for.
    remaining: Option<u64>,
    /// The plan quota, reconstructed as `used + remaining` — `GetUsage` does
    /// not report it directly, and reading it from `GetUsagePlan` would cost a
    /// grant this slice deliberately does not take.
    limit: Option<u64>,
    /// First day of the current period, `YYYY-MM-DD` — ours: the calendar
    /// month, UTC.
    period_start: String,
    /// Last day of the current period, inclusive, `YYYY-MM-DD`.
    period_end: String,
    /// When the quota resets under our stated rule: the 1st of the next month,
    /// 00:00 UTC, as an RFC 3339 instant.
    resets_at: String,
    /// When the `GetUsage` behind this answer was made, RFC 3339. The "last
    /// updated" line renders this — for a cached or stale-served answer it is
    /// the fetch time, not now, which is the point.
    as_of: String,
}

/// What the cache remembers for one caller.
///
/// "No key" is cached alongside real answers, deliberately: the lookup for a
/// keyless caller costs the same `GetApiKeys` as anyone else's, and a keyless
/// caller pressing refresh is the same loop as anyone else pressing refresh.
#[derive(Clone)]
enum CachedAnswer {
    Usage(UsageResponse),
    NoKey,
}

#[derive(Clone)]
struct CacheEntry {
    answer: CachedAnswer,
    fetched_at: Instant,
}

/// The whole route: authenticate, consult the cache, look the key up, ask AWS,
/// answer.
async fn usage(State(state): State<UsageState>, headers: HeaderMap) -> Response {
    let Some(oauth) = state.oauth.as_ref() else {
        return unconfigured();
    };
    let Some(gateway) = state.gateway.as_ref() else {
        return unconfigured();
    };

    let Some(session) = super::auth::current_session(oauth, &headers) else {
        return no_store(errors::unauthorized_with(
            NOT_SIGNED_IN,
            "sign in with Discord to see your usage",
        ));
    };

    // Same last line as the key routes, same reason: a `sub` that is not a
    // snowflake cannot become a `nameQuery`.
    let Some(name) = key_name(&session.sub) else {
        tracing::warn!("a session carried a user id that is not a snowflake; refusing");
        return no_store(errors::unauthorized_with(
            NOT_SIGNED_IN,
            "sign in with Discord to see your usage",
        ));
    };

    // Computed once per request and used to validate cache entries as well as
    // to build the query: an entry answering for a different period_start is
    // last month's answer wearing this month's label, and the minute after a
    // month boundary is exactly when a viewer checks whether the reset
    // happened.
    let period_now = current_period(Utc::now().date_naive());

    // Fresh cache hit: no control-plane call of any kind. This is the
    // "repeated dashboard loads do not produce one GetUsage call each"
    // acceptance criterion, in one branch.
    if let Some(entry) = cached(&state, &session.sub, state.ttl, &period_now.start) {
        return answer(entry.answer);
    }

    // Read BEFORE the lookup starts: if a concurrent key issue bumps it while
    // the fetch is in flight, the fetch's "no key" snapshot is stale and must
    // not be stored. See `CacheInner`.
    let epoch = epoch_of(&state, &session.sub);

    let fetched = match tokio::time::timeout(state.deadline, fetch(gateway, &name)).await {
        Ok(fetched) => fetched,
        Err(_elapsed) => {
            // A throttled control plane often shows up HERE, not as
            // `Throttled`: the SDK's retries inside each per-call budget can
            // spend the whole deadline before any error surfaces, and the
            // timeout cancels the future first. Same condition, different
            // timing — so it gets the same answer as the throttle arm below:
            // the last good answer (re-stamped, so the next TTL of loads
            // leaves the struggling control plane alone) beats the error page.
            if let Some(entry) = cached(&state, &session.sub, STALE_KEEP, &period_now.start) {
                tracing::warn!(
                    deadline_secs = state.deadline.as_secs_f32(),
                    "portal usage lookup ran out of time; serving the cached answer"
                );
                remember(&state, &session.sub, entry.answer.clone(), epoch);
                return answer(entry.answer);
            }
            tracing::error!(
                deadline_secs = state.deadline.as_secs_f32(),
                "portal usage lookup ran out of time and nothing is cached"
            );
            return no_store(errors::service_unavailable(
                USAGE_UNAVAILABLE,
                "the usage service is taking too long to answer; try again",
            ));
        }
    };

    match fetched {
        Ok(answer_now) => {
            remember(&state, &session.sub, answer_now.clone(), epoch);
            answer(answer_now)
        }
        // Backing off (the SDK's own retries) was not enough: AWS is
        // rate-limiting the control plane. The last good answer — fresh or
        // not — beats an error page, because `as_of` makes its age visible.
        // With nothing cached there is nothing honest to show, so say what is
        // happening and invite a retry; the entry the next success writes ends
        // the condition.
        Err(GatewayError::Throttled { operation }) => {
            if let Some(entry) = cached(&state, &session.sub, STALE_KEEP, &period_now.start) {
                tracing::warn!(
                    operation,
                    "control plane is throttling; serving the cached usage answer"
                );
                // Re-stamped, so the NEXT load inside the TTL is a cache hit
                // that never reaches AWS — without this, every viewer refresh
                // during a throttle event still fired both control-plane calls
                // (SDK retries included) just to be told to slow down again,
                // which is the opposite of backing off. The answer keeps its
                // original `as_of`, so the page keeps dating it honestly; the
                // cost is that a throttle outlasting `STALE_KEEP` can keep
                // re-serving an older answer — accepted, because the honest
                // age is on screen and the alternative is the error page this
                // criterion exists to prevent.
                remember(&state, &session.sub, entry.answer.clone(), epoch);
                return answer(entry.answer);
            }
            tracing::warn!(
                operation,
                "control plane is throttling and nothing is cached"
            );
            no_store(errors::service_unavailable(
                USAGE_UNAVAILABLE,
                "AWS is rate-limiting the usage lookup right now; try again in a moment",
            ))
        }
        Err(error) => {
            tracing::error!(error = %error, "portal usage lookup failed");
            no_store(
                (
                    StatusCode::BAD_GATEWAY,
                    Json(errors::ErrorEnvelope {
                        code: USAGE_UNAVAILABLE,
                        message: "could not reach the usage service; try again".into(),
                        details: None,
                    }),
                )
                    .into_response(),
            )
        }
    }
}

/// Look the key up (read-only) and read its usage.
async fn fetch(gateway: &Gateway, name: &str) -> Result<CachedAnswer, GatewayError> {
    // The same list → exact filter → rank as the reveal, so the usage shown is
    // the usage of the key the reveal hands out — and nothing more: no create,
    // no attach, no delete. See the module docs.
    let candidates = exact_matches(gateway.list_named(name).await?, name);
    let Some(winner) = choose_winner(&candidates) else {
        return Ok(CachedAnswer::NoKey);
    };

    let today = Utc::now().date_naive();
    let period = current_period(today);
    // The QUERY ends today, not at the month's last day. The rendered
    // period_end stays the month boundary — that is our rule — but whether
    // the live control plane accepts a future `endDate` has never been
    // verified (the mock accepts any string), days after today can carry no
    // data anyway, and a rejected query here would turn every dashboard load
    // into a 502 on the first deployed run.
    let query_end = today.format("%Y-%m-%d").to_string();
    let usage = gateway
        .usage_of(&winner.id, &period.start, &query_end)
        .await?;

    // Stamped when the call was actually made, not when the answer is served —
    // a cached or stale-served response keeps this value, which is what makes
    // the "last updated" line truthful.
    let as_of = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

    Ok(CachedAnswer::Usage(UsageResponse {
        used: usage.map(|u| u.used),
        remaining: usage.map(|u| u.remaining),
        limit: usage.map(|u| u.limit()),
        period_start: period.start,
        period_end: period.end,
        resets_at: period.resets_at,
        as_of,
    }))
}

/// The current quota period under **our** rule: the calendar month, UTC.
///
/// Pure, so the December rollover and the leap year are decidable by a unit
/// test rather than by waiting for one. AWS is deliberately not consulted —
/// see the module docs.
struct Period {
    /// `YYYY-MM-DD`, the 1st of the current month.
    start: String,
    /// `YYYY-MM-DD`, the last day of the current month (inclusive, which is
    /// what `GetUsage`'s `endDate` expects).
    end: String,
    /// RFC 3339, the 1st of the next month at 00:00 UTC.
    resets_at: String,
}

fn current_period(today: NaiveDate) -> Period {
    let start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
        .expect("the 1st of a real month exists");
    let next_first = if today.month() == 12 {
        NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1)
    }
    .expect("the 1st of the following month exists");
    let end = next_first
        .pred_opt()
        .expect("the day before the 1st exists");

    Period {
        start: start.format("%Y-%m-%d").to_string(),
        end: end.format("%Y-%m-%d").to_string(),
        resets_at: format!("{}T00:00:00Z", next_first.format("%Y-%m-%d")),
    }
}

/// The cached entry for `sub`, if it is younger than `max_age` **and answers
/// for the current period**.
fn cached(
    state: &UsageState,
    sub: &str,
    max_age: Duration,
    current_period_start: &str,
) -> Option<CacheEntry> {
    let cache = state
        .cache
        .lock()
        .expect("the usage cache lock is not poisoned");
    cache
        .entries
        .get(sub)
        .filter(|entry| entry.fetched_at.elapsed() < max_age)
        .filter(|entry| answers_for_period(&entry.answer, current_period_start))
        .cloned()
}

/// The caller's current eviction epoch — read before a lookup starts, and
/// compared by [`remember`] before a "no key" is stored. See [`CacheInner`].
fn epoch_of(state: &UsageState, sub: &str) -> u64 {
    let cache = state
        .cache
        .lock()
        .expect("the usage cache lock is not poisoned");
    cache.epochs.get(sub).map(|mark| mark.value).unwrap_or(0)
}

/// Whether a cached answer still describes the current period.
///
/// An entry cached before midnight on the last of the month and served after
/// it would render last month's `period_start`/`period_end` and a `resets_at`
/// already in the past, labelled "this period" — a minute a month under the
/// TTL, up to [`STALE_KEEP`] under the throttle fallback, and precisely when a
/// viewer looks to see whether the reset happened. "No key" carries no period
/// and stays valid across the boundary.
fn answers_for_period(answer: &CachedAnswer, current_period_start: &str) -> bool {
    match answer {
        CachedAnswer::Usage(body) => body.period_start == current_period_start,
        CachedAnswer::NoKey => true,
    }
}

/// Store an answer, pruning entries (and epoch marks) too old even for the
/// throttle fallback.
///
/// `epoch` is the caller's epoch **as read before the lookup started**. A
/// "no key" computed under an epoch that has since moved is discarded rather
/// than stored: the move means a key issue completed while the lookup was in
/// flight, so the snapshot is stale and storing it would resurrect the exact
/// answer the eviction removed — for a full TTL, across reloads. The response
/// already in flight still says "no key" once; the point is that nothing
/// remembers it. Real usage answers are stored regardless — see
/// [`CacheInner`].
///
/// Pruning on insert bounds both maps by activity: they hold at most the
/// callers seen in the last [`STALE_KEEP`], which for this portal is a small
/// number — and a warm Lambda container is the only place they live long
/// enough to matter.
fn remember(state: &UsageState, sub: &str, answer: CachedAnswer, epoch: u64) {
    let mut cache = state
        .cache
        .lock()
        .expect("the usage cache lock is not poisoned");
    if matches!(answer, CachedAnswer::NoKey)
        && cache.epochs.get(sub).map(|mark| mark.value).unwrap_or(0) != epoch
    {
        tracing::debug!("a key was issued while this lookup ran; not caching its 'no key'");
        return;
    }
    cache
        .entries
        .retain(|_, entry| entry.fetched_at.elapsed() < STALE_KEEP);
    cache
        .epochs
        .retain(|_, mark| mark.bumped_at.elapsed() < STALE_KEEP);
    cache.entries.insert(
        sub.to_string(),
        CacheEntry {
            answer,
            fetched_at: Instant::now(),
        },
    );
}

/// Serialize a cached answer, `no-store` attached.
fn answer(cached: CachedAnswer) -> Response {
    match cached {
        CachedAnswer::Usage(body) => no_store(Json(body).into_response()),
        // A real portal `404` with the JSON envelope — deliberately
        // distinguishable from the gate's empty one: the portal is open, the
        // caller is signed in, and the honest answer is "you have no key",
        // which the page turns into "issue one first".
        CachedAnswer::NoKey => no_store(
            (
                StatusCode::NOT_FOUND,
                Json(errors::ErrorEnvelope {
                    code: NO_KEY,
                    message: "you have no API key yet; issue one from the dashboard first".into(),
                    details: None,
                }),
            )
                .into_response(),
        ),
    }
}

/// `no-store` on every response this module produces — the handler's own
/// statement of the rule the gateway (`portalSettings`) and CloudFront
/// (`CachingDisabled`) also state. Usage is per-caller data on an
/// authenticated route; any shared cache entry serves one visitor another
/// visitor's numbers.
fn no_store(mut response: Response) -> Response {
    cache_control::attach(&mut response, cache_control::NO_STORE);
    response
}

/// `503` for a deployment that reached this route with nothing wired.
fn unconfigured() -> Response {
    no_store(errors::service_unavailable(
        USAGE_UNCONFIGURED,
        "usage reporting is not configured on this deployment",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Under the portal prefix, so [0183]'s gate covers it without knowing it
    /// exists.
    #[test]
    fn the_route_lives_under_the_gated_prefix() {
        assert!(USAGE_PATH.starts_with(super::super::PORTAL_API_PREFIX));
    }

    /// Depth 3 under `/api-tokens/api/`, like `/key`: one segment, no deeper.
    /// The currently-deployed gateway maps depth 1-2 only (task 0205 ships the
    /// greedy proxy), so this route works against production even before that
    /// deploy — asserted so nobody moves it deeper without noticing.
    #[test]
    fn the_route_is_one_segment_under_the_prefix() {
        let rest = USAGE_PATH.trim_start_matches(super::super::PORTAL_API_PREFIX);
        assert_eq!(rest, "usage");
        assert!(!rest.contains('/'));
    }

    fn period(y: i32, m: u32, d: u32) -> Period {
        current_period(NaiveDate::from_ymd_opt(y, m, d).unwrap())
    }

    /// Mid-month, mid-year — the ordinary case.
    #[test]
    fn the_period_is_the_calendar_month() {
        let p = period(2026, 8, 19);
        assert_eq!(p.start, "2026-08-01");
        assert_eq!(p.end, "2026-08-31");
        assert_eq!(p.resets_at, "2026-09-01T00:00:00Z");
    }

    /// The 1st and the last day are both inside their own month.
    #[test]
    fn the_boundaries_belong_to_their_month() {
        let first = period(2026, 9, 1);
        assert_eq!(first.start, "2026-09-01");
        assert_eq!(first.end, "2026-09-30");
        let last = period(2026, 9, 30);
        assert_eq!(last.start, "2026-09-01");
        assert_eq!(last.resets_at, "2026-10-01T00:00:00Z");
    }

    /// December resets into the next year.
    #[test]
    fn december_rolls_into_january() {
        let p = period(2026, 12, 31);
        assert_eq!(p.start, "2026-12-01");
        assert_eq!(p.end, "2026-12-31");
        assert_eq!(p.resets_at, "2027-01-01T00:00:00Z");
    }

    /// February in a leap year has its 29th.
    #[test]
    fn a_leap_february_ends_on_the_29th() {
        let p = period(2028, 2, 15);
        assert_eq!(p.end, "2028-02-29");
        assert_eq!(p.resets_at, "2028-03-01T00:00:00Z");
    }

    fn usage_answer(period_start: &str) -> CachedAnswer {
        CachedAnswer::Usage(UsageResponse {
            used: Some(1),
            remaining: Some(2),
            limit: Some(3),
            period_start: period_start.to_string(),
            period_end: "2026-08-31".to_string(),
            resets_at: "2026-09-01T00:00:00Z".to_string(),
            as_of: "2026-08-19T10:00:00Z".to_string(),
        })
    }

    /// A cached answer survives the cache only inside its own period: last
    /// month's numbers must not be served labelled "this period" the minute
    /// after the boundary.
    #[test]
    fn a_cached_answer_dies_at_the_month_boundary() {
        assert!(answers_for_period(
            &usage_answer("2026-08-01"),
            "2026-08-01"
        ));
        assert!(!answers_for_period(
            &usage_answer("2026-08-01"),
            "2026-09-01"
        ));
    }

    /// "No key" carries no period and stays valid across the boundary — a
    /// month rolling over does not conjure a key into existence.
    #[test]
    fn no_key_is_period_independent() {
        assert!(answers_for_period(&CachedAnswer::NoKey, "2026-09-01"));
    }

    const ANY_PERIOD: &str = "2026-08-01";

    /// The write-after-eviction race, replayed step by step: a "no key"
    /// snapshotted before a concurrent issue bumped the epoch must be
    /// discarded, not stored — storing it would resurrect, for a full TTL,
    /// exactly the answer the eviction removed.
    #[test]
    fn a_no_key_from_before_an_eviction_is_not_stored() {
        let state = UsageState::new(None, None);
        let sub = "308994132968210433";

        // The lookup starts: it reads the epoch, then (concurrently) the
        // user's key issue completes and invalidates.
        let epoch_at_lookup_start = epoch_of(&state, sub);
        state.cache_handle().invalidate_no_key(sub);

        // The lookup finishes with its stale keyless snapshot.
        remember(&state, sub, CachedAnswer::NoKey, epoch_at_lookup_start);
        assert!(
            cached(&state, sub, CACHE_TTL, ANY_PERIOD).is_none(),
            "a pre-eviction 'no key' must not be cached"
        );

        // Whereas a lookup that STARTED after the eviction stores normally.
        remember(&state, sub, CachedAnswer::NoKey, epoch_of(&state, sub));
        assert!(cached(&state, sub, CACHE_TTL, ANY_PERIOD).is_some());
    }

    /// The guard is for "no key" only: a real usage answer cannot be
    /// falsified by a key coming into existence, so it stores regardless of
    /// the epoch — and the eviction itself leaves real answers alone.
    #[test]
    fn a_real_answer_stores_regardless_of_the_epoch() {
        let state = UsageState::new(None, None);
        let sub = "308994132968210433";

        let stale_epoch = epoch_of(&state, sub);
        state.cache_handle().invalidate_no_key(sub);
        remember(&state, sub, usage_answer(ANY_PERIOD), stale_epoch);
        assert!(cached(&state, sub, CACHE_TTL, ANY_PERIOD).is_some());

        // And invalidating again does not evict it — only "no key" is the
        // handle's to remove.
        state.cache_handle().invalidate_no_key(sub);
        assert!(cached(&state, sub, CACHE_TTL, ANY_PERIOD).is_some());
    }
}
