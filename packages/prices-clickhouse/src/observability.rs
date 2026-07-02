//! Shared observability setup for the prices-api worker Lambdas.
//!
//! The tracing-subscriber init block was byte-identical across every worker
//! `main.rs` (asset-discovery, cleanup, oracle, supply, enrichment, and the two
//! task-0056 probes). Hoisting it here keeps the fleet's log schema in one place
//! so a change (a common field, span events, toggling `.json()`) is one edit,
//! not a per-worker clone that silently drifts.

/// Initialise the JSON tracing subscriber for a Lambda entrypoint: structured
/// (`.json()`) output filtered by `RUST_LOG` (via `EnvFilter::from_default_env`,
/// defaulting to the crate's compiled-in level when unset). Call once at cold
/// start, before building clients. Panics if a global subscriber is already set
/// (same semantics as calling `.init()` directly) — a Lambda inits exactly once.
pub fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();
}
