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
/// JSON, not XDR). `contract_id` is BE's Int64 surrogate, mapped back to a
/// `C…` strkey via [`resolve_contract_ids`].
///
/// ⚠️ `transaction_id` is NO LONGER BE's surrogate. BE dropped that column from
/// `soroban_events` on 2026-09-17 (task 0304); the read now SYNTHESISES a
/// stable per-transaction key from `(ledger_sequence, transaction_index)`,
/// which is all any consumer needs — `RawSorobanEvent::transaction_id` is
/// documented as "only a grouping key — any stable per-transaction identifier
/// works".
///
/// Field order MUST match the `SELECT` column order below — the `Row` derive
/// binds positionally. `EVENT_ROW_COLUMNS` below pins the two together.
///
/// ⚠️ And field WIDTH must match the column's. The client sends bare
/// `RowBinary` (clickhouse-0.13 `query.rs:88`): no names-and-types header, so
/// a field narrower or wider than its column is not an error — the cursor
/// reads the wrong number of bytes and every field after it decodes garbage.
/// BE widened `event_index` from `Int16` to `UInt32` in the same 2026-09-17
/// change that moved `application_order` onto the row, which is exactly this.
/// Every numeric column is therefore projected through an EXPLICIT cast to the
/// field's type, pinned by `every_numeric_column_off_the_event_row_is_cast`: a
/// future widening then truncates one visible value instead of shifting the
/// whole row. Note that `chq` (raw SQL) never exercises RowBinary, so a
/// production spot check cannot catch a width drift — only the cast can.
#[derive(Debug, Row, Deserialize)]
pub struct EventRow {
    pub contract_id: i64,
    pub transaction_id: i64,
    pub ledger_sequence: u32,
    /// The event's operation within its transaction — a column BE added in the
    /// same 2026-09-17 change (task 0304). Read because it is part of the fill
    /// key ADR 0287 D1 defines, and because the same change widened
    /// `event_index`, which is what a per-OPERATION numbering would need: were
    /// `event_index` to restart at each operation, `(transaction_id,
    /// event_index)` alone would collide across two operations of one
    /// transaction and `run::read_chunk` would drop the second event as an RMT
    /// double. Keying on the operation too is correct under either numbering.
    pub operation_index: u16,
    /// ⚠️ `UInt32` on the server since 2026-09-17, NOT the `Int16` it was
    /// (task 0304). See the width rule above.
    pub event_index: u32,
    /// Ledger close time, unix seconds — the candle-minute bucketing key.
    pub closed_at: i64,
    pub topics_xdr: String,
    pub data_xdr: String,
    /// The transaction's position in its ledger's apply order (task 0286 D1),
    /// read straight off the event row since BE's 2026-09-17 change (task
    /// 0304). There is no join and therefore no "found" marker any more: the
    /// column is non-nullable, so every row carries a real value. It is `Int16`
    /// and a NEGATIVE value is still not a position — see
    /// `run::resolve_transaction_index`.
    pub application_order: i16,
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
    "operation_index",
    "event_index",
    "closed_at",
    "topics_xdr",
    "data_xdr",
    "application_order",
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
/// transaction_index, event_index)` so the run can group them by ledger then
/// transaction with per-tx event order preserved, in the real APPLY order
/// (task 0286 D1 — `event_index` alone restarts in every transaction). Rows are pulled one at a time (`cursor.next()`), so peak memory is
/// one ledger's events — not the whole chunk (which, filtered to AMM pools, still
/// includes Phoenix's 8 events/swap plus reserves/transfers and can be millions
/// of rows on a dense range).
///
/// ⚠️ **`application_order` used to come from a `LEFT JOIN default.transactions`
/// and no longer does** (task 0304). BE put the column on the event row itself
/// on 2026-09-17 and dropped `soroban_events.transaction_id` in the same change,
/// which killed the join's `ON` clause outright (`Code: 47`). Removing the join
/// also removes everything that existed to defend it: the `GROUP BY id` against
/// a build side with two rows per `id` MULTIPLYING the probe row and repricing a
/// transaction's events twice (task 0286 CR-01), the `ifNull` null-framing, and
/// the "apply order not found" fallback. Do not reintroduce it — the column is
/// right there.
///
/// Both source tables are ReplacingMergeTree; the surviving join collapses its
/// build side to one row per `sequence` via `GROUP BY`, and `soroban_events`
/// duplicates (identical `(contract_id, ledger, tx, event_index)` rows) are
/// removed adjacently in the run loop — together deduping the RMT doubling
/// without a full-table `FINAL`.
///
/// `transaction_id` is SYNTHESISED as `ledger_sequence * 100000 +
/// transaction_index`: BE's surrogate is gone, and every consumer of this field
/// uses it only to group a transaction's events together.
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
            toInt64(e.contract_id) AS contract_id, \
            toInt64(e.ledger_sequence) * 100000 + toInt64(e.transaction_index) \
                AS transaction_id, \
            toUInt32(e.ledger_sequence) AS ledger_sequence, \
            toUInt16(e.operation_index) AS operation_index, \
            toUInt32(e.event_index) AS event_index, \
            ifNull(l.closed_at, 0) AS closed_at, \
            e.topics_xdr AS topics_xdr, \
            e.data_xdr AS data_xdr, \
            toInt16(e.application_order) AS application_order \
         FROM default.soroban_events e \
         LEFT JOIN ( \
            SELECT sequence, toInt64(min(toUnixTimestamp(closed_at))) AS closed_at \
            FROM default.ledgers \
            WHERE sequence BETWEEN {start} AND {end} \
            GROUP BY sequence \
         ) l ON l.sequence = e.ledger_sequence \
         WHERE e.ledger_sequence BETWEEN {start} AND {end} \
           AND e.contract_id IN ({in_list}) \
         ORDER BY e.ledger_sequence, e.application_order, e.transaction_index, \
                  e.operation_index, e.event_index"
    )
}

