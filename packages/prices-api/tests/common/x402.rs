//! What the paid routes' tests share (task 0356): an x402 facilitator on
//! loopback and the buyer's side of a request. Used by `tests/x402.rs` and
//! `tests/x402_it.rs` via `#[path]`, as `mock_discord.rs` is.
//!
//! The facilitator answers the three routes a seller calls: `GET /supported`,
//! `POST /verify` and `POST /settle`. It finds every payment valid and settles
//! every one, and **records the paths it was asked**, so a test asserts what
//! the service did: above all that a refused request never reached `/settle`.
// Each binary uses a subset of these helpers.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use prices_api::x402::{self, Settings};
use prices_api::{AppConfig, AppState, app};
use serde_json::{Value, json};
use tower::ServiceExt;

pub const PAY_TO: &str = "GCH5WHIRNCHA2J23YDM2O2SYDBCAHUJ6YKTLM74FEI7DV4IOJR4N5WEN";
pub const USDC: &str = "USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";
pub const AQUA: &str = "AQUA:GBNZILSTVQZ4R7IKQDGHYGY2QXL5QOFJYQMXPKWRRM5PAV7Y4M67AQUA";
/// Canonical USDC's own contract address: a listed asset, named another way.
pub const USDC_CONTRACT: &str = "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75";

pub fn settings(facilitator: &MockFacilitator, network: &str) -> Settings {
    Settings {
        pay_to: PAY_TO.to_owned(),
        network: network.to_owned(),
        facilitator_url: facilitator.url.clone(),
        price_usd: "0.001".to_owned(),
        price_heavy_usd: "0.005".to_owned(),
        sandbox_assets: vec!["native".to_owned(), USDC.to_owned()],
    }
}

pub fn config(api_keys: &[&str]) -> AppConfig {
    AppConfig {
        ch_enabled: false,
        base_url: Some("https://prices-api.example.com".to_owned()),
        api_keys: api_keys.iter().map(|key| (*key).to_owned()).collect(),
        portal_enabled: false,
        portal_oauth: None,
        portal_endpoints: Default::default(),
        portal_keys: None,
        portal_eligibility: None,
        portal_rate_limit: None,
        portal_web_origin: None,
    }
}

/// The application as the Lambda builds it, with the paid routes mounted.
pub fn seller(settings: Option<&Settings>, config: &AppConfig, state: AppState) -> Router {
    x402::mount(app(config, state.clone()), settings, config, state)
}

/// A request to `uri`: `POST` with `body` as JSON when there is one, else `GET`.
pub fn request(uri: &str, body: Option<&Value>, payment: Option<&str>) -> Request<Body> {
    let mut request = Request::builder().uri(uri);
    if let Some(payment) = payment {
        request = request.header("PAYMENT-SIGNATURE", payment);
    }
    match body {
        Some(body) => request
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => request.body(Body::empty()).unwrap(),
    }
}

pub async fn send(seller: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    let response = seller.clone().oneshot(request).await.unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A base64 JSON header, such as `PAYMENT-REQUIRED` or `PAYMENT-RESPONSE`.
pub fn decoded(headers: &HeaderMap, name: &str) -> Value {
    let header = headers.get(name).unwrap_or_else(|| panic!("no {name}"));
    serde_json::from_slice(&STANDARD.decode(header.as_bytes()).unwrap()).unwrap()
}

/// The 402's terms, decoded from `PAYMENT-REQUIRED`.
pub fn required(headers: &HeaderMap) -> Value {
    decoded(headers, "payment-required")
}

/// Asks for the terms, then sends the request again with a payment for them.
pub async fn pay(
    seller: &Router,
    uri: &str,
    body: Option<&Value>,
) -> (StatusCode, HeaderMap, Value) {
    let (status, headers, _) = send(seller, request(uri, body, None)).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{uri}");
    let payment =
        json!({ "x402Version": 2, "accepted": required(&headers)["accepts"][0], "payload": {} });
    let payment = STANDARD.encode(payment.to_string());
    send(seller, request(uri, body, Some(&payment))).await
}

pub const PAYER: &str = "GAZQTB6FZ6O7GPATG3OXCV5PPI3Y62RVF7GEKXF3P53VJPLKLBQSZFKG";
pub const TRANSACTION: &str = "6bd72a6ef6a77a7dc08e501bd4849b6c28ad88cceabb51a4fd105673949412a2";

#[derive(Clone)]
pub struct MockFacilitator {
    pub url: String,
    asked: Arc<Mutex<Vec<String>>>,
    down: Arc<Mutex<bool>>,
}

impl MockFacilitator {
    /// Settles the `exact` scheme on both networks.
    pub async fn start() -> Self {
        let mock = Self {
            url: String::new(),
            asked: Arc::default(),
            down: Arc::default(),
        };
        let app = Router::new()
            .route("/supported", get(answer))
            .route("/verify", post(answer))
            .route("/settle", post(answer))
            .with_state(mock.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { url, ..mock }
    }

    /// Answers `/supported` with 503 from now on, as a facilitator being deployed does.
    pub fn go_down(&self) {
        *self.down.lock().unwrap() = true;
    }

    /// The paths asked so far, in order.
    pub fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }

    pub fn settled(&self) -> usize {
        self.asked()
            .iter()
            .filter(|path| *path == "/settle")
            .count()
    }
}

async fn answer(State(mock): State<MockFacilitator>, uri: Uri) -> impl IntoResponse {
    mock.asked.lock().unwrap().push(uri.path().to_owned());
    let kind = |network: &str| {
        json!({
            "x402Version": 2,
            "scheme": "exact",
            "network": network,
            "extra": { "areFeesSponsored": true }
        })
    };
    match uri.path() {
        "/supported" if *mock.down.lock().unwrap() => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "shutting_down" })),
        ),
        "/supported" => (
            StatusCode::OK,
            Json(json!({ "kinds": [kind("stellar:testnet"), kind("stellar:pubnet")] })),
        ),
        "/verify" => (
            StatusCode::OK,
            Json(json!({ "isValid": true, "payer": PAYER })),
        ),
        _ => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "payer": PAYER,
                "transaction": TRANSACTION,
                "network": "stellar:testnet"
            })),
        ),
    }
}
