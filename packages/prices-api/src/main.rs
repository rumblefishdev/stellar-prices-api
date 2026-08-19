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
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
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
