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
pub mod keys;
pub mod usage;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::common::cache_control;
use crate::config::AppConfig;

/// Path prefix owned by the portal's backend. Everything under it is gated.
///
/// Both halves are load-bearing and are fixed by [0184]'s CloudFront behaviour
/// table: `<app>/*` is an app's bundle and `<app>/api/*` is its backend, with
/// the `/api/*` behaviour ordered **first**. The OAuth redirect URI registered
/// with Discord ([0186]) must live **under** this prefix — it is a concrete
/// callback path such as `/api-tokens/api/auth/callback`, and Discord matches
/// the whole URI, so registering the bare prefix would fail at callback time.
pub const PORTAL_API_PREFIX: &str = "/api-tokens/api/";

/// The one portal route that answers while the portal is closed.
pub const CONFIG_PATH: &str = "/api-tokens/api/config";

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
/// half-built portal to every integrator reading the spec. [0195]'s Swagger UI
/// describes the public API; the portal describes itself to its own bundle.
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

    // Sign-in (task 0186), merged the same way and for the same reason. Mounted
    // UNCONDITIONALLY, including when no OAuth credentials were loaded: the
    // handlers answer `503` in that case rather than the routes silently not
    // existing, so a deployment that opens the portal without provisioning the
    // secret says so instead of looking like a portal with no sign-in. While the
    // portal is closed the gate below makes the distinction moot — every path
    // here is the same empty `404` as an unrouted one.
    let sign_in = auth::routes(auth::AuthState::new(
        config.portal_oauth.clone(),
        config.portal_endpoints.clone(),
    ));

    // Usage against quota (task 0188), merged the same way and mounted under
    // the same conditions as sign-in: unconditionally, answering `503` when
    // nothing is provisioned rather than not existing. It shares the key
    // routes' control-plane client — usage is scoped to
    // `(usagePlanId, apiKeyId)` and the key id comes from the same lookup —
    // but carries a state of its own, because it also owns the in-process
    // cache that keeps dashboard refreshes off the account-wide control-plane
    // budget. Built before the key routes so they can hold the cache handle
    // below.
    let usage_state =
        usage::UsageState::new(config.portal_oauth.clone(), config.portal_keys.clone());
    let usage_cache = usage_state.cache_handle();
    let usage = usage::routes(usage_state);

    // Self-service API keys (task 0187), merged the same way and mounted under
    // the same conditions: unconditionally, answering `503` when nothing is
    // provisioned rather than not existing. The state carries the OAuth secret
    // because the session cookie is what authorizes a key — there is no API key
    // to present on the route whose job is to hand one out. The usage-cache
    // handle lets a successful issue evict a cached "no key" (task 0188).
    let api_keys = keys::routes(
        keys::KeysState::new(config.portal_oauth.clone(), config.portal_keys.clone())
            .with_usage_cache(usage_cache),
    );

    router
        .merge(routes)
        .merge(sign_in)
        .merge(api_keys)
        .merge(usage)
        .layer(axum::middleware::from_fn_with_state(gate, gate_portal))
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
/// [`CONFIG_PATH`] is exempt in both directions — see [`config_handler`].
pub async fn gate_portal(State(gate): State<PortalGate>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if !is_portal_path(path) || path == CONFIG_PATH || gate.enabled {
        return next.run(req).await;
    }
    StatusCode::NOT_FOUND.into_response()
}

#[cfg(test)]
mod tests {
    use super::{CONFIG_PATH, PORTAL_API_PREFIX, is_portal_path};

    #[test]
    fn portal_paths_are_recognised_by_prefix() {
        assert!(is_portal_path(CONFIG_PATH));
        assert!(is_portal_path("/api-tokens/api/auth/login"));
        assert!(is_portal_path("/api-tokens/api/key"));
        assert!(is_portal_path("/api-tokens/api/usage"));
        // Routes that do not exist yet still match — that is the point.
        assert!(is_portal_path("/api-tokens/api/nothing-here"));
    }

    #[test]
    fn non_portal_paths_are_untouched() {
        assert!(!is_portal_path("/health"));
        assert!(!is_portal_path("/api-docs-json"));
        assert!(!is_portal_path("/v1/prices/batch"));
    }

    /// The bundle lives at `/api-tokens/*` and is served by S3, never by this
    /// Lambda. Gating it here would be gating something that never arrives —
    /// and worse, would suggest the bundle is protected when it is public.
    #[test]
    fn the_static_bundle_prefix_is_not_ours() {
        assert!(!is_portal_path("/api-tokens/"));
        assert!(!is_portal_path("/api-tokens/dashboard"));
        assert!(!is_portal_path("/api-tokens/assets/index.js"));
    }

    /// `/api-tokens/api` with no trailing slash is not a route we serve, and
    /// must not be mistaken for one — the prefix carries its slash on purpose.
    #[test]
    fn the_prefix_carries_its_trailing_slash() {
        assert!(PORTAL_API_PREFIX.ends_with('/'));
        assert!(!is_portal_path("/api-tokens/api"));
    }
}
