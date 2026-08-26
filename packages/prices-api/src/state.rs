//! Shared application state handed to every handler as axum `State`.

use clickhouse::Client;
use std::sync::Arc;
use tokio::sync::OnceCell;

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
    /// Resolved reference `asset_id`s for canonical USDC / native XLM /
    /// canonical USDT, memoized for the life of this state.
    ///
    /// These are compile-time-constant *identities* whose surrogate ids never
    /// change, but `resolve_asset_id` is a `SELECT … FROM assets FINAL` per
    /// call. `/ohlcv` needs all three on every USD request, so without this the
    /// endpoint issued three extra `FINAL` lookups per request — 300/s at task
    /// 0121's sustained 100 req/s, purely to re-learn values that never move.
    ///
    /// Scoped to the `AppState`, deliberately **not** a process-wide static: the
    /// integration tests build a fresh state per database, and the same identity
    /// resolves to a different `asset_id` in each. A global would leak one
    /// test's ids into another.
    usd_refs: Arc<OnceCell<crate::assets::queries_ch::UsdRefs>>,
}

impl AppState {
    /// Construct with a live ClickHouse client (the cold-start path).
    pub fn new(ch: Client) -> Self {
        Self {
            ch: Some(ch),
            usd_refs: Arc::new(OnceCell::new()),
        }
    }

    /// Memoized reference ids; resolved once per state, then shared.
    pub fn usd_refs(&self) -> &OnceCell<crate::assets::queries_ch::UsdRefs> {
        &self.usd_refs
    }

    /// Construct without a ClickHouse client — for `/health` smoke tests and
    /// local runs of CH-free routes.
    pub fn without_ch() -> Self {
        Self {
            ch: None,
            usd_refs: Arc::new(OnceCell::new()),
        }
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
