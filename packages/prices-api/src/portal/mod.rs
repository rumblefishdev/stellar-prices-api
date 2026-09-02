//! Ship-to-production safety gate for the onboarding portal (task 0183).
//!
//! **There is one environment and it is production.** `envName` is typed
//! `'production'` and `infra/envs/` holds only `production.json`, so a
//! `cdk deploy` is a release. The portal is built in twelve further slices
//! (0184–0195) and every one of them reaches the production distribution the
//! moment it merges — there is no staging distribution to be relaxed on.
//!
//! This module is the switch that keeps the unfinished ones unreachable. One
//! boolean, `PORTAL_ENABLED`, read at cold start alongside `CH_ENABLED` and
//! `API_KEYS` (`crate::config`). Off by default, which is the only safe default
//! for a flag whose whole job is to be off.
//!
//! # Using it
//!
//! Locally, turn it on for the run:
//!
//! ```text
//! PORTAL_ENABLED=true cargo run -p prices-api --features local-server --bin serve
//! ```
//!
//! In production it is set explicitly to `'false'` in `compute-stack.ts`, so
//! opening the portal is a one-word diff and shows up in a deploy review.
//!
//! # What "off" looks like on the wire
//!
//! A bare `404` with **no body** — deliberately byte-identical to what the
//! router already returns for a path that does not exist. Not `403`, which
//! confirms the route is there and merely refused; not `503`, which says "come
//! back later"; and not [`crate::common::errors`]'s JSON envelope, which is how
//! a real portal `404` will look once these routes exist and would therefore
//! give the gate away. The router has no `fallback`, so axum's own miss is an
//! empty `404` — this matches it exactly.
//!
//! # What it is not
//!
//! It is **not** an incident kill switch. Flipping it in production takes a
//! deploy, because it is an environment variable. That is a deliberate trade:
//! the flag's job is to keep half-built slices invisible during the build, and
//! it is flipped once, by [0194]'s audit, after [0189]'s eligibility gate
//! passes. If we ever need a switch that beats a rollback on time, that is a
//! runtime-config change (SSM read through the Parameters and Secrets extension
//! the Lambda already loads) and a different task — do not reach for it here.

pub mod auth;
pub mod eligibility;
pub mod keys;
pub mod period;
pub mod usage;

use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::header::{ACCEPT, CONTENT_TYPE};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tower_http::cors::{AllowCredentials, AllowOrigin, CorsLayer};

use crate::common::cache_control;
use crate::config::AppConfig;

/// Path prefix owned by the portal's backend. Everything under it is gated.
///
/// `/api/` is the whole self-service portal on the shared host (task 0194,
/// 2026-08-31): the bundle **and** the backend answer under this one prefix,
/// with no sub-prefix for either. Nothing routes between them, because they
/// are on different HOSTS — the bundle is served from the block explorer's
/// distribution at `sorobanscan.rumblefish.dev/api/`, and the page calls this
/// backend on the API's own hostname, cross-origin and same-site. So from in
/// here the prefix is simply ours end to end; a bundle path that arrives (it
/// should not) is a plain `404`, the same as any unrouted path.
///
/// Replaces [0161]'s `<app>/*` + `<app>/api/*` convention, which produced
/// `/api/api/…` for an app that is itself called "api". The OAuth redirect URI
/// registered with Discord ([0186]) must live **under** this prefix — it is a
/// concrete callback path such as `/api/auth/callback`, and Discord matches
/// the whole URI, so registering the bare prefix would fail at callback time.
pub const PORTAL_API_PREFIX: &str = "/api/";

/// The one portal route that answers while the portal is closed.
pub const CONFIG_PATH: &str = "/api/config";

/// The OpenAPI document, under the portal prefix.
///
/// The same bytes as `GET /api-docs-json` (task 0124) — `lib.rs` mounts one
/// handler at both paths. This alias exists because on the shared host the
/// root belongs to another application, so the portal's "API reference" link
/// has to stay under `/api/` to reach us at all. Exempt from the gate like
/// [`CONFIG_PATH`] and from the API-key check like the root copy: an API
/// description is public documentation, and hiding it while the portal is
/// closed would serve nobody.
///
/// Not registered in the OpenAPI document itself — it is an alias of the
/// route that is, and `tools/scripts/verify-openapi-routes.mjs` refuses any
/// documented path under the prefix.
pub const OPENAPI_PATH: &str = "/api/api-docs-json";

