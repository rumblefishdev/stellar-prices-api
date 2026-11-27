//! Paid twins of three data routes: a caller with no API key pays USDC for one
//! request through x402 and gets the answer (task 0356; the design is
//! stellar-x402's task 0041).
//!
//! | Route                                          | Twin of                      |
//! | ---------------------------------------------- | ---------------------------- |
//! | `GET /x402/v1/assets/{asset_identifier}/price` | `GET /v1/assets/{…}/price`   |
//! | `GET /x402/v1/assets/{asset_identifier}/ohlcv` | `GET /v1/assets/{…}/ohlcv`   |
//! | `POST /x402/v1/prices/batch`                   | `POST /v1/prices/batch`      |
//!
//! Each is the keyed route's own handler behind the `stellar-x402` crate's
//! payment layer. Without a payment the layer answers `402` with the terms and
//! the handler does not run. With one, the facilitator verifies it, the handler
//! runs, and only an answer below `400` is settled. So three things are turned
//! into a `4xx` here, each before any money moves:
//!
//! - an answer with nothing in it ([`Nothing`]), which the keyed route gives
//!   as a `200`: nobody pays for an empty answer;
//! - a request outside the [`Sandbox`], while payments are in testnet USDC;
//! - whatever the keyed handler already refuses.
//!
//! The routes are added to the application AFTER `auth::apply`, so the in-app
//! key gate does not cover them: these three are keyless by construction, and
//! that gate's exemptions are not touched. They are added one by one with
//! `Router::route`, not merged as a router: `Router::merge` replaces the
//! application's fallback with one the key gate does not wrap, and an unknown
//! path would then answer `404` where it answers `401` today (measured in
//! `tests/x402.rs`). They exist only when `X402_PAY_TO` is set
//! ([`Settings::from_env`]).
//!
//! The facilitator is asked on the first paid-route request, never at cold
//! start (`PaymentGate::lazy`): a facilitator outage costs the paid routes a
//! `5xx` and the keyed routes nothing.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{MethodRouter, get, post};
use axum::{Extension, Router};
use serde_json::json;
use stellar_x402::axum::{PaidRoute, PaymentEvent, PaymentGate};
use stellar_x402::{Facilitator, Listing, Network, Price};

use crate::assets::handlers::{OhlcvParams, PriceParams, get_ohlcv, get_price};
use crate::assets::queries_ch::Timeframe;
use crate::batch::dto::{BatchRequest, MAX_BATCH_BODY_BYTES};
use crate::batch::handlers::post_batch;
use crate::common::extract::{ValidatedJson, ValidatedPath, ValidatedQuery};
use crate::common::{cache_control, errors};
use crate::config::AppConfig;
use crate::state::AppState;

/// The longest one call to the facilitator may take. A settlement takes about
/// 6 s; API Gateway gives the whole request 29 s.
const FACILITATOR_TIMEOUT: Duration = Duration::from_secs(25);
/// The most assets one sandbox batch may ask for.
const SANDBOX_BATCH: usize = 5;

type Error = Box<dyn std::error::Error + Send + Sync>;

/// The paid routes' settings, from the environment.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The Stellar account that is paid. The service holds no key for it.
    pub pay_to: String,
    /// `stellar:testnet` or `stellar:pubnet`: the network the USDC is paid on.
    pub network: String,
    pub facilitator_url: String,
    /// USD for one `price` request, as a decimal.
    pub price_usd: String,
    /// USD for one `ohlcv` or `batch` request.
    pub price_heavy_usd: String,
    /// The assets the sandbox sells; read only while `network` is testnet.
    pub sandbox_assets: Vec<String>,
}

