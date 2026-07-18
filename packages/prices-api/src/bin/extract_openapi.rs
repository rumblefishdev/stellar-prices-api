//! Emit the OpenAPI spec as pretty JSON to stdout.
//!
//! Consumes only the library's `register_routes`, so the emitted spec always
//! matches the live routes. Build with default features (no AWS/Lambda stack):
//!   cargo run -p prices-api --bin extract_openapi > openapi.json

fn main() {
    let (_, spec) = prices_api::openapi::register_routes().split_for_parts();
    println!(
        "{}",
        spec.to_pretty_json()
            .expect("failed to serialize OpenAPI spec as pretty JSON")
    );
}
