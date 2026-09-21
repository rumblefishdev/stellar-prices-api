//! Coverage sweep probe (task 0100): layer 3 of the coverage model.
//!
//! - Layer 1 is the pool registry itself: what we index.
//! - Layer 2 (`UnregisteredPoolEvents`, task 0291) counts trades the live
//!   processor dropped from venues we index but pools we have not registered.
//! - Layer 3, this crate, looks wider: once a week it finds every contract
//!   that emits swap- or trade-shaped Soroban events and is neither in
//!   `prices.pool_registry` nor on the committed allow-list
//!   (`allowlist.toml`). That residual may be a venue we do not index at all —
//!   the SushiSwap V3 case (task 0290), which traded unseen for months.
//!
//! The sweep **only reports**. It never registers anything: a contract leaves
//! the residual when a human registers it (with an extractor) or allow-lists
//! it (with a reason and a task). Auto-registering routers would double-count
//! the pool-level trades they wrap.
//!
//! The residual is published to `Prices/Coverage` only when non-zero, so the
//! alarm is a plain `UnclassifiedSwapEvents >= 1` over `NOT_BREACHING`.
//! Triage: `docs/runbooks/0100-coverage-sweep-triage.md`.

pub mod allowlist;
pub mod sweep;

pub use allowlist::AllowList;
pub use sweep::{SweepReport, SweepRow};

/// CloudWatch namespace. Must equal the `cloudwatch:namespace` IAM condition
/// on the probe's role and the alarm's namespace in `observability-stack.ts`.
pub const METRIC_NAMESPACE: &str = "Prices/Coverage";
/// Number of unclassified contracts in the window.
pub const UNCLASSIFIED_SWAP_CONTRACTS: &str = "UnclassifiedSwapContracts";
/// Sum of their swap/trade-shaped events in the window. The alarm watches this.
pub const UNCLASSIFIED_SWAP_EVENTS: &str = "UnclassifiedSwapEvents";

/// BE's database (`soroban_events`, `soroban_contracts`).
pub const BE_DATABASE: &str = "default";
/// Our database (`pool_registry`).
pub const PRICES_DATABASE: &str = "prices";

/// Client-side `max_execution_time` for every statement, in seconds. The sweep
/// runs as `prices_writer`, whose profile carries no execution bound.
///
/// A run issues two statements (the window's `max(ledger_sequence)`, then the
/// sweep), each bounded separately, inside the 120 s Lambda timeout — so the
/// bound must be under half of it for a slow run to end in a ClickHouse
/// `TIMEOUT_EXCEEDED` (a logged error carrying the CH code) rather than a
/// Lambda kill (counted in `Errors` too, but with no cause in the log).
/// 2 × 50 s = 100 s < 120 s; 50 s is ~5× the measured 9.6 s (review WR-05).
pub const SWEEP_MAX_EXECUTION_SECS: u64 = 50;

/// One datapoint to publish.
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub name: &'static str,
    pub value: f64,
}

/// The residual's datapoints: nothing when nothing is unclassified, else the
/// contract count and the event sum. Absent-while-healthy, like the ledger
/// processor's `unregistered_pool_event_metrics`.
pub fn unclassified_metrics(unclassified: &[SweepRow]) -> Vec<Metric> {
    if unclassified.is_empty() {
        return Vec::new();
    }
    let events: u64 = unclassified.iter().map(|r| r.events).sum();
    vec![
        Metric {
            name: UNCLASSIFIED_SWAP_CONTRACTS,
            value: unclassified.len() as f64,
        },
        Metric {
            name: UNCLASSIFIED_SWAP_EVENTS,
            value: events as f64,
        },
    ]
}

/// Publish `metrics` under [`METRIC_NAMESPACE`] with an `Environment`
/// dimension, in one `PutMetricData`. A no-op on an empty slice:
/// `PutMetricData` with no data is an API error.
#[cfg(feature = "lambda")]
pub async fn publish(
    client: &aws_sdk_cloudwatch::Client,
    environment: &str,
    metrics: &[Metric],
) -> Result<(), aws_sdk_cloudwatch::Error> {
    use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};

    if metrics.is_empty() {
        return Ok(());
    }

    let env_dim = Dimension::builder()
        .name("Environment")
        .value(environment)
        .build();

    let data = metrics
        .iter()
        .map(|m| {
            MetricDatum::builder()
                .metric_name(m.name)
                .value(m.value)
                .unit(StandardUnit::Count)
                .dimensions(env_dim.clone())
                .build()
        })
        .collect::<Vec<_>>();

    client
        .put_metric_data()
        .namespace(METRIC_NAMESPACE)
        .set_metric_data(Some(data))
        .send()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(events: u64) -> SweepRow {
        SweepRow {
            strkey: "C".into(),
            wasm: None,
            contract_surrogate: 1,
            events,
            txs: 1,
            first_ledger: 1,
            last_ledger: 1,
            top_action: String::new(),
            top_shape: String::new(),
        }
    }

    #[test]
    fn nothing_unclassified_publishes_nothing() {
        assert!(unclassified_metrics(&[]).is_empty());
    }

    #[test]
    fn residual_publishes_count_and_event_sum() {
        let m = unclassified_metrics(&[row(730), row(63)]);
        assert_eq!(
            m,
            vec![
                Metric {
                    name: UNCLASSIFIED_SWAP_CONTRACTS,
                    value: 2.0
                },
                Metric {
                    name: UNCLASSIFIED_SWAP_EVENTS,
                    value: 793.0
                },
            ]
        );
    }
}