/// What the portal frontend needs before it can render anything.
///
/// [0185]'s app calls this on load and uses `enabled` to decide between the
/// real UI and a "not yet available" page with no sign-in button. Keeping that
/// decision on the server is what stops the bundle and the backend disagreeing
/// about whether the portal is open — the bundle is public and cached at a CDN,
/// the flag is not.
#[derive(Debug, Serialize)]
pub struct PortalConfig {
    /// Whether the portal is open for business.
    pub enabled: bool,
    /// The free plan's per-key rate limit in requests per second, for the
    /// dashboard to state (task 0188).
    ///
    /// Served from here rather than written into the bundle because it is a
    /// per-env config value (`pricingApiFreePlanRateLimit`) that the gateway
    /// enforces and the page merely reports: a literal in the frontend is the
    /// one number on that panel that can drift from what is actually enforced.
    /// It rides on `/config` rather than on `/usage` because the dashboard
    /// states it in the no-key state too, and that state is a `404` with no
    /// body to carry it — and because the limit is a property of the plan every
    /// key joins, not of any one caller's key.
    ///
    /// Omitted from the JSON entirely when this deployment was not told what
    /// the limit is; the page then omits the line rather than inventing a
    /// figure. See `AppConfig::portal_rate_limit`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_second: Option<u32>,
}

/// Cloneable gate state carried by the middleware.
#[derive(Clone)]
pub struct PortalGate {
    enabled: bool,
    rate_limit: Option<u32>,
}

impl PortalGate {
    /// Build a gate directly, so a test can drive [`gate_portal`] against a
    /// route of its own under [`PORTAL_API_PREFIX`].
    ///
    /// Not a convenience. Exporting the middleware while withholding its state
    /// is how the first version of `tests/portal.rs` came to assert nothing:
    /// the only route under the prefix was [`CONFIG_PATH`], which the gate
    /// skips, so every "closed" assertion was really watching an unrouted path
    /// 404 — and the whole suite stayed green with the gate deleted.
    /// Only [`gate_portal`] reads this state, and the gate turns on `enabled`
    /// alone — so the rate limit a test does not care about stays `None` here
    /// rather than becoming a second argument at every call site.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            rate_limit: None,
        }
    }
}

