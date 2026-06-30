//! Axum handler for `/v1/backfill/status`.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};

use crate::backfill::dto::{AmmStream, BackfillStatus, SdexStream};
use crate::backfill::queries_ch::{self, ProgressRow};
use crate::common::{cache_control, errors};
use crate::state::AppState;

/// `GET /backfill/status` — push-state of both backfill streams.
#[utoipa::path(
    get,
    path = "/backfill/status",
    tag = "backfill",
    responses((status = 200, description = "Backfill progress", body = BackfillStatus))
)]
pub async fn get_status(State(state): State<AppState>) -> Response {
    let rows = match queries_ch::all_progress(state.ch()).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "backfill all_progress query failed");
            return errors::internal_error(errors::DB_ERROR, "backfill status lookup failed");
        }
    };

    let find = |name: &str| rows.iter().find(|r| r.task_name == name);

    let sdex = find("sdex_archive").map(|r| SdexStream {
        status: r.status.clone(),
        current_ledger: r.current_ledger,
        start_ledger: r.start_ledger,
        target_ledger: r.target_ledger,
        progress_pct: progress_pct(r),
        ledgers_remaining: r.current_ledger.saturating_sub(r.start_ledger),
        last_push_at: r.last_push_at.clone(),
    });

    let soroban_amm = find("soroban_amm").map(|r| AmmStream {
        status: r.status.clone(),
        last_push_at: r.last_push_at.clone(),
        completed_at: r.completed_at.clone(),
    });

    let realtime_tip_ledger = sdex.as_ref().map(|s| s.target_ledger).unwrap_or(0);

    let body = BackfillStatus {
        realtime_tip_ledger,
        sdex,
        soroban_amm,
    };
    let mut resp = Json(body).into_response();
    cache_control::attach(&mut resp, cache_control::MEDIUM);
    resp
}

/// `(target - current) / (target - start) * 100`, guarded against a zero span
/// (placeholder rows seed all ledgers to 0).
fn progress_pct(r: &ProgressRow) -> f64 {
    let span = r.target_ledger.saturating_sub(r.start_ledger);
    if span == 0 {
        return 0.0;
    }
    let done = r.target_ledger.saturating_sub(r.current_ledger);
    (done as f64 / span as f64) * 100.0
}
