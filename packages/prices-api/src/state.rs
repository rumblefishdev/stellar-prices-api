//! Shared application state handed to every handler as axum `State`.

use clickhouse::Client;

/// Cheaply-cloneable state shared across all handlers and warm Lambda
/// invocations.
///
/// `clickhouse::Client` is `Arc`-backed, so cloning [`AppState`] shares the warm
/// hyper/mTLS connection pool built once at cold start (ADR 0007) — never
/// rebuild it per request. `ch` is optional so local/test builds can run routes
/// that don't touch ClickHouse (e.g. `/health`) without a client. Mirrors BE's
/// `AppState { ch: Option<clickhouse::Client> }`.
#[derive(Clone)]
pub struct AppState {
    ch: Option<Client>,
}

impl AppState {
    /// Construct with a live ClickHouse client (the cold-start path).
    pub fn new(ch: Client) -> Self {
        Self { ch: Some(ch) }
    }

    /// Construct without a ClickHouse client — for `/health` smoke tests and
    /// local runs of CH-free routes.
    pub fn without_ch() -> Self {
        Self { ch: None }
    }

    /// Borrow the ClickHouse client.
    ///
    /// Panics with a clear message if a handler reaches the CH path but no
    /// client was built at cold start (a config error — e.g. `CH_ENABLED=false`
    /// on a data route), surfacing it loudly rather than as a silent 500.
    /// Mirrors BE's `AppState::ch`.
    pub(crate) fn ch(&self) -> &Client {
        self.ch.as_ref().expect(
            "ClickHouse client not initialised (CH_ENABLED=false?) but a CH route was reached",
        )
    }
}