/// Mount the portal's always-available routes and layer the gate.
///
/// Wired in [`crate::app`] rather than through `openapi::register_routes`,
/// deliberately: these are the portal's own endpoints, not partner-facing data
/// routes, and publishing them in the OpenAPI document would advertise a
/// half-built portal to every integrator reading the spec. [0195]'s API
/// reference describes the public API; the portal describes itself to its own
/// bundle.
pub fn apply(router: Router, config: &AppConfig) -> Router {
    let gate = PortalGate {
        enabled: config.portal_enabled,
        rate_limit: config.portal_rate_limit,
    };
    // Merged as its own `Router` rather than `.route()`d onto the caller's:
    // by this point the data routes have had `AppState` applied and the router
    // is `Router<()>`, so a route needing `PortalGate` has to bring and resolve
    // its own state before it joins.
    let routes = Router::new()
        .route(CONFIG_PATH, get(config_handler))
        .with_state(gate.clone());

    // Usage against quota (task 0188), merged the same way and mounted under
    // the same conditions as everything below: unconditionally, answering
    // `503` when nothing is provisioned rather than not existing. It shares
    // the key routes' control-plane client — usage is scoped to
    // `(usagePlanId, apiKeyId)` and the key id comes from the same lookup —
    // but carries a state of its own, because it also owns the in-process
    // cache that keeps dashboard refreshes off the account-wide control-plane
    // budget. Built first so sign-in and the key routes can hold the cache
    // handle below.
    let usage_state =
        usage::UsageState::new(config.portal_oauth.clone(), config.portal_keys.clone());
    let usage_cache = usage_state.cache_handle();
    let usage = usage::routes(usage_state);

    // Sign-in (task 0186) and the eligibility-checked issue round-trip
    // (task 0189), merged the same way and for the same reason. Mounted
    // UNCONDITIONALLY, including when no OAuth credentials were loaded: the
    // handlers answer `503` in that case rather than the routes silently not
    // existing, so a deployment that opens the portal without provisioning the
    // secret says so instead of looking like a portal with no sign-in. While the
    // portal is closed the gate below makes the distinction moot — every path
    // here is the same empty `404` as an unrouted one.
    //
    // The issue deps carry the control-plane client and the usage-cache handle
    // because the `action=issue` callback is where a key is actually created
    // (`keys::issue_for`) — the key ROUTE below is read-only, which is what
    // makes "issue is unreachable with a session cookie alone" structural.
    let sign_in = auth::routes(
        auth::AuthState::new(config.portal_oauth.clone(), config.portal_endpoints.clone())
            .with_issue(auth::issue::IssueDeps::new(
                config.portal_keys.clone(),
                Some(usage_cache.clone()),
                config.portal_eligibility.clone(),
            ))
            .with_web_origin(config.portal_web_origin.as_deref()),
    );

    // The key reveal (task 0187, read-only since task 0189), merged the same
    // way and mounted under the same conditions: unconditionally, answering
    // `503` when nothing is provisioned rather than not existing. The state
    // carries the OAuth secret because the session cookie is what authorizes a
    // reveal — showing the caller what already belongs to them, which is why a
    // session suffices here and does not for the issue above. The usage-cache
    // handle lets a successful reveal evict a cached "no key" (task 0188).
    let api_keys = keys::routes(
        keys::KeysState::new(config.portal_oauth.clone(), config.portal_keys.clone())
            .with_usage_cache(usage_cache)
            .with_web_origin(config.portal_web_origin.as_deref()),
    );

    // The portal's routes as one group, so the CORS layer covers exactly them:
    // the data routes and the OpenAPI copies stay outside it. The gate wraps
    // the whole router as before and runs FIRST (outermost), so a closed
    // portal answers a preflight with the same empty `404` as anything else
    // under the prefix — a browser then reports a CORS failure, which is the
    // honest reading of a door that is not open.
    let portal = Router::new()
        .merge(routes)
        .merge(sign_in)
        .merge(api_keys)
        .merge(usage)
        .layer(cors_layer(config.portal_web_origin.as_deref()));

    router
        .merge(portal)
        .layer(axum::middleware::from_fn_with_state(gate, gate_portal))
}

/// The CORS answer for the portal's routes: one origin, with credentials.
///
/// The bundle is served from another application's distribution
/// (`AppConfig::portal_web_origin`) and calls this backend on a host of its
/// own — cross-origin, same-site (task 0194). Same-site is what lets the
/// `SameSite=Lax` session cookie ride on those calls at all; this layer is
/// what lets the page read the answers. Three properties, each load-bearing:
///
/// - **Exactly one origin**, never a reflection of the request's. A
///   credentialed answer cannot say `*`, and reflecting whatever `Origin`
///   arrives would hand every site on the internet a readable, cookie-bearing
///   `GET /api/key`. With nothing configured the allow-list is EMPTY: no
///   `Access-Control-Allow-Origin` on any answer, which is the same-origin
///   deployment's correct behaviour and what every existing test sees.
/// - **`Access-Control-Allow-Credentials: true`**, because the session is a
///   cookie and a `fetch` with `credentials: 'include'` is refused by the
///   browser without it.
/// - **The marker header is allowed** (`keys::PORTAL_REQUEST_HEADER`), or
///   the revoke's preflight fails and the one write the portal has becomes
///   unreachable — silently, from this side.
///
/// In production the preflight itself is answered by API Gateway's MOCK
/// (`addCorsPreflight` in `api-gateway-stack.ts`, same origin, same
/// headers, same `max-age`) and never reaches this code; this layer's job
/// there is the headers on the actual responses. Locally, with `serve` and a
/// bundle built for a different port, it answers both — one configuration,
/// two callers.
pub(crate) fn cors_layer(web_origin: Option<&str>) -> CorsLayer {
    let Some(ours) = web_origin.and_then(|o| HeaderValue::from_str(o).ok()) else {
        // Nothing allowed, nothing emitted: an empty list answers no origin.
        return CorsLayer::new().allow_origin(AllowOrigin::list([]));
    };
    // `list`, not `exact`: `exact` writes its value on EVERY answer and leaves
    // the comparison to the browser, which is correct for the browser and
    // wrong for this API's tests and its logs — an answer to `evil.example`
    // would carry our origin as if it had been granted. `list` compares, and
    // so does the credentials predicate, so both headers appear together or
    // not at all.
    let granted = ours.clone();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list([ours]))
        .allow_credentials(AllowCredentials::predicate(move |origin, _| {
            origin == granted
        }))
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([
            CONTENT_TYPE,
            ACCEPT,
            HeaderName::from_static(keys::PORTAL_REQUEST_HEADER.0),
        ])
        .max_age(Duration::from_secs(3600))
}

