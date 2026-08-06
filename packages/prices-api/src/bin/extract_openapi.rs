//! Emit the OpenAPI spec as pretty JSON to stdout.
//!
//! Consumes only the library's `register_routes` + `stamp_servers`, so the
//! emitted document is the same one the deployed API serves at
//! `GET /api-docs-json` for the same configuration — which is what makes
//! linting this output meaningful. Set `API_BASE_URL` to stamp `servers`
//! (required for a clean lint; see `npm run openapi:lint`). Build with default
//! features (no AWS/Lambda stack):
//!   API_BASE_URL=https://…/production \
//!     cargo run -p prices-api --bin extract_openapi > openapi.json

fn main() {
    let config = prices_api::AppConfig::from_env();
    let (_, mut spec) = prices_api::openapi::register_routes().split_for_parts();
    prices_api::openapi::stamp_servers(&mut spec, &config);
    println!(
        "{}",
        spec.to_pretty_json()
            .expect("failed to serialize OpenAPI spec as pretty JSON")
    );
}
