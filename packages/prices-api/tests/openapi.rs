//! Contract tests for the OpenAPI document served at `GET /api-docs-json`
//! (task 0124).
//!
//! The document is a public promise about the deployed API, so these assert the
//! properties a consumer relies on: it is reachable anonymously, it advertises
//! a base URL that actually serves the API, it declares the auth the data
//! routes need, and its route list matches the deployed router **in both
//! directions**.
//!
//! `EXPECTED_ROUTES` is deliberately a hand-written list rather than something
//! derived from the router: derived-from-the-router would make the coverage
//! test tautological, since the spec is generated from that same router and
//! could only ever agree with itself. What the list mirrors is
//! `infra/src/lib/stacks/api-gateway-stack.ts` — the routes API Gateway maps.
//!
//! Being hand-written, it can itself go stale, so it is the *fast* half of a
//! two-part guard. The authoritative half is
//! `tools/scripts/verify-openapi-routes.mjs`, which runs in CI and derives both
//! sides from artifacts — the synthesized CloudFormation template and the
//! extracted document — so neither side can drift unnoticed. This test fails in
//! milliseconds on `cargo test` and needs no synth; that one cannot be fooled by
//! a stale mirror. Keep both.
//!
//! The drift they exist to catch is real: `/api-docs-json` was in the axum
//! router for months while the gateway never mapped it, and nothing failed.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use prices_api::{AppConfig, AppState, app};
use serde_json::Value;
use tower::ServiceExt; // for `oneshot`

/// Every (method, path) API Gateway maps, per `api-gateway-stack.ts`.
const EXPECTED_ROUTES: &[(&str, &str)] = &[
    ("get", "/health"),
    ("get", "/api-docs-json"),
    ("get", "/v1/assets"),
    ("get", "/v1/assets/{asset_identifier}"),
    ("get", "/v1/assets/{asset_identifier}/price"),
    ("get", "/v1/assets/{asset_identifier}/ohlcv"),
    ("get", "/v1/oracles/{asset_identifier}"),
    ("get", "/v1/backfill/status"),
    ("post", "/v1/prices/batch"),
];

/// The two routes that are anonymous by design: the liveness probe and the API
/// description itself. Everything else must carry the `x-api-key` requirement.
const ANONYMOUS_ROUTES: &[&str] = &["/health", "/api-docs-json"];

fn config_with(base_url: Option<&str>, api_keys: Vec<String>) -> AppConfig {
    AppConfig {
        ch_enabled: false,
        base_url: base_url.map(str::to_string),
        api_keys,
    }
}

