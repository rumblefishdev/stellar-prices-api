//! Local HTTP server for development + load testing — NOT the Lambda
//! entrypoint (that's `src/main.rs`, behind the `lambda` feature).
//!
//! Serves the exact same [`prices_api::app`] router over a TCP listener,
//! backed by a plaintext local ClickHouse, so a load generator (k6) can hit a
//! real server. Gated behind the `local-server` feature so it never affects the
//! lean Lambda build.
//!
//! ```sh
//! docker compose up -d clickhouse
//! # seed a price row (see loadtest/seed.sql), then:
//! CLICKHOUSE_URL=http://localhost:8123 PORT=8080 \
//!   cargo run -p prices-api --bin serve --features local-server
//! ```

use prices_api::{AppConfig, AppState, app};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Plaintext local CH client (CLICKHOUSE_URL / _USER / _PASSWORD / _DATABASE,
    // defaulting to http://localhost:8123 and database `prices`).
    let ch = prices_clickhouse::client(&prices_clickhouse::Config::from_env());
    let state = AppState::new(ch);
    let mut config = AppConfig::from_env();

    // Portal sign-in (task 0186). This build has no Secrets Manager client, so
    // the only source available here is `PORTAL_OAUTH_SECRET_FILE` — a local
    // JSON file holding a Discord application whose redirect URI points at
    // localhost. That is the configuration the acceptance criteria's local
    // round-trip runs in:
    //
    //     PORTAL_ENABLED=true PORTAL_OAUTH_SECRET_FILE=.portal-oauth.json \
    //       cargo run -p prices-api --features local-server --bin serve
    //
    // Only attempted when the portal is on, so the ordinary local run of the
    // data API needs none of it.
    config
        .load_portal_oauth()
        .await
        .expect("failed to load portal OAuth credentials");

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let addr = format!("0.0.0.0:{port}");

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind");
    tracing::info!("prices-api local server listening on http://{addr}");

    axum::serve(listener, app(&config, state))
        .await
        .expect("server error");
}