impl Settings {
    /// `None` without `X402_PAY_TO`: the paid routes then do not exist.
    pub fn from_env() -> Option<Self> {
        let var = |name: &str, default: &str| {
            let value = std::env::var(name).unwrap_or_default();
            match value.trim() {
                "" => default.to_owned(),
                value => value.to_owned(),
            }
        };
        let pay_to = var("X402_PAY_TO", "");
        if pay_to.is_empty() {
            return None;
        }
        Some(Self {
            pay_to,
            network: var("X402_NETWORK", "stellar:testnet"),
            facilitator_url: var(
                "X402_FACILITATOR_URL",
                "https://x402-facilitator.sorobanscan.com",
            ),
            price_usd: var("X402_PRICE_USD", "0.001"),
            price_heavy_usd: var("X402_PRICE_HEAVY_USD", "0.005"),
            sandbox_assets: var(
                "X402_SANDBOX_ASSETS",
                "native,USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN",
            )
            .split(',')
            .map(|asset| asset.trim().to_owned())
            .filter(|asset| !asset.is_empty())
            .collect(),
        })
    }
}

/// `app` with the paid routes, when there are settings and they are valid.
///
/// A malformed setting (an address, a price, the network, a facilitator URL that
/// does not parse) is logged and leaves `app` as it was; a well-formed wrong
/// one mounts the routes, which then answer `5xx`. The keyed routes must start whatever is
/// wrong here.
pub fn mount(
    app: Router,
    settings: Option<&Settings>,
    config: &AppConfig,
    state: AppState,
) -> Router {
    let Some(settings) = settings else {
        return app;
    };
    match routes(settings, config, state) {
        Ok(paid) => paid
            .into_iter()
            .fold(app, |app, (path, route)| app.route(path, route)),
        Err(error) => {
            tracing::error!(%error, "x402: a setting is wrong, the paid routes are not served");
            app
        }
    }
}