/// Report whether the portal is open. Always answers, in both states — it is
/// the question "is the portal open?", so refusing to answer it while closed
/// would be circular.
async fn config_handler(State(gate): State<PortalGate>) -> Response {
    let mut resp = Json(PortalConfig {
        enabled: gate.enabled,
        rate_limit_per_second: gate.rate_limit,
    })
    .into_response();
    // Never cached: the flag changes on deploy, and a CDN or browser holding a
    // stale `enabled: false` would keep the portal dark for its viewers long
    // after it opened — with nothing on screen to suggest why.
    cache_control::attach(&mut resp, cache_control::NO_STORE);
    resp
}

/// True for any path the portal owns.
///
/// Prefix match, not equality: it has to cover routes that do not exist yet
/// ([0191]'s rework, [0192]'s revocation), which is the point of gating by
/// prefix rather than enumerating. [0186]'s `/auth/*`, [0187]'s `/key` and
/// [0188]'s `/usage` have since landed and none of them touched this function —
/// which is the property working, not an argument for replacing it with a list.
fn is_portal_path(path: &str) -> bool {
    path.starts_with(PORTAL_API_PREFIX)
}

/// Serve portal routes only when the portal is open; otherwise return the same
/// empty `404` a nonexistent path would.
///
/// [`CONFIG_PATH`] and [`OPENAPI_PATH`] are exempt in both directions — see
/// [`config_handler`] and the note on the latter.
pub async fn gate_portal(State(gate): State<PortalGate>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if !is_portal_path(path) || path == CONFIG_PATH || path == OPENAPI_PATH || gate.enabled {
        return next.run(req).await;
    }
    StatusCode::NOT_FOUND.into_response()
}

#[cfg(test)]
mod tests {
    use super::{CONFIG_PATH, OPENAPI_PATH, PORTAL_API_PREFIX, is_portal_path};

    #[test]
    fn portal_paths_are_recognised_by_prefix() {
        assert!(is_portal_path(CONFIG_PATH));
        assert!(is_portal_path("/api/auth/login"));
        assert!(is_portal_path("/api/key"));
        assert!(is_portal_path("/api/usage"));
        // Routes that do not exist yet still match — that is the point.
        assert!(is_portal_path("/api/nothing-here"));
    }

    #[test]
    fn non_portal_paths_are_untouched() {
        assert!(!is_portal_path("/health"));
        assert!(!is_portal_path("/api-docs-json"));
        assert!(!is_portal_path("/v1/prices/batch"));
    }

    /// The bundle shares the prefix and is carved out at the CDN, so as far
    /// as this Lambda is concerned those paths are ours too. They should never
    /// arrive; if one does, the gate treats it like any other unrouted portal
    /// path — a `404` in both states, never a hint that the bundle is
    /// protected (it is public).
    #[test]
    fn the_bundle_paths_are_inside_the_prefix() {
        assert!(is_portal_path("/api/"));
        assert!(is_portal_path("/api/dashboard"));
        assert!(is_portal_path("/api/assets/index.js"));
    }

    /// The alias sits under the prefix (so it reaches us on the shared host)
    /// while the root copy does not — both are served, only one is gated by
    /// prefix and then exempted by name.
    #[test]
    fn the_openapi_alias_is_under_the_prefix_and_the_root_copy_is_not() {
        assert!(is_portal_path(OPENAPI_PATH));
        assert!(!is_portal_path("/api-docs-json"));
        assert!(OPENAPI_PATH.starts_with(PORTAL_API_PREFIX));
    }

    /// `/api` with no trailing slash is not a route we serve, and
    /// must not be mistaken for one — the prefix carries its slash on purpose.
    #[test]
    fn the_prefix_carries_its_trailing_slash() {
        assert!(PORTAL_API_PREFIX.ends_with('/'));
        assert!(!is_portal_path("/api"));
    }
}
