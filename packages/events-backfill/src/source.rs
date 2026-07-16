//! Read side of the CH-to-CH reprice: pulls AMM contract events out of BE's
//! `default.soroban_events` (+ `ledgers` for close time, `soroban_contracts` for
//! the strkey) so [`crate::run`] can feed them through the shared extraction
//! seam. No ledger archive is touched — this is the whole point of task 0097.

use std::collections::HashMap;

use clickhouse::query::RowCursor;
use clickhouse::{Client, Row};
use serde::Deserialize;

use crate::error::EventsBackfillError;

/// One `soroban_events` row joined to its ledger close time. `topics_xdr` /
/// `data_xdr` are the typed-JSON SCVal strings BE persists (misnamed — they are
/// JSON, not XDR). `contract_id` / `transaction_id` are BE's Int64 surrogates;
/// `contract_id` is mapped back to a `C…` strkey via [`resolve_contract_ids`].
///
/// Field order MUST match the `SELECT` column order below — the `Row` derive
/// binds positionally.
#[derive(Debug, Row, Deserialize)]
pub struct EventRow {
    pub contract_id: i64,
    pub transaction_id: i64,
    pub ledger_sequence: u32,
    pub event_index: i16,
    /// Ledger close time, unix seconds — the candle-minute bucketing key.
    pub closed_at: i64,
    pub topics_xdr: String,
    pub data_xdr: String,
}

/// Resolve AMM pool `C…` strkeys to BE's Int64 `soroban_contracts.id` surrogates.
///
/// The events read filters on the numeric `contract_id`, not the strkey: since
/// `soroban_events` is `ORDER BY (contract_id, ledger_sequence, …)`, a numeric
/// `contract_id IN (…)` prunes by the primary index — the difference between
/// scanning the AMM pools' events and scanning the whole (hundreds-of-millions
/// row) table. Returns `id → strkey` so the run can tag each event back.
pub async fn resolve_contract_ids(
    client: &Client,
    strkeys: &[String],
) -> Result<HashMap<i64, String>, EventsBackfillError> {
    if strkeys.is_empty() {
        return Ok(HashMap::new());
    }
    // strkeys come from our own prices.pool_registry (trusted), but they are
    // interpolated into the SQL string, so hard-gate them to strkey shape
    // (uppercase alphanumeric) — no injection surface even if the registry is
    // ever fed from a less-trusted source.
    let in_list = strkeys
        .iter()
        .filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|s| format!("'{s}'"))
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!(
        "SELECT id, contract_id FROM default.soroban_contracts FINAL \
         WHERE contract_id IN ({in_list})"
    );

    #[derive(Row, Deserialize)]
    struct IdRow {
        id: i64,
        contract_id: String,
    }

    let rows = client.query(&sql).fetch_all::<IdRow>().await?;
    Ok(rows.into_iter().map(|r| (r.id, r.contract_id)).collect())
}

/// Open a **streaming** cursor over all events emitted by the given AMM contracts
/// in `[start, end]`, ordered by `(ledger_sequence, transaction_id, event_index)`
/// so the run can group them by ledger then transaction with per-tx event order
/// preserved. Rows are pulled one at a time (`cursor.next()`), so peak memory is
/// one ledger's events — not the whole chunk (which, filtered to AMM pools, still
/// includes Phoenix's 8 events/swap plus reserves/transfers and can be millions
/// of rows on a dense range).
///
/// Both source tables are ReplacingMergeTree; the join collapses `ledgers`
/// duplicates via `GROUP BY sequence`, and `soroban_events` duplicates (identical
/// `(contract_id, ledger, tx, event_index)` rows) are removed adjacently in the
/// run loop — together deduping the RMT doubling without a full-table `FINAL`.
///
/// The join is a **LEFT** join with `ifNull(closed_at, 0)`: an event whose ledger
/// is absent from `default.ledgers` still comes back (with `closed_at = 0`) so the
/// run loop can count and skip it, instead of an INNER JOIN silently dropping the
/// whole ledger's AMM swaps.
///
/// Caller guarantees `contract_ids` is non-empty (an empty `IN ()` is invalid SQL).
pub fn stream_chunk(
    client: &Client,
    contract_ids: &[i64],
    start: u32,
    end: u32,
) -> Result<RowCursor<EventRow>, EventsBackfillError> {
    let in_list = contract_ids
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!(
        "SELECT \
            e.contract_id AS contract_id, \
            e.transaction_id AS transaction_id, \
            toUInt32(e.ledger_sequence) AS ledger_sequence, \
            e.event_index AS event_index, \
            ifNull(l.closed_at, 0) AS closed_at, \
            e.topics_xdr AS topics_xdr, \
            e.data_xdr AS data_xdr \
         FROM default.soroban_events e \
         LEFT JOIN ( \
            SELECT sequence, toInt64(min(toUnixTimestamp(closed_at))) AS closed_at \
            FROM default.ledgers \
            WHERE sequence BETWEEN {start} AND {end} \
            GROUP BY sequence \
         ) l ON l.sequence = e.ledger_sequence \
         WHERE e.ledger_sequence BETWEEN {start} AND {end} \
           AND e.contract_id IN ({in_list}) \
         ORDER BY e.ledger_sequence, e.transaction_id, e.event_index"
    );

    Ok(client.query(&sql).fetch::<EventRow>()?)
}

/// Advisory registry-completeness probe (dry-run only). Counts events and distinct
/// contracts in `[start, end]` that emitted a swap/trade-shaped event but are NOT
/// in the resolved registry id set — i.e. AMM activity the reprice would miss
/// because the pool is absent from `prices.pool_registry` (its events are never
/// even fetched by [`read_chunk`], so it produces no candle AND no
/// `unresolved_pools` record).
///
/// Heuristic: matches the common sym-`swap` / sym-`trade` signatures and the
/// Soroswap-pair envelope (`String("SoroswapPair")`, whose `signature` is NULL).
/// It may include non-AMM `swap`/`trade` emitters and misses the rare NULL-sig
/// Phoenix micro-event shape, so it is a *verify-this* prompt, not a hard error.
/// Returns `(distinct_contracts, events)`.
pub async fn count_unregistered_amm_emitters(
    client: &Client,
    contract_ids: &[i64],
    start: u32,
    end: u32,
) -> Result<(u64, u64), EventsBackfillError> {
    let not_in = if contract_ids.is_empty() {
        String::new()
    } else {
        format!(
            "AND e.contract_id NOT IN ({}) ",
            contract_ids
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let sql = format!(
        "SELECT uniqExact(e.contract_id) AS contracts, count() AS events \
         FROM default.soroban_events e \
         WHERE e.ledger_sequence BETWEEN {start} AND {end} \
           AND (e.signature IN ('swap', 'trade') OR e.topics_xdr LIKE '%SoroswapPair%') \
           {not_in}"
    );

    #[derive(Row, Deserialize)]
    struct Counts {
        contracts: u64,
        events: u64,
    }

    let c = client.query(&sql).fetch_one::<Counts>().await?;
    Ok((c.contracts, c.events))
}
