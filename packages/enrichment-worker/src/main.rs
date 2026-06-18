//! Lambda entrypoint — EventBridge Scheduler handler.
//!
//! The Scheduler payload is **ignored**; the Lambda runs the
//! enrichment pass driven entirely by env-var config and the
//! fixture files (in prototype mode). Production swaps the
//! fixtures for CH queries; the handler shape stays the same.
//!
//! Cold-start eager init mirrors BE's indexer pattern: missing env
//! / unreachable fixture surfaces as a Lambda Init Errors entry,
//! not a per-event panic.

use std::path::PathBuf;
use std::sync::Arc;

use enrichment_worker::candidates::JsonlCandidateSource;
use enrichment_worker::ch_enrich::{ChEnrichConfig, ChEnrichmentPass};
use enrichment_worker::oracle::InMemoryOracleLookup;
use enrichment_worker::pass::run_pass;
use enrichment_worker::sink::StdoutJsonSink;
use lambda_runtime::{Error, LambdaEvent, service_fn};
use tokio::sync::Mutex;
use tracing::{error, info};

const ENV_CANDIDATES: &str = "CANDIDATES_FIXTURE";
const ENV_ORACLE_FIXTURE: &str = "ORACLE_FIXTURE";
const ENV_ORACLE_NAME: &str = "ORACLE_NAME";
const ENV_WINDOW_S: &str = "FORWARD_FILL_WINDOW_S";
const ENV_PIVOT_WINDOW_S: &str = "PIVOT_WINDOW_S";
const ENV_BATCH_SIZE: &str = "BATCH_SIZE";
const ENV_MAX_BATCHES: &str = "MAX_BATCHES";

// Production (CH Form-B) selector + connection. When `CLICKHOUSE_URL`
// is set the Lambda runs the batch ASOF-JOIN enrichment against
// ClickHouse; otherwise it falls back to the fixture-driven prototype.
const ENV_CLICKHOUSE_URL: &str = "CLICKHOUSE_URL";
const ENV_CH_DATABASE: &str = "CLICKHOUSE_DATABASE";
const ENV_CH_TABLE: &str = "CLICKHOUSE_TABLE";

#[derive(Clone)]
struct Cfg {
    candidates_path: PathBuf,
    oracle_path: PathBuf,
    oracle_name: String,
    window_s: u32,
    batch_size: usize,
    max_batches: usize,
}

struct State {
    oracle: InMemoryOracleLookup,
    cfg: Cfg,
    /// Mutex around the candidate source — `next_batch` takes `&mut self`.
    /// In prototype Lambda mode each invocation rewinds via load (see handler).
    /// Kept here for future production swap.
    _candidates_path_marker: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    // Production swap: `CLICKHOUSE_URL` present → batch ASOF-JOIN
    // enrichment against ClickHouse (Form B). Absent → fixture prototype.
    if let Ok(url) = std::env::var(ENV_CLICKHOUSE_URL) {
        return run_production(url).await;
    }

    let cfg = Cfg {
        candidates_path: std::env::var(ENV_CANDIDATES)
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("fixtures/candidates.jsonl")),
        oracle_path: std::env::var(ENV_ORACLE_FIXTURE)
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("fixtures/oracle_prices.jsonl")),
        oracle_name: std::env::var(ENV_ORACLE_NAME).unwrap_or_else(|_| "reflector".to_string()),
        window_s: parse_env_or(ENV_WINDOW_S, 300),
        batch_size: parse_env_or(ENV_BATCH_SIZE, 10_000),
        max_batches: parse_env_or(ENV_MAX_BATCHES, 20),
    };

    info!(
        candidates = %cfg.candidates_path.display(),
        oracle = %cfg.oracle_path.display(),
        oracle_name = %cfg.oracle_name,
        window_s = cfg.window_s,
        batch_size = cfg.batch_size,
        max_batches = cfg.max_batches,
        "enrichment-worker cold start"
    );

    let oracle = InMemoryOracleLookup::load_jsonl(&cfg.oracle_path)
        .await
        .map_err(|e| {
            error!(error = %e, "oracle fixture load failed");
            format!("oracle fixture load failed: {e}")
        })?;

    let state = Arc::new(Mutex::new(State {
        oracle,
        cfg: cfg.clone(),
        _candidates_path_marker: cfg.candidates_path.clone(),
    }));

    lambda_runtime::run(service_fn(move |event: LambdaEvent<serde_json::Value>| {
        let s = state.clone();
        async move { handler(event, s).await }
    }))
    .await
}

