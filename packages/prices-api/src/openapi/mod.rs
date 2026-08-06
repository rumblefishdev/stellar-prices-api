//! OpenAPI assembly via `utoipa` + `utoipa-axum`.
//!
//! [`register_routes`] is the single source of routes: the live app calls
//! `.with_state(state).split_for_parts()` to get `(Router, OpenApi)`, while
//! `bin/extract_openapi.rs` calls `.split_for_parts()` directly for the spec —
//! so the documented routes and the served routes can never drift. Each `/v1`
//! resource adds its `routes!(...)` here as it lands (Phases 2–3).

use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Header name carrying the partner API key. Must match the in-app gate
/// (`crate::auth`) and the API Gateway usage-plan key header (ADR 0008).
pub const API_KEY_HEADER: &str = "x-api-key";

/// Name of the `x-api-key` security scheme in `components.securitySchemes`.
///
/// The global requirement in the `#[openapi(...)]` attribute below must spell
/// the same name, and has to spell it as a literal — attribute macros cannot
/// read a const. Change one and the requirement points at a scheme that does
/// not exist; `tests/openapi.rs` asserts both spellings so that fails loudly.
pub const API_KEY_SCHEME: &str = "api_key";

/// Declares the `x-api-key` scheme in `components.securitySchemes`.
///
/// Has to be a [`Modify`] pass rather than a `#[openapi]` attribute because
/// utoipa only exposes `SecurityScheme` construction at runtime.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        openapi
            .components
            .get_or_insert_with(Default::default)
            .add_security_scheme(
                API_KEY_SCHEME,
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new(API_KEY_HEADER))),
            );
    }
}

/// Documentation-only stub for `GET /api-docs-json` (task 0124).
///
/// The route itself is wired in [`crate::app`], *after* `split_for_parts()` —
/// it has to be, since it serves the very spec that split produces. Declaring
/// the path here keeps the document honest: every deployed route appears in it,
/// including this one. Anonymous (`security(())`) — an API description is public
/// documentation, and gating it behind a key the reader does not have yet is a
/// self-service dead end.
// The explicit `summary`/`description` keep the rustdoc above (which is aimed
// at maintainers, and carries intra-doc link syntax) out of the published
// document.
#[utoipa::path(
    get,
    path = "/api-docs-json",
    tag = "ops",
    summary = "`GET /api-docs-json` — this OpenAPI document.",
    description = "Returns the machine-readable description of this API. \
                   Served anonymously. The document is identical for the life \
                   of a deployment; revalidate every 5 minutes to pick up the \
                   next one.",
    security(()),
    responses(
        (
            status = 200,
            description = "This OpenAPI document",
            content_type = "application/json"
        ),
        // Anonymous and input-free is not the same as error-free. This route
        // sits OUTSIDE the usage plan, so its only limiter is the stage-wide
        // throttle it shares with paying traffic — API Gateway can 429 it. And
        // unlike `/health` (a MockIntegration) a cache miss here reaches the
        // Lambda, so a cold-start or panic surfaces as a gateway 5xx.
        //
        // Neither carries `ErrorEnvelope`: both are produced by API Gateway,
        // whose body is `{"message": …}`. Documented so a client generated from
        // this document has an error branch rather than trying to parse that as
        // the OpenAPI document itself.
        (status = 429, description = "Rate limit exceeded (API Gateway stage throttle)"),
        (status = 500, description = "The API description could not be served"),
    )
)]
#[allow(dead_code, reason = "documentation-only; the route is wired in lib.rs")]
fn api_docs_json() {}

/// Top-level API metadata. Schema/path components are collected from the
/// `#[utoipa::path]` handlers wired in [`register_routes`].
///
/// The global `security` requirement means "every operation needs `x-api-key`
/// unless it says otherwise"; `/health` and `/api-docs-json` say otherwise with
/// `security(())`, matching the two exemptions in `auth::is_exempt`.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Stellar Prices API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Public read API for Stellar asset prices, OHLCV, oracle \
                       cross-reference, and backfill status."
    ),
    modifiers(&SecurityAddon),
    security(("api_key" = [])),
    paths(api_docs_json),
    components(schemas(
        crate::common::errors::ErrorEnvelope,
        crate::assets::dto::PriceResponse,
        crate::assets::dto::AssetDetail,
        crate::assets::dto::AssetListItem,
        crate::assets::dto::AssetListResponse,
        crate::assets::dto::Candle,
        crate::assets::dto::OhlcvResponse,
        crate::oracles::dto::OraclesResponse,
        crate::oracles::dto::OracleEntry,
        crate::backfill::dto::BackfillStatus,
        crate::backfill::dto::SdexStream,
        crate::backfill::dto::AmmStream,
        crate::batch::dto::BatchRequest,
        crate::batch::dto::BatchResponse,
    )),
    tags(
        (name = "ops", description = "Operational endpoints (health)"),
        (name = "assets", description = "Asset metadata"),
        (name = "prices", description = "Asset prices (current + batch)"),
        (name = "oracles", description = "Oracle cross-reference prices"),
        (name = "backfill", description = "Historical backfill progress")
    )
)]
pub struct ApiDoc;

/// Stamp `servers` from the configured public base URL.
///
/// Shared by [`crate::app`] (the served document) and `bin/extract_openapi`
/// (the CI-linted document) so the two cannot disagree about the advertised
/// base. A `None` base leaves `servers` absent — correct for local runs, and
/// the reason the CI lint step sets `API_BASE_URL` (an empty `servers` is a
/// lint error, and rightly so: a published document with no base is not
/// actionable).
///
/// The URL must include the stage path for an execute-api host; that invariant
/// is enforced where the value is configured (`infra/src/lib/types.ts`).
pub fn stamp_servers(spec: &mut utoipa::openapi::OpenApi, config: &crate::AppConfig) {
    if let Some(base) = &config.base_url {
        spec.servers = Some(vec![utoipa::openapi::Server::new(base)]);
    }
}

/// Build the un-stated router carrying both the axum routes and their OpenAPI
/// paths. Resource routers nest under `/v1`; operational routes stay at the
/// root.
pub fn register_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(crate::ops::health))
        .nest("/v1", crate::assets::router())
        .nest("/v1", crate::oracles::router())
        .nest("/v1", crate::backfill::router())
        .nest("/v1", crate::batch::router())
}
