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

    let config = AppConfig::from_env();

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