async fn handler(
    _event: LambdaEvent<serde_json::Value>,
    state: Arc<Mutex<State>>,
) -> Result<serde_json::Value, Error> {
    let state = state.lock().await;
    // Re-open the candidate source each invocation — production
    // form runs one CH query per invocation, equivalent.
    let mut candidates = JsonlCandidateSource::open(&state.cfg.candidates_path)
        .await
        .map_err(|e| format!("candidates open failed: {e}"))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let stats = run_pass(
        &mut candidates,
        &state.oracle,
        &StdoutJsonSink,
        &state.cfg.oracle_name,
        state.cfg.window_s,
        state.cfg.batch_size,
        state.cfg.max_batches,
        now,
    )
    .await
    .map_err(|e| format!("pass failed: {e}"))?;

    info!(
        batches = stats.batches,
        candidates_seen = stats.candidates_seen,
        enriched = stats.rows_enriched,
        misses = stats.oracle_misses,
        "enrichment pass complete"
    );

    Ok(serde_json::json!({
        "batches": stats.batches,
        "candidates_seen": stats.candidates_seen,
        "rows_enriched": stats.rows_enriched,
        "oracle_misses": stats.oracle_misses,
    }))
}

/// Production entrypoint — the CH Form-B path. Builds the enrichment
/// pass at cold start (preflight failures surface as Lambda Init
/// Errors, mirroring the prototype's eager oracle load), then runs one
/// bounded pass per Scheduler event.
async fn run_production(url: String) -> Result<(), Error> {
    let cfg = ChEnrichConfig {
        url,
        database: std::env::var(ENV_CH_DATABASE).unwrap_or_else(|_| "prices".to_string()),
        table: std::env::var(ENV_CH_TABLE).unwrap_or_else(|_| "price_ohlcv_1m".to_string()),
        oracle_name: std::env::var(ENV_ORACLE_NAME).unwrap_or_else(|_| "reflector".to_string()),
        window_s: parse_env_or(ENV_WINDOW_S, 300),
        pivot_window_s: parse_env_or(ENV_PIVOT_WINDOW_S, 86_400),
        batch_size: parse_env_or(ENV_BATCH_SIZE, 10_000),
        max_batches: parse_env_or(ENV_MAX_BATCHES, 20),
    };

    info!(
        url = %cfg.url,
        database = %cfg.database,
        table = %cfg.table,
        oracle_name = %cfg.oracle_name,
        window_s = cfg.window_s,
        pivot_window_s = cfg.pivot_window_s,
        batch_size = cfg.batch_size,
        max_batches = cfg.max_batches,
        "enrichment-worker cold start (clickhouse mode)"
    );

    let pass = ChEnrichmentPass::new(cfg);
    pass.preflight().await.map_err(|e| {
        error!(error = %e, "clickhouse preflight failed");
        format!("clickhouse preflight failed: {e}")
    })?;

    let pass = Arc::new(pass);
    lambda_runtime::run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let pass = pass.clone();
        async move {
            let stats = pass
                .run()
                .await
                .map_err(|e| format!("enrichment pass failed: {e}"))?;
            Ok::<_, Error>(serde_json::json!({
                "batches": stats.batches,
                "candidates_before": stats.candidates_before,
                "candidates_after": stats.candidates_after,
                "rows_enriched": stats.rows_enriched,
            }))
        }
    }))
    .await
}

fn parse_env_or<T: std::str::FromStr>(var: &str, default: T) -> T {
    std::env::var(var)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
