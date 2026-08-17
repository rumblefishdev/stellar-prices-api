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
        portal_enabled: false,
        // Sign-in credentials are loaded asynchronously from Secrets Manager
        // (task 0186) and are never part of the environment; `None` is the shape
        // every non-portal test wants.
        portal_oauth: None,
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
                // Matches `HTTP_METHODS` in
                // `tools/scripts/verify-openapi-routes.mjs`, including `head`:
                // that script skips OPTIONS only on the *gateway* side (task
                // 0126's `addCorsPreflight` emits one per resource) and rejects
                // a documented OPTIONS outright. Excluding `head` from both
                // guards, as an earlier revision did, left documented HEAD
                // operations checked by neither — the same unroutable-route
                // hole task 0124 exists to close. `options` is absent here for
                // the same reason it is there: it gets its own assertion below.
                .filter(|k| {
                    matches!(
                        k.as_str(),
                        "get" | "put" | "post" | "delete" | "patch" | "head" | "trace"
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

/// Every `$ref` target name appearing anywhere under `node`.
fn referenced_schemas(node: &Value, out: &mut Vec<String>) {
    match node {
        Value::Object(map) => {
            for (key, value) in map {
                if key == "$ref" {
                    if let Some(name) = value
                        .as_str()
                        .and_then(|r| r.strip_prefix("#/components/schemas/"))
                    {
                        out.push(name.to_string());
                    }
                } else {
                    referenced_schemas(value, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|i| referenced_schemas(i, out)),
        _ => {}
    }
}

/// Schema names reachable from `root`, transitively. Walks `$ref` wherever it
/// appears, so `Option<T>`'s `oneOf: [{type: null}, {$ref}]` is followed like
/// any other reference.
fn reachable_schemas(spec: &Value, root: &Value) -> Vec<String> {
    let mut pending = Vec::new();
    referenced_schemas(root, &mut pending);
    let mut seen: Vec<String> = Vec::new();
    while let Some(name) = pending.pop() {
        if seen.contains(&name) {
            continue;
        }
        referenced_schemas(&spec["components"]["schemas"][&name], &mut pending);
        seen.push(name);
    }
    seen.sort();
    seen
}

/// Whether a property definition declares an integer type (`"integer"` or a
/// nullable `["integer", "null"]`).
fn is_integer(definition: &Value) -> bool {
    match &definition["type"] {
        Value::String(t) => t == "integer",
        Value::Array(types) => types.iter().any(|t| t == "integer"),
        _ => false,
    }
}

fn assert_uint32_ceiling(schema: &str, field: &str, definition: &Value) {
    let maximum = definition["maximum"].as_u64().unwrap_or_else(|| {
        panic!(
            "{schema}.{field} carries a ledger sequence but publishes no `maximum` \
             — add #[schema(maximum = 4_294_967_295u64)]"
        )
    });
    assert_eq!(
        maximum,
        u64::from(u32::MAX),
        "{schema}.{field} publishes maximum {maximum}, but a ledger sequence is \
         uint32 (max {})",
        u32::MAX
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
    // Two complementary rules, because either alone has a blind spot.

    // Rule 1 — by TYPE, over the schemas the backfill endpoint actually
    // returns. Those schemas are reached from the response `$ref`, not named
    // here, so renaming a DTO cannot quietly drop it from the check. Every
    // integer they publish is a ledger sequence (the only other numeric field
    // is `progress_pct`, a double), so this catches a new ledger field whatever
    // it is called — which a name-shaped filter cannot.
    let response = &spec["paths"]["/v1/backfill/status"]["get"]["responses"]["200"]["content"]["application/json"]
        ["schema"];
    let backfill_schemas = reachable_schemas(&spec, response);
    assert!(
        !backfill_schemas.is_empty(),
        "no schemas reachable from the /v1/backfill/status response — the \
         response shape changed and this test is checking nothing"
    );

    let mut by_type = 0;
    for name in &backfill_schemas {
        let Some(properties) = spec["components"]["schemas"][name]["properties"].as_object() else {
            continue;
        };
        for (field, definition) in properties {
            if !is_integer(definition) {
                continue;
            }
            assert_uint32_ceiling(name, field, definition);
            by_type += 1;
        }
    }

    // realtime_tip_ledger + SdexStream's four. Guards against a response-shape
    // change that empties the walk, and forces a decision if a non-ledger
    // integer is ever added to these DTOs.
    assert_eq!(
        by_type, 5,
        "expected 5 integer fields across the backfill DTOs, found {by_type} — \
         if a non-ledger integer was added, this rule needs to stop assuming \
         every integer here is a ledger sequence"
    );

    // Rule 2 — by NAME, over the WHOLE document. Rule 1 cannot see a ledger
    // field on a schema some other endpoint returns; this one can. Deliberately
    // `contains`, not a suffix match: `newest_data_ledger_seq` and
    // `tip_ledger_num` are exactly the names a suffix rule misses.
    let mut by_name = 0;
    for (name, schema) in spec["components"]["schemas"]
        .as_object()
        .expect("spec has component schemas")
    {
        let Some(properties) = schema["properties"].as_object() else {
            continue;
        };
        for (field, definition) in properties {
            if !field.contains("ledger") {
                continue;
            }
            assert_uint32_ceiling(name, field, definition);
            by_name += 1;
        }
    }
    assert!(
        by_name >= 5,
        "expected at least the 5 known ledger-named fields, found {by_name}"
    );
}

#[tokio::test]
async fn no_options_operations_are_documented() {
    let (_, _, spec) = fetch_spec(&config_with(None, vec![])).await;

    // `verify-openapi-routes.mjs` skips OPTIONS on the gateway side, because it
    // cannot tell a CDK-generated preflight (task 0126's `addCorsPreflight`)
    // from a deliberately mapped one. That makes a documented OPTIONS
    // uncheckable against the gateway in either direction, so it is rejected
    // here instead of passing over in silence.
    let documented: Vec<&String> = spec["paths"]
        .as_object()
        .expect("spec has a paths object")
        .iter()
        .filter(|(_, ops)| ops.get("options").is_some())
        .map(|(path, _)| path)
        .collect();

    assert!(
        documented.is_empty(),
        "OPTIONS operations are documented but cannot be compared against the \
         gateway: {documented:?} — stop documenting them, or teach \
         verify-openapi-routes.mjs to tell a mapped OPTIONS from a preflight"
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
async fn spec_response_is_cacheable_but_revalidates_within_the_gateway_ttl() {
    let (_, cache_control, _) = fetch_spec(&config_with(None, vec![])).await;

    // Deliberately SHORTER than the 3600s gateway TTL on this route
    // (api-gateway-stack.ts). The gateway entry is dropped when a deployment
    // ships (`make -C infra flush-production-cache`); a partner's HTTP cache is
    // not, so the client-facing window is the one that has to be short or a
    // reader keeps generating clients from the previous build's document. See
    // cache_control::DEPLOY_STATIC.
    assert_eq!(cache_control.as_deref(), Some("public, max-age=300"));
}