/// The three paid routes, each behind its price.
fn routes(
    settings: &Settings,
    config: &AppConfig,
    state: AppState,
) -> Result<[(&'static str, MethodRouter); 3], Error> {
    let network = match settings.network.as_str() {
        "stellar:testnet" => Network::Testnet,
        "stellar:pubnet" => Network::Pubnet,
        other => return Err(format!("X402_NETWORK is {other:?}").into()),
    };
    let facilitator =
        Facilitator::new(&settings.facilitator_url)?.with_timeout(FACILITATOR_TIMEOUT);
    let mut gate = PaymentGate::lazy(facilitator).on_event(log);
    // The 402 and the catalog's listing name each route by this address.
    if let Some(base_url) = &config.base_url {
        gate = gate.base_url(base_url);
    }
    // Free money buys a sandbox, never the product.
    let sandbox =
        Sandbox((network == Network::Testnet).then(|| settings.sandbox_assets.clone().into()));
    // The payment layer goes on last, so it is the outermost: an unpaid request
    // is answered before the body is read or the handler runs.
    let paid = |route: MethodRouter<AppState>,
                usd: &str,
                description: &str,
                listing: Listing|
     -> Result<MethodRouter, Error> {
        let price: PaidRoute = gate
            .charge(network, &settings.pay_to, Price::usdc(usd)?)?
            .description(description)
            .mime_type("application/json")
            .listing(listing);
        Ok(route
            .layer::<_, Infallible>(Extension(sandbox.clone()))
            .layer::<_, Infallible>(DefaultBodyLimit::max(MAX_BATCH_BODY_BYTES))
            .route_layer(price)
            .with_state(state.clone()))
    };

    let price_listing = Listing::query(json!({})).output(json!({
        "asset": "native",
        "price_usd": "0.2731",
        "price_xlm": "1",
        "vwap_24h": "0.2725",
        "volume_24h_usd": "1843021.55",
        "change_24h_pct": "-1.42",
        "updated_at": "2026-11-27T12:00:00Z",
        "price_status": "priced"
    }));
    let ohlcv_listing = Listing::query(json!({ "timeframe": "24h" })).output(json!({
        "asset": "native",
        "granularity": "15m",
        "base_currency": "USD",
        "data": [{
            "timestamp": "2026-11-27T12:00:00Z",
            "open": "0.2729",
            "high": "0.2735",
            "low": "0.2727",
            "close": "0.2731",
            "volume_base": "51234.5",
            "volume_quote_usd": "13990.1",
            "vwap": "0.2730",
            "trade_count": 42
        }]
    }));
    let batch_listing = Listing::json_body(json!({ "assets": ["native"] }))
        .input_schema(json!({
            "type": "object",
            "properties": { "assets": { "type": "array", "items": { "type": "string" } } },
            "required": ["assets"]
        }))
        .output(json!({
            "prices": [{ "asset": "native", "price_usd": "0.2731", "price_status": "priced" }],
            "not_found": []
        }));

    Ok([
        (
            "/x402/v1/assets/{asset_identifier}/price",
            paid(
                get(price),
                &settings.price_usd,
                "Current USD and XLM price of a Stellar asset, with its 24-hour volume and change",
                price_listing,
            )?,
        ),
        (
            "/x402/v1/assets/{asset_identifier}/ohlcv",
            paid(
                get(ohlcv),
                &settings.price_heavy_usd,
                "OHLCV candles of a Stellar asset in USD",
                ohlcv_listing,
            )?,
        ),
        (
            "/x402/v1/prices/batch",
            paid(
                post(batch),
                &settings.price_heavy_usd,
                "Current prices of several Stellar assets in one call",
                batch_listing,
            )?,
        ),
    ])
}

/// What happened to a payment, for the logs. The transaction's hash and the
/// facilitator's reason codes only: a payer's payment is never logged.
fn log(event: &PaymentEvent<'_>) {
    match event {
        PaymentEvent::Settled(settled) => {
            let catalog = settled
                .extension_responses
                .as_ref()
                .and_then(|answers| answers.get("bazaar"));
            tracing::info!(transaction = %settled.transaction, ?catalog, "x402: payment settled");
        }
        PaymentEvent::SettleRefused(settled) => tracing::warn!(
            reason = ?settled.error_reason,
            transaction = %settled.transaction,
            "x402: the route answered and the payment was not settled"
        ),
        PaymentEvent::VerifyRefused(verified) => {
            tracing::info!(reason = ?verified.invalid_reason, "x402: payment refused");
        }
        PaymentEvent::FacilitatorFailed(error) => {
            tracing::error!(%error, "x402: the facilitator failed");
        }
        PaymentEvent::Unsupported(network) => tracing::error!(
            %network,
            "x402: the facilitator does not settle on X402_NETWORK, every paid request is 500"
        ),
        _ => {}
    }
}

/// What the paid routes sell while payments are in testnet USDC, which is free:
/// a listed asset, `ohlcv` for `1h` and `24h` without `start` or `end`, a batch
/// of at most [`SANDBOX_BATCH`]. `None` on pubnet: the whole product.
///
/// The service has pubnet data only, so without this a caller would get all of
/// it for nothing and nobody would need pubnet.
#[derive(Clone)]
struct Sandbox(Option<Arc<[String]>>);

impl Sandbox {
    /// Whether `asset` may be asked for: on the list exactly as written (a path
    /// value arrives percent-decoded), or `native` in any letter case, as the
    /// keyed routes read it. Nothing else names a listed asset: not its
    /// contract address, another letter case or surrounding spaces. Checked
    /// before the keyed handler runs, so before any query.
    fn sells(&self, asset: &str) -> bool {
        let Some(assets) = &self.0 else {
            return true;
        };
        assets.iter().any(|listed| {
            listed == asset || (listed == "native" && asset.eq_ignore_ascii_case("native"))
        })
    }

    /// The 403 for a request outside the sandbox. Nothing is charged for it.
    fn refusal(&self) -> Response {
        let assets = self.0.as_deref().unwrap_or_default().join(", ");
        errors::refusal(
            StatusCode::FORBIDDEN,
            errors::SANDBOX_SCOPE,
            format!(
                "payments are in testnet USDC, so this route sells a sandbox: the assets \
                 {assets}; ohlcv for timeframe 1h or 24h without start or end; a batch of at \
                 most {SANDBOX_BATCH}"
            ),
        )
    }
}

/// Marks a keyed route's `200` as having nothing in it; the value is the code
/// the paid twin answers `404` with. The keyed route itself ignores it: a
/// response extension is never sent.
#[derive(Clone, Copy)]
pub(crate) struct Nothing(pub(crate) &'static str);

/// The keyed route's answer, as the paid twin sends it: `404` in place of an
/// answer marked [`Nothing`], so the payment layer does not settle, and never
/// stored by a cache, since the next caller has not paid.
fn sold(mut response: Response) -> Response {
    if let Some(Nothing(code)) = response.extensions().get::<Nothing>() {
        return errors::refusal(
            StatusCode::NOT_FOUND,
            code,
            "there is no data to sell for this request, so nothing was charged",
        );
    }
    cache_control::attach(&mut response, cache_control::NO_STORE);
    response
}

async fn price(
    State(state): State<AppState>,
    Extension(sandbox): Extension<Sandbox>,
    ValidatedPath(asset): ValidatedPath<String>,
    ValidatedQuery(params): ValidatedQuery<PriceParams>,
) -> Response {
    if !sandbox.sells(&asset) {
        return sandbox.refusal();
    }
    sold(get_price(State(state), ValidatedPath(asset), ValidatedQuery(params)).await)
}

async fn ohlcv(
    State(state): State<AppState>,
    Extension(sandbox): Extension<Sandbox>,
    ValidatedPath(asset): ValidatedPath<String>,
    ValidatedQuery(params): ValidatedQuery<OhlcvParams>,
) -> Response {
    let narrow = matches!(
        params.timeframe,
        None | Some(Timeframe::H1 | Timeframe::H24)
    ) && params.start.is_none()
        && params.end.is_none();
    if !sandbox.sells(&asset) || (sandbox.0.is_some() && !narrow) {
        return sandbox.refusal();
    }
    sold(get_ohlcv(State(state), ValidatedPath(asset), ValidatedQuery(params)).await)
}

async fn batch(
    State(state): State<AppState>,
    Extension(sandbox): Extension<Sandbox>,
    ValidatedJson(request): ValidatedJson<BatchRequest>,
) -> Response {
    let small = request.assets.len() <= SANDBOX_BATCH;
    if !request.assets.iter().all(|asset| sandbox.sells(asset)) || (sandbox.0.is_some() && !small) {
        return sandbox.refusal();
    }
    sold(post_batch(State(state), ValidatedJson(request)).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    const USDC: &str = "USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";

    #[test]
    fn the_sandbox_sells_a_listed_asset_only_as_it_is_written() {
        let sandbox = Sandbox(Some(vec!["native".to_owned(), USDC.to_owned()].into()));

        for asset in ["native", "Native", "NATIVE", USDC] {
            assert!(sandbox.sells(asset), "{asset}");
        }
        for asset in [
            // Canonical USDC's and XLM's own contract addresses.
            "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75",
            "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA",
            "usdc:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN",
            " native",
            "USDC%3AGA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN",
            "AQUA:GBNZILSTVQZ4R7IKQDGHYGY2QXL5QOFJYQMXPKWRRM5PAV7Y4M67AQUA",
            "",
        ] {
            assert!(!sandbox.sells(asset), "{asset}");
        }
    }

    #[test]
    fn without_a_sandbox_every_asset_is_sold() {
        assert!(
            Sandbox(None).sells("AQUA:GBNZILSTVQZ4R7IKQDGHYGY2QXL5QOFJYQMXPKWRRM5PAV7Y4M67AQUA")
        );
    }

    #[test]
    fn an_answer_marked_as_nothing_is_a_404_and_any_other_is_never_cached() {
        let mut empty = Response::new(axum::body::Body::empty());
        empty.extensions_mut().insert(Nothing(errors::NO_DATA));
        assert_eq!(sold(empty).status(), StatusCode::NOT_FOUND);

        let mut full = Response::new(axum::body::Body::empty());
        cache_control::attach(&mut full, cache_control::SHORT);
        let full = sold(full);
        assert_eq!(full.status(), StatusCode::OK);
        assert_eq!(full.headers()["cache-control"], "no-store");
    }
}
