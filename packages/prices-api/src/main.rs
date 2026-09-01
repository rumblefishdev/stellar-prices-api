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

    // The portal's three sources, read through the same Parameters & Secrets
    // extension as the mTLS bundle below — so no secret VALUE is ever an
    // environment variable (ADR 0007, Tranche 3 AC 6):
    //
    // - sign-in credentials (task 0186): the Discord OAuth secret;
    // - key issuance (task 0187): the `pricing-api-free` usage-plan id from
    //   SSM, plus the API Gateway control-plane client built from the
    //   execution role's credentials;
    // - the eligibility gate (task 0189): the SSM parameter NAMES for the guild
    //   id and the minimum account age — resolved per issuance so operator
    //   changes need no redeploy — each probed once here so a mis-seeded
    //   parameter is found now and not at a visitor's click.
    //
    // A no-op while `PORTAL_ENABLED` is false; with it true (task 0194) all
    // three are read at every cold start, and the reads are four: one secret,
    // three parameters. See the deploy-gate note on `PORTAL_ENABLED` in
    // `compute-stack.ts`.
    //
    // **Closed, not crashed.** A failed read closes the portal in this
    // execution environment and is logged here; it does not panic init. This
    // Lambda also serves `/v1`, and an init panic is a `502` to the next data
    // API caller — for sources `/v1` never uses, read with no retry against a
    // 40 TPS account-wide Parameter Store budget. The reasoning and the cost
    // are on `AppConfig::load_portal_or_close`. The log line below is one
    // signal a misconfigured or throttled deploy leaves; `/config` answering
    // `enabled: false` is the other, and it is the probe the deploy runbook
    // makes.
    if let Err(err) = config.load_portal_or_close().await {
        tracing::error!(
            error = %err,
            "portal closed at cold start: a portal source failed to load; /v1 is \
             unaffected, and the portal answers as closed in this execution \
             environment until it is recycled"
        );
    }

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
