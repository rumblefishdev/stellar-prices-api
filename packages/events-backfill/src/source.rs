//! Read side of the CH-to-CH reprice: pulls AMM contract events out of BE's
//! `default.soroban_events` (+ `ledgers` for close time, `soroban_contracts` for
//! the strkey) so [`crate::run`] can feed them through the shared extraction
//! seam. No ledger archive is touched — this is the whole point of task 0097.

use std::collections::{HashMap, HashSet};

use clickhouse::query::RowCursor;
use clickhouse::{Client, Row};
use serde::Deserialize;

use crate::error::EventsBackfillError;

// NOTE — no `SETTINGS` clause anywhere in this file, deliberately. The client
// sends read queries as POST with an explicit `readonly=1` whenever the SQL
// exceeds its GET-length threshold (clickhouse-0.13 `query.rs:158-166`), and our
// reads always do: they embed the full registry contract-id list. Under
// `readonly=1` ClickHouse rejects any per-query setting change with code 164
// (READONLY) — which is exactly how a `SETTINGS max_threads` on the coverage
// probe failed. Bound reads by CHUNKING (the memory limit is per-query), never
// by per-query settings.

/// One `soroban_events` row joined to its ledger close time. `topics_xdr` /
/// `data_xdr` are the typed-JSON SCVal strings BE persists (misnamed — they are
/// JSON, not XDR). `contract_id` / `transaction_id` are BE's Int64 surrogates;
/// `contract_id` is mapped back to a `C…` strkey via [`resolve_contract_ids`].
///
/// Field order MUST match the `SELECT` column order below — the `Row` derive
/// binds positionally. `EVENT_ROW_COLUMNS` below pins the two together.
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
    /// The transaction's position in its ledger's apply order, from BE's
    /// `default.transactions` (task 0286 D1). 0 when the join found nothing —
    /// read it together with `apply_order_found`, never on its own.
    pub application_order: i16,
    /// 1 when the apply order above came from a real joined row, 0 when the
    /// transaction could not be resolved. A LEFT join yields the column's type
    /// DEFAULT, not NULL, for an unmatched row, so a bare 0 in
    /// `application_order` is ambiguous without this marker.
    pub apply_order_found: u8,
}

/// [`EventRow`]'s fields, in order, as the chunk read must alias them. The
/// `Row` derive binds by POSITION: a SELECT that projects the same columns in a
/// different order decodes silently wrong values rather than erroring, so the
/// two are pinned to this list by a unit test (task 0286). Test-only: it is a
/// contract with the `SELECT` in `chunk_sql`, asserted, never read at runtime.
#[cfg(test)]
pub(crate) const EVENT_ROW_COLUMNS: [&str; 9] = [
    "contract_id",
    "transaction_id",
    "ledger_sequence",
    "event_index",
    "closed_at",
    "topics_xdr",
    "data_xdr",
    "application_order",
    "apply_order_found",
];

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
/// in `[start, end]`, ordered by `(ledger_sequence, application_order,
/// transaction_id, event_index)` so the run can group them by ledger then
/// transaction with per-tx event order preserved, in the real APPLY order
/// (task 0286 D1 — `event_index` alone restarts in every transaction). Rows are pulled one at a time (`cursor.next()`), so peak memory is
/// one ledger's events — not the whole chunk (which, filtered to AMM pools, still
/// includes Phoenix's 8 events/swap plus reserves/transfers and can be millions
/// of rows on a dense range).
///
/// All three source tables are ReplacingMergeTree; both joins collapse their
/// build side to one row per key via `GROUP BY` (`sequence` for `ledgers`, `id`
/// for `transactions`), and `soroban_events` duplicates (identical
/// `(contract_id, ledger, tx, event_index)` rows) are removed adjacently in the
/// run loop — together deduping the RMT doubling without a full-table `FINAL`.
///
/// The `GROUP BY id` is load-bearing, not tidiness: `default.transactions` is
/// keyed `(ledger_sequence, application_order, id)`, so `FINAL` collapses only
/// rows that already agree on `application_order`. A LEFT JOIN on a build side
/// with two rows for one `id` MULTIPLIES the probe row, and every AMM event of
/// that transaction would be repriced twice — double volume, double trade
/// count, silently (task 0286 CR-01).
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
    Ok(client
        .query(&chunk_sql(contract_ids, start, end))
        .fetch::<EventRow>()?)
}