/// Advisory registry-completeness probe (dry-run only). Counts events and distinct
/// contracts in `[start, end]` that emitted a swap/trade-shaped event but are NOT
/// in the resolved registry id set — i.e. AMM activity the reprice would miss
/// because the pool is absent from `prices.pool_registry` (its events are never
/// even fetched by [`read_chunk`], so it produces no candle AND no
/// `unresolved_pools` record).
///
/// Heuristic: matches the common sym-`swap` / sym-`trade` signatures, the
/// Soroswap-pair envelope (`String("SoroswapPair")`, whose `signature` is NULL)
/// and the Comet pool envelope `[Symbol("POOL"), Symbol("swap")]` (task 0300 —
/// keyed on topics, because its `signature` would be `POOL` for the pool's
/// liquidity events too). It may include non-AMM `swap`/`trade` emitters and
/// misses the rare NULL-sig Phoenix micro-event shape, so it is a *verify-this*
/// prompt, not a hard error. Returns `(distinct_contracts, events)`.
pub async fn count_unregistered_amm_emitters(
    client: &Client,
    contract_ids: &[i64],
    start: u32,
    end: u32,
    chunk_size: u32,
) -> Result<(u64, u64), EventsBackfillError> {
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
        let sql = unregistered_emitters_sql(chunk_start, chunk_end, contract_ids);
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

/// One chunk of [`count_unregistered_amm_emitters`]: the swap/trade-shaped
/// emitters in `[chunk_start, chunk_end]` outside `contract_ids`. A pure
/// function so the shapes it matches are pinned by a test. Interpolates only
/// integers; no `SETTINGS` (the read runs under `readonly=1`).
///
/// The Comet disjunct (task 0300) uses 1-based `topics_xdr` indexes, the same
/// form as `discover.rs`'s factory read.
pub(crate) fn unregistered_emitters_sql(
    chunk_start: u32,
    chunk_end: u32,
    contract_ids: &[i64],
) -> String {
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
    format!(
        "SELECT e.contract_id AS contract_id, count() AS events \
         FROM default.soroban_events e \
         WHERE e.ledger_sequence BETWEEN {chunk_start} AND {chunk_end} \
           AND (e.signature IN ('swap', 'trade') \
                OR e.topics_xdr LIKE '%SoroswapPair%' \
                OR (JSONExtractString(e.topics_xdr, 1, 'value') = 'POOL' \
                    AND JSONExtractString(e.topics_xdr, 2, 'value') = 'swap')) \
           {not_in}\
         GROUP BY e.contract_id"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sql() -> String {
        chunk_sql(&[11, 22], 1000, 2000)
    }

    /// Task 0300 D6: the unregistered-emitter probe matches every swap shape
    /// the ingest prices, Comet's `[POOL, swap]` included (keyed on topics, not
    /// on `signature`, which would also be `POOL` for its liquidity events).
    #[test]
    fn unregistered_emitters_sql_matches_every_indexed_swap_shape() {
        let sql = unregistered_emitters_sql(1, 2, &[7, 9]);
        let sql = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(sql.contains("BETWEEN 1 AND 2"));
        assert!(sql.contains("e.signature IN ('swap', 'trade')"));
        assert!(sql.contains("LIKE '%SoroswapPair%'"));
        assert!(sql.contains(
            "(JSONExtractString(e.topics_xdr, 1, 'value') = 'POOL' \
             AND JSONExtractString(e.topics_xdr, 2, 'value') = 'swap')"
        ));
        assert!(sql.contains("NOT IN (7,9)"));
        assert!(!sql.contains("SETTINGS"), "readonly=1 rejects SETTINGS");

        let unfiltered = unregistered_emitters_sql(1, 2, &[]);
        assert!(!unfiltered.contains("NOT IN"));
        assert!(!unfiltered.contains("SETTINGS"));
    }

    /// Task 0286 D1, as task 0304 left it. The apply order used to come from a
    /// `LEFT JOIN default.transactions`; BE put it on the event row itself on
    /// 2026-09-17 and dropped `soroban_events.transaction_id` in the same
    /// change, which killed the join's `ON` clause (`Code: 47`).
    ///
    /// Reintroducing that join would not merely be redundant — it would bring
    /// back the fan-out of CR-01, the null-framing and the found-marker, all of
    /// which exist only to defend it. So this test forbids it rather than
    /// requiring it.
    #[test]
    fn the_apply_order_comes_off_the_event_row_not_a_join() {
        let sql = sql();
        assert!(
            sql.contains("toInt16(e.application_order) AS application_order"),
            "the apply order is a column on soroban_events since 2026-09-17"
        );
        assert!(
            !sql.contains("default.transactions"),
            "the transactions join is gone with BE's transaction_id column;              do not bring it back"
        );
        assert!(
            !sql.contains("e.transaction_id"),
            "soroban_events has no transaction_id column any more (task 0304)"
        );
        assert!(
            !sql.contains("INNER JOIN"),
            "an unmatched ledger must degrade, never drop the event"
        );
    }

    /// Task 0286 T-jiv-05. The memory limit is per query, and this file may not
    /// use `SETTINGS` (readonly, code 164), so the ONLY bound is what the read
    /// is allowed to touch: the chunk's ledgers on both sides of the surviving
    /// `ledgers` join, and the chunk's AMM contracts on the probe side.
    #[test]
    fn the_read_is_bounded_on_both_sides() {
        let sql = sql();
        let join = sql
            .split("FROM default.ledgers")
            .nth(1)
            .expect("ledgers subquery");
        let join = join.split(") l ON").next().unwrap_or(join);
        assert!(
            join.contains("sequence BETWEEN 1000 AND 2000"),
            "the build side must be pruned by the ledgers table's sort key"
        );
        assert!(
            sql.contains("e.ledger_sequence BETWEEN 1000 AND 2000"),
            "the probe side stays bounded too"
        );
        assert!(
            sql.contains("e.contract_id IN (11,22)"),
            "and filtered on the numeric contract ids, which prune by the primary index"
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

    /// CR-01, which now has one join left to apply to: a LEFT JOIN whose build
    /// side has more than one row per key MULTIPLIES the probe row, and every
    /// AMM event of that ledger would be emitted twice, reach the extraction
    /// seam twice and be dispatched into two ticks — double `volume_base`,
    /// double `volume_quote`, double `trade_count`, silently. Same failure
    /// class as task 0282.
    ///
    /// `default.ledgers` is a ReplacingMergeTree, so the `GROUP BY sequence` is
    /// load-bearing rather than tidiness. (The `default.transactions` half of
    /// this rule died with the join in task 0304.)
    #[test]
    fn the_ledgers_join_yields_one_row_per_sequence() {
        let sql = sql();
        let join = sql
            .split("FROM default.ledgers")
            .nth(1)
            .expect("ledgers subquery");
        let join = join.split(") l ON").next().unwrap_or(join);
        assert!(
            join.contains("GROUP BY sequence") || join.contains("LIMIT 1 BY sequence"),
            "the ledgers build side must be collapsed to one row per sequence, \
             or the join fans out and double-counts AMM volume: {join}"
        );
    }

    /// WR-02. `EventRow` binds POSITIONALLY, and `join_use_nulls = 1` is a
    /// profile setting this file cannot override (it may not emit `SETTINGS` —
    /// readonly, code 164). Under it an unmatched LEFT JOIN column becomes
    /// `Nullable(...)`, RowBinary prefixes a null byte, and every field from
    /// that column onward decodes as garbage.
    ///
    /// ⚠️ This applies to columns that come from a JOIN. After task 0304 only
    /// `closed_at` does — `application_order` is projected straight off `e`, so
    /// null-framing it would be noise, and wrapping a non-nullable column in
    /// `ifNull` would hide a future schema change rather than surface it.
    #[test]
    fn the_joined_column_is_null_framed() {
        let sql = sql();
        assert!(
            sql.contains("ifNull(l.closed_at, 0) AS closed_at"),
            "the one remaining joined column must be null-framed"
        );
        assert!(
            !sql.contains("ifNull(e."),
            "columns read straight off the event row are not joined and must \
             not be null-framed — that would mask a dropped column"
        );
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
            "e.application_order",
            "e.transaction_index",
            "e.operation_index",
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
            "ORDER BY must be ledger, apply order, transaction, operation, \
             event index: {order_by}"
        );
    }

    /// Task 0304 CR-02. `EventRow` binds positionally AND by Rust type: the
    /// client sends bare `RowBinary`, which carries no names-and-types header,
    /// so a field whose width differs from its column's shifts every field
    /// after it and decodes garbage — no error, no 500, just wrong candles.
    /// That is not hypothetical: BE widened `event_index` from `Int16` to
    /// `UInt32` on 2026-09-17 and the read kept an `i16` field.
    ///
    /// So every numeric column off `e` is projected through an explicit cast to
    /// its field's type. A cast cannot prevent a future widening, but it turns
    /// one into a truncated value in one column instead of a shifted row — and
    /// unlike the alias-order contract above, nothing else in the crate can
    /// catch this: a `chq` spot check runs raw SQL and never frames a row.
    #[test]
    fn every_numeric_column_off_the_event_row_is_cast() {
        let sql = sql();
        let select = sql
            .split(" FROM default.soroban_events")
            .next()
            .expect("the SELECT");

        for (expr, field) in [
            ("toInt64(e.contract_id) AS contract_id", "i64"),
            ("toUInt32(e.ledger_sequence) AS ledger_sequence", "u32"),
            ("toUInt16(e.operation_index) AS operation_index", "u16"),
            ("toUInt32(e.event_index) AS event_index", "u32"),
            ("toInt16(e.application_order) AS application_order", "i16"),
        ] {
            assert!(
                select.contains(expr),
                "the projection must pin the wire width to EventRow's {field}: \
                 expected `{expr}` in {select}"
            );
        }

        // And no numeric column slips back in bare. A leading space is what
        // distinguishes a projection (` e.x AS`) from a cast's argument
        // (`(e.x) AS`), which is the shape the loop above requires.
        for bare in [
            " e.contract_id AS",
            " e.ledger_sequence AS",
            " e.operation_index AS",
            " e.event_index AS",
            " e.application_order AS",
        ] {
            assert!(
                !select.contains(bare),
                "`{bare}` is an uncast projection — give it an explicit cast: {select}"
            );
        }
    }
}