/// Fetch `/api-docs-json` through the real router and parse the body.
async fn fetch_spec(config: &AppConfig) -> (StatusCode, Option<String>, Value) {
    let response = app(config, AppState::without_ch())
        .oneshot(
            Request::builder()
                .uri("/api-docs-json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let cache_control = response
        .headers()
        .get(header::CACHE_CONTROL)
        .map(|v| v.to_str().unwrap().to_string());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    (
        status,
        cache_control,
        serde_json::from_slice(&body).unwrap(),
    )
}

/// Collect `(method, path)` for every operation in the document.
fn spec_routes(spec: &Value) -> Vec<(String, String)> {
    let mut routes: Vec<(String, String)> = spec["paths"]
        .as_object()
        .expect("spec has a paths object")
        .iter()
        .flat_map(|(path, ops)| {
            ops.as_object()
                .expect("path item is an object")
                .keys()
                // Path items may carry non-operation keys (parameters, summary).
                // `head`/`options` are excluded to match `HTTP_METHODS` in
                // `tools/scripts/verify-openapi-routes.mjs`: that script drops
                // them from BOTH sides so task 0126's `addCorsPreflight` does
                // not read as undocumented drift. Including them here would put
                // the two guards on different method sets, and the day an
                // OPTIONS operation appears this test would flag what the
                // authoritative check deliberately ignores.
                .filter(|k| {
                    matches!(
                        k.as_str(),
                        "get" | "put" | "post" | "delete" | "patch" | "trace"
                    )
                })
                .map(|method| (method.clone(), path.clone()))
                .collect::<Vec<_>>()
        })
        .collect();
    routes.sort();
    routes
}

#[tokio::test]
async fn spec_is_valid_openapi_3() {
    let (status, _, spec) = fetch_spec(&config_with(None, vec![])).await;

    assert_eq!(status, StatusCode::OK);
    // utoipa 5 emits 3.1.0 (it has no 3.0 mode). 3.1 is a valid OpenAPI major
    // release and what the linter is pointed at; see task 0124.
    let version = spec["openapi"]
        .as_str()
        .expect("openapi version is a string");
    assert!(
        version.starts_with("3."),
        "expected an OpenAPI 3.x document, got {version}"
    );
    assert_eq!(spec["info"]["title"], "Stellar Prices API");
    assert!(
        !spec["info"]["version"].as_str().unwrap().is_empty(),
        "info.version must be populated from CARGO_PKG_VERSION"
    );
}

#[tokio::test]
async fn spec_route_coverage_matches_the_deployed_gateway_both_ways() {
    let (_, _, spec) = fetch_spec(&config_with(None, vec![])).await;

    let actual = spec_routes(&spec);
    let mut expected: Vec<(String, String)> = EXPECTED_ROUTES
        .iter()
        .map(|(m, p)| (m.to_string(), p.to_string()))
        .collect();
    expected.sort();

    let missing: Vec<_> = expected.iter().filter(|r| !actual.contains(r)).collect();
    let undeployed: Vec<_> = actual.iter().filter(|r| !expected.contains(r)).collect();

    assert!(
        missing.is_empty(),
        "gateway maps routes the spec does not document: {missing:?}"
    );
    assert!(
        undeployed.is_empty(),
        "spec documents routes the gateway does not map (they would 403/404 for \
         every reader): {undeployed:?} — add them to api-gateway-stack.ts or \
         stop documenting them"
    );
}

#[tokio::test]
async fn every_ledger_field_publishes_the_uint32_ceiling() {
    let (_, _, spec) = fetch_spec(&config_with(None, vec![])).await;

    // A Stellar ledger sequence is `uint32` in the protocol's `LedgerHeader`,
    // so every ledger-valued field carries `maximum = 4_294_967_295` as a
    // literal (`backfill/dto.rs` — attribute macros cannot read a const). This
    // reads those literals back out of the served document, which is the only
    // place they are observable: a const in the DTO module can assert things
    // about itself but nothing about what the attributes emitted.
    //
    // The field set is derived from the document rather than listed here, so a
    // ledger field added later without the attribute fails as a missing
    // `maximum` instead of passing unnoticed.
    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("spec has component schemas");

    let mut checked = 0;
    for (schema_name, schema) in schemas {
        let Some(properties) = schema["properties"].as_object() else {
            continue;
        };
        for (field, definition) in properties {
            if !(field.ends_with("_ledger") || field == "ledgers_remaining") {
                continue;
            }
            let maximum = definition["maximum"].as_u64().unwrap_or_else(|| {
                panic!(
                    "{schema_name}.{field} is a ledger sequence but publishes no \
                     `maximum` — add #[schema(maximum = 4_294_967_295u64)]"
                )
            });
            assert_eq!(
                maximum,
                u64::from(u32::MAX),
                "{schema_name}.{field} publishes maximum {maximum}, but a ledger \
                 sequence is uint32 (max {})",
                u32::MAX
            );
            checked += 1;
        }
    }

    // realtime_tip_ledger + SdexStream's four. A rename that empties the filter
    // would otherwise pass vacuously.
    assert_eq!(
        checked, 5,
        "expected 5 ledger-valued fields in the document, found {checked} — \
         update this count if the DTOs genuinely changed"
    );
}

#[tokio::test]
async fn spec_declares_the_x_api_key_security_scheme() {
    let (_, _, spec) = fetch_spec(&config_with(None, vec![])).await;

    let scheme = &spec["components"]["securitySchemes"]["api_key"];
    assert_eq!(scheme["type"], "apiKey");
    assert_eq!(scheme["in"], "header");
    // Must match the header the in-app gate and the usage plan read.
    assert_eq!(scheme["name"], "x-api-key");
}

#[tokio::test]
async fn key_gated_routes_require_the_key_and_anonymous_ones_opt_out() {
    let (_, _, spec) = fetch_spec(&config_with(None, vec![])).await;

    // The document-wide default: everything needs `api_key` unless it says
    // otherwise.
    assert_eq!(spec["security"][0]["api_key"], serde_json::json!([]));

    for (method, path) in spec_routes(&spec) {
        let op = &spec["paths"][&path][&method];
        let requirements = op["security"].as_array();
        if ANONYMOUS_ROUTES.contains(&path.as_str()) {
            // An explicit empty requirement `[{}]` is how OpenAPI says "this
            // operation opts out of the global requirement".
            let requirements = requirements
                .unwrap_or_else(|| panic!("{method} {path} must opt out of the global security"));
            assert_eq!(
                requirements,
                &vec![serde_json::json!({})],
                "{method} {path} is served anonymously but does not say so"
            );
        } else {
            assert!(
                requirements.is_none(),
                "{method} {path} overrides the global x-api-key requirement — data \
                 routes must inherit it"
            );
        }
    }
}

#[tokio::test]
async fn servers_is_stamped_from_the_configured_base_url() {
    let base = "https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production";
    let (_, _, spec) = fetch_spec(&config_with(Some(base), vec![])).await;

    let servers = spec["servers"].as_array().expect("servers is stamped");
    assert_eq!(servers.len(), 1);
    let url = servers[0]["url"].as_str().unwrap();
    assert_eq!(url, base);
    // The stage-prefix trap (task 0089): an execute-api base without the stage
    // path advertises a URL where every route 403s.
    assert!(
        !url.contains(".execute-api.") || url.ends_with("/production"),
        "execute-api `servers` URL must include the stage path, got {url}"
    );
}

#[tokio::test]
async fn spec_is_reachable_without_a_key_when_the_gate_is_armed() {
    // With `API_KEYS` set the in-app gate is armed; the spec must still answer,
    // matching the keyless API Gateway mapping.
    let config = config_with(None, vec!["partner-key".to_string()]);
    let (status, _, spec) = fetch_spec(&config).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(spec["info"]["title"], "Stellar Prices API");
}

#[tokio::test]
async fn spec_response_is_cacheable_for_the_life_of_a_deployment() {
    let (_, cache_control, _) = fetch_spec(&config_with(None, vec![])).await;

    // Mirrors the 3600s gateway TTL on /api-docs-json (api-gateway-stack.ts).
    assert_eq!(cache_control.as_deref(), Some("public, max-age=3600"));
}
