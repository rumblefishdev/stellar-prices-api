//! Lambda entrypoint for the Prices API.
//!
//! Only compiled with `--features lambda` (see `required-features` in
//! Cargo.toml). Boots the JSON tracing subscriber, builds the ClickHouse mTLS
//! client once at cold start (so the warm connection pool is primed before the
//! first request, ADR 0007 / BE pattern), and hands the shared
//! [`prices_api::app`] router to the Lambda HTTP runtime.

use prices_api::{AppConfig, AppState, app};

#[tokio::main]
async fn main() {
    // `telemetry::env_filter`, not `EnvFilter::from_default_env`: the AWS SDK
    // prints a `GetApiKey` response — key value and all — at `trace`, and
    // `RUST_LOG` is one `UpdateFunctionConfiguration` away from being `trace`.
    // The filter drops every directive that could outrank the pins — the ones
    // aimed at those crates, and every span/field directive, which would
    // otherwise enable the same events by scope — then pins the crates at
    // `info`. `tests/telemetry_filter.rs` is the measurement that this holds for
    // the hostile `RUST_LOG` values, rather than the claim that it does.
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(prices_api::telemetry::env_filter())
        .with_target(false)
        .init();

    let mut config = AppConfig::from_env();

    // Portal sign-in credentials (task 0186), read through the same Parameters &
    // Secrets extension as the mTLS bundle below — so no secret VALUE is ever an
    // environment variable (ADR 0007, Tranche 3 AC 6).
    //
    // A no-op while `PORTAL_ENABLED` is false, which is production for the whole
    // of the portal's build: see `AppConfig::load_portal_oauth` for why loading
    // it unconditionally would let a missing portal secret take out `/v1`. With
    // the portal open, a missing or malformed secret fails init on purpose —
    // that surfaces as `Init Errors` at deploy rather than as a `503` under a
    // sign-in button.
    config
        .load_portal_oauth()
        .await
        .expect("failed to load portal OAuth credentials at cold start");

    // Self-service key issuance (task 0187). Reads the `pricing-api-free`
    // usage-plan id from SSM through the same extension, and builds the API
    // Gateway control-plane client from the execution role's credentials.
    //
    // A no-op while `PORTAL_ENABLED` is false, which does double duty: it keeps
    // two operations off the cold-start path of `/v1`, and it means a closed
    // portal has no control-plane client in the process at all — so no code path
    // in this build can create or delete a production API key.
    config
        .load_portal_keys()
        .await
        .expect("failed to configure portal key issuance at cold start");

    // The eligibility gate (task 0189). Stores the SSM parameter names for the
    // guild id and the minimum account age — resolved per issuance so operator
    // changes need no redeploy — and probes both once, so a mis-seeded
    // parameter fails here in `Init Errors` rather than at a visitor's click.
    // A no-op while `PORTAL_ENABLED` is false, like the two loads above.
    config
        .load_portal_eligibility()
        .await
        .expect("failed to configure the portal eligibility gate at cold start");

    // Build the CH client eagerly at cold start; it is Arc-backed and shared via
    // AppState across warm invocations. `client_from_lambda_env` reads
    // MTLS_SECRET_NAME + CH_DOMAIN (set by CDK) and fetches the cert bundle from
    // the Parameters & Secrets Lambda Extension.
    let state = if config.ch_enabled {
        let ch = prices_clickhouse::mtls::client_from_lambda_env(prices_clickhouse::PROD_DATABASE)
            .await
            .expect("failed to build mTLS ClickHouse client at cold start");
        AppState::new(ch)
    } else {
        AppState::without_ch()
    };

    lambda_http::run(app(&config, state))
        .await
        .expect("failed to run Lambda");
}
