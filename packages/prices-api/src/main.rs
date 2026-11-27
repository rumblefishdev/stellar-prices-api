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

    let config = AppConfig::from_env();

    // The cold start reads one source: the mTLS bundle below. The portal's
    // five (the Discord OAuth secret and four SSM parameters) load on the
    // first portal request that needs them (`prices_api::portal::sources`),
    // so a burst of `/v1` cold starts makes no Parameter Store read, and a
    // failed portal read costs that one request rather than the environment
    // (task 0311).

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

    let app = app(&config, state).layer(axum::middleware::from_fn(within_its_resource));
    lambda_http::run(app).await.expect("failed to run Lambda");
}

/// Refuses a request whose path, once `lambda_http` resolved its dot segments,
/// left the gateway resource it came through: `/api/../v1/...` would otherwise
/// reach a keyed route through a keyless resource (task 0357).
async fn within_its_resource(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use lambda_http::request::RequestContext;
    if let Some(RequestContext::ApiGatewayV1(context)) = request.extensions().get()
        && let Some(resource) = context.resource_path.as_deref()
        && prices_api::common::gateway::left_its_resource(resource, request.uri().path())
    {
        tracing::warn!(
            resource,
            path = request.uri().path(),
            "a path left the gateway resource it came through"
        );
        return prices_api::common::errors::not_found("no such route");
    }
    next.run(request).await
}