/// The chunk read, as text. Extracted from [`stream_chunk`] so its shape is
/// assertable without a ClickHouse: this query carries three properties that
/// are invisible until production breaks on them — the memory bound, the fill
/// order, and the positional alignment with [`EventRow`] (task 0286).
pub(crate) fn chunk_sql(contract_ids: &[i64], start: u32, end: u32) -> String {
    let in_list = contract_ids
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");

    format!(
        "SELECT \
            e.contract_id AS contract_id, \
            e.transaction_id AS transaction_id, \
            toUInt32(e.ledger_sequence) AS ledger_sequence, \
            e.event_index AS event_index, \
            ifNull(l.closed_at, 0) AS closed_at, \
            e.topics_xdr AS topics_xdr, \
            e.data_xdr AS data_xdr, \
            ifNull(t.application_order, 0) AS application_order, \
            ifNull(t.found, 0) AS apply_order_found \
         FROM default.soroban_events e \
         LEFT JOIN ( \
            SELECT sequence, toInt64(min(toUnixTimestamp(closed_at))) AS closed_at \
            FROM default.ledgers \
            WHERE sequence BETWEEN {start} AND {end} \
            GROUP BY sequence \
         ) l ON l.sequence = e.ledger_sequence \
         LEFT JOIN ( \
            SELECT id, \
                   argMin(application_order, ledger_sequence) AS application_order, \
                   toUInt8(1) AS found \
            FROM default.transactions FINAL \
            WHERE ledger_sequence BETWEEN {start} AND {end} \
              AND id IN ( \
                 SELECT DISTINCT transaction_id \
                 FROM default.soroban_events \
                 WHERE ledger_sequence BETWEEN {start} AND {end} \
                   AND contract_id IN ({in_list}) \
              ) \
            GROUP BY id \
         ) t ON t.id = e.transaction_id \
         WHERE e.ledger_sequence BETWEEN {start} AND {end} \
           AND e.contract_id IN ({in_list}) \
         ORDER BY e.ledger_sequence, application_order, e.transaction_id, e.event_index"
    )
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
    chunk_size: u32,
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
    #[derive(Row, Deserialize)]
    struct EmitterRow {
        contract_id: i64,
        events: u64,
    }

    // Walk the range in the SAME chunks as the reprice. As one query over the
    // full range this read `topics_xdr` (a large JSON column) for EVERY event in
    // range — the `NOT IN` and the `LIKE` leave nothing for the index to prune,
    // unlike the main loop, which filters to registry contracts. On a 12.9M-ledger
    // range that exceeded ch-prod-01's 5.59 GiB per-query memory quota
    // (MEMORY_LIMIT_EXCEEDED while reading `topics_xdr`). The memory limit is
    // per-query, so chunking is what bounds it — capping `max_threads` is not an
    // option here (see the `readonly=1` note at the top of this file).
    //
    // `uniqExact` over the whole range is replaced by a per-chunk `GROUP BY`
    // whose distinct contracts are unioned client-side — the set of AMM-shaped
    // emitters is small, so this stays exact without server-side state.
    let mut seen: HashSet<i64> = HashSet::new();
    let mut events_total: u64 = 0;
    let mut chunk_start = start;
    // `execute` rejects 0 before we get here; clamp anyway so this pub fn can
    // never underflow into a panic on a direct call.
    let step = chunk_size.max(1);
    loop {
        let chunk_end = chunk_start.saturating_add(step - 1).min(end);
        let sql = format!(
            "SELECT e.contract_id AS contract_id, count() AS events \
             FROM default.soroban_events e \
             WHERE e.ledger_sequence BETWEEN {chunk_start} AND {chunk_end} \
               AND (e.signature IN ('swap', 'trade') OR e.topics_xdr LIKE '%SoroswapPair%') \
               {not_in}\
             GROUP BY e.contract_id"
        );
        for row in client.query(&sql).fetch_all::<EmitterRow>().await? {
            seen.insert(row.contract_id);
            events_total += row.events;
        }
        if chunk_end >= end {
            break;
        }
        chunk_start = chunk_end + 1;
    }

    Ok((seen.len() as u64, events_total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sql() -> String {
        chunk_sql(&[11, 22], 1000, 2000)
    }

    /// Task 0286 D1. BE's `default.transactions` holds the apply order this
    /// read has to carry; `default.soroban_events` does not. Without the join
    /// every AMM fill of a ledger looks like it came from transaction 0 and the
    /// candle closes on whichever event happened to have the highest index.
    #[test]
    fn chunk_sql_joins_the_transactions_apply_order() {
        let sql = sql();
        assert!(
            sql.contains("LEFT JOIN") && sql.contains("default.transactions FINAL"),
            "the apply order comes from default.transactions, FINAL (it is a ReplacingMergeTree)"
        );
        assert!(
            !sql.contains("INNER JOIN"),
            "an unmatched transaction must degrade, never drop the event"
        );
    }

    /// Task 0286 T-jiv-05. The memory limit is per query, and this file may not
    /// use `SETTINGS` (readonly, code 164), so the ONLY bound is what the join
    /// is allowed to read: the chunk's ledgers on both sides, and only the
    /// transactions that actually emitted one of the chunk's AMM events.
    #[test]
    fn the_apply_order_join_is_bounded_on_both_sides() {
        let sql = sql();
        let join = sql
            .split("default.transactions FINAL")
            .nth(1)
            .expect("transactions subquery");
        let join = join.split(") t ON").next().unwrap_or(join);
        assert!(
            join.contains("ledger_sequence BETWEEN 1000 AND 2000"),
            "the build side must be pruned by the transactions table's leading sort key"
        );
        assert!(
            join.contains("default.soroban_events") && join.contains("contract_id IN (11,22)"),
            "and restricted to transactions that emitted one of this chunk's AMM events"
        );
        assert!(
            sql.contains("e.ledger_sequence BETWEEN 1000 AND 2000"),
            "the probe side stays bounded too"
        );
    }

    /// No `SETTINGS` anywhere — see the note at the top of this file. A
    /// per-query setting on a `readonly=1` POST is rejected with code 164, and
    /// the read is long enough that the client always POSTs.
    #[test]
    fn chunk_sql_carries_no_settings_clause() {
        assert!(!sql().contains("SETTINGS"));
    }

    /// The `Row` derive binds POSITIONALLY: `EventRow`'s field order and this
    /// SELECT's alias order are one contract, and a mismatch decodes garbage
    /// rather than erroring. [`EVENT_ROW_COLUMNS`] is what pins them together.
    #[test]
    fn chunk_sql_projects_event_row_columns_in_field_order() {
        let sql = sql();
        let select = sql.split(" FROM default.soroban_events").next().unwrap();
        let aliases: Vec<&str> = select
            .split(" AS ")
            .skip(1)
            .map(|part| part.split([',', ' ']).find(|t| !t.is_empty()).unwrap())
            .collect();
        assert_eq!(aliases, EVENT_ROW_COLUMNS.to_vec());
    }

    /// CR-01, and the reason the sibling `ledgers` join is a `GROUP BY`
    /// aggregate rather than a bare SELECT: a LEFT JOIN whose build side has
    /// more than one row per key MULTIPLIES the probe row.
    ///
    /// `default.transactions` is a ReplacingMergeTree whose sort key is
    /// `(ledger_sequence, application_order, id)`, so `FINAL` collapses only
    /// rows that already AGREE on `application_order` — two rows for one `id`
    /// with different apply orders both survive. Every AMM event of that
    /// transaction would then be emitted twice, reach the extraction seam
    /// twice, and be dispatched into two ticks: double `volume_base`, double
    /// `volume_quote`, double `trade_count`, silently. That is the same
    /// failure class task 0282 is fighting.
    #[test]
    fn the_apply_order_join_yields_one_row_per_transaction() {
        let sql = sql();
        let join = sql
            .split("default.transactions FINAL")
            .nth(1)
            .expect("transactions subquery");
        let join = join.split(") t ON").next().unwrap_or(join);
        assert!(
            join.contains("GROUP BY id") || join.contains("LIMIT 1 BY id"),
            "the transactions build side must be collapsed to one row per id, \
             or the join fans out and double-counts AMM volume: {join}"
        );
    }

    /// WR-02. `EventRow` binds POSITIONALLY, and `join_use_nulls = 1` is a
    /// profile setting this file cannot override (it may not emit `SETTINGS` —
    /// readonly, code 164). Under it an unmatched LEFT JOIN column becomes
    /// `Nullable(...)`, RowBinary prefixes a null byte, and every field from
    /// `application_order` onward decodes as garbage. The pre-existing
    /// `ifNull(l.closed_at, 0)` is the record of someone hitting this already.
    #[test]
    fn the_joined_columns_are_null_framed_like_their_sibling() {
        let sql = sql();
        for projection in [
            "ifNull(l.closed_at, 0) AS closed_at",
            "ifNull(t.application_order, 0) AS application_order",
            "ifNull(t.found, 0) AS apply_order_found",
        ] {
            assert!(
                sql.contains(projection),
                "every joined column must be null-framed: missing `{projection}`"
            );
        }
    }

    /// The candle's fill order, in the read that feeds it. Grouping in
    /// `run.rs` and `soroban.rs` also requires a transaction's events to stay
    /// contiguous, which sorting by apply order before the transaction id keeps.
    #[test]
    fn chunk_sql_orders_by_ledger_then_apply_order_then_transaction() {
        let sql = sql();
        let order_by = sql.rsplit("ORDER BY").next().expect("ORDER BY");
        let positions: Vec<usize> = [
            "e.ledger_sequence",
            "application_order",
            "e.transaction_id",
            "e.event_index",
        ]
        .iter()
        .map(|needle| {
            order_by
                .find(needle)
                .unwrap_or_else(|| panic!("ORDER BY is missing {needle}: {order_by}"))
        })
        .collect();
        assert!(
            positions.windows(2).all(|w| w[0] < w[1]),
            "ORDER BY must be ledger, apply order, transaction, event index: {order_by}"
        );
    }
}
