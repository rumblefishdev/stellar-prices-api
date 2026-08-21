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
    // The same guard the Lambda uses, and it matters at least as much here: a
    // local run holds production credentials, so a raised `RUST_LOG` would print
    // real key values into a terminal with scrollback. See
    // `prices_api::telemetry`, and `tests/telemetry_filter.rs` for what it was
    // measured against.
    tracing_subscriber::fmt()
        .with_env_filter(prices_api::telemetry::env_filter())
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

    // Self-service key issuance (task 0187). This build has no Parameters and
    // Secrets extension client, so the plan id comes from `PORTAL_FREE_PLAN_ID`
    // — a local-only variable that is compiled out of the Lambda. The AWS
    // credentials are whatever the ambient profile provides, and they are real:
    //
    //     PORTAL_ENABLED=true PORTAL_OAUTH_SECRET_FILE=.portal-oauth.json \
    //       PORTAL_FREE_PLAN_ID=<plan id> AWS_PROFILE=<profile> \
    //       cargo run -p prices-api --features local-server --bin serve
    //
    // **Every key this creates and deletes is a production key** — there is one
    // environment (task 0183's module docs) and `PORTAL_ENABLED=false` protects
    // the Lambda, not a laptop holding production credentials. Exercise the
    // reconciler against keys this task created and nothing else; task 0194
    // cleans up.
    config
        .load_portal_keys()
        .await
        .expect("failed to configure portal key issuance");

    // The eligibility gate (task 0189). This build has no Parameters and
    // Secrets extension client, so both knobs come from the local-only seams,
    // compiled out of the Lambda like `PORTAL_FREE_PLAN_ID`:
    //
    //     PORTAL_GUILD_ID=<guild snowflake> PORTAL_MIN_ACCOUNT_AGE_MINUTES=5
    //
    // Point `DISCORD_API_BASE` at a mock (or use the real one) — the member
    // check runs against whatever Discord the sign-in does.
    config
        .load_portal_eligibility()
        .await
        .expect("failed to configure the portal eligibility gate");

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
