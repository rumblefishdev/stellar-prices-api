---
title: 'Recommendation — Option A: add quote_asset_id to price_ohlcv PK'
type: synthesis
status: mature
spawned_from: ./Q-decision-space.md
spawns: ['ADR-draft']
tags: [schema, ohlcv, primary-key, recommendation, sdex]
links:
  - './Q-decision-space.md'
  - '../README.md'
  - '../../../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md'
  - '../../../../docs/database-schema/database-schema-overview.md'
history:
  - date: 2026-05-13
    status: mature
    who: okarcz
    note: >
      Recommendation: Option A (add `quote_asset_id` to PK). Reasoning
      below; ADR draft in this directory translates this into the
      formal record.
---

# Recommendation — Option A: add `quote_asset_id` to `price_ohlcv` PK

## TL;DR

**Adopt Option A.** Change `price_ohlcv` PK from
`(timestamp, asset_id, granularity)` to
`(timestamp, asset_id, quote_asset_id, granularity)`. Backfill
writes one row per native pair per minute (e.g. one row for
USDC/XLM, another for USDC/USDT in the same minute). API reads
aggregate across rows to produce the per-asset USD or XLM
projection.

Migration is trivial because the schema is greenfield — no rows
exist in `price_ohlcv` yet. The change lands in task 0012's
schema-migration step before the backfill writes its first row.

## Option matrix

| Option | Storage shape                                | API projection at read time       | Migration cost                                           | Verdict         |
| ------ | -------------------------------------------- | --------------------------------- | -------------------------------------------------------- | --------------- |
| **A**  | One row per native pair                      | Aggregate across rows             | Add column + recreate PK                                 | **Adopt**       |
| B      | One row per pair via `asset_pairs` surrogate | Same + join to resolve identities | New table + ALTER `price_ohlcv` + populate `asset_pairs` | Defer           |
| C      | One row per (asset, USD-or-XLM kind)         | Direct read                       | Couples backfill writes to oracle availability           | Reject          |
| D      | Status quo (base-only PK)                    | Direct read                       | None                                                     | Reject (broken) |

## Why A over B

Both A and B preserve per-native-pair granularity. The trade-off is
where the pair identity lives.

| Aspect                           | A                                                                    | B                                                                                         |
| -------------------------------- | -------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| PK size                          | 4 columns (timestamp, asset_id, quote_asset_id, granularity)         | 3 columns (timestamp, asset_pair_id, granularity)                                         |
| Bytes per PK tuple               | ~32 B (TIMESTAMPTZ + 2 INT + VARCHAR(5))                             | ~24 B (TIMESTAMPTZ + INT + VARCHAR(5))                                                    |
| Reads "all candles for asset X"  | `WHERE asset_id = X` (direct)                                        | Need pair lookup: `WHERE asset_pair_id IN (SELECT id FROM asset_pairs WHERE base_id = X)` |
| Writes from backfill             | Look up `asset_id`, `quote_asset_id` already done in §3 of 0022 spec | Same plus a pair-id lookup (or insert) before the OHLCV UPSERT                            |
| Ad-hoc analytics                 | "Which pairs traded USDC on date X?" — direct SELECT                 | Same plus join to `asset_pairs`                                                           |
| Schema-doc match                 | Smallest delta from current schema                                   | Larger delta (new table, new FK)                                                          |
| Migration cost (now, greenfield) | Add one column, recreate PK                                          | Create `asset_pairs`, populate, alter `price_ohlcv`                                       |
| Re-use of `asset_pairs`          | N/A                                                                  | Only consumer is `price_ohlcv` today; speculative re-use                                  |

**Decision:** A is materially simpler today and B's surrogate
doesn't earn its complexity from any second consumer. If a future
table needs to key on pair (e.g. orderbook state, pair-level
analytics), spawn a follow-up task to introduce `asset_pairs`
and migrate `price_ohlcv` to it then. That's a cleaner just-in-time
move than introducing the table speculatively now.

## Why not C (pre-normalise to USD/XLM at write time)

Option C tempts because the API only exposes USD and XLM views
anyway — why store anything else? Two reasons it doesn't work:

1. **Backfill cannot pre-normalise without oracle data.** The
   USD denomination for SDEX trades on USDC/XLM requires the
   XLM/USD oracle price at the trade minute. The oracle path is
   handled by task 0024 (the `volume_quote_usd` enrichment pass)
   which runs _after_ the backfill. Forcing pre-normalisation
   would couple backfill correctness to oracle availability —
   regressing 0024's whole rationale.

2. **Per-source attribution requires per-native-pair rows.** The
   `sources` JSONB on `current_prices` exposes SDEX vs Soroswap
   vs Aquarius separately. The aggregator (Current Price Updater
   Lambda) reads `price_ohlcv` rows with `source != 'aggregated'`
   to build that breakdown. If we pre-normalise to quote-kind, we
   lose the source-level breakdown — the SDEX USDC/USDT trade
   and the Soroswap USDC/USDT trade would collapse into one row
   labelled `source = 'aggregated'` and the per-source view
   becomes impossible.

3. **Pre-normalisation re-runs are expensive.** If an oracle
   price is later corrected (a fix in `oracle_prices`), all
   downstream `price_ohlcv` rows derived from it would need
   re-derivation. With Option A the source data stays raw and
   the projection is recomputed at read time — corrections are
   free.

## Why not D (status quo, base-only PK)

PK collisions on assets with multiple quote pairs. Already
established in 0022 spec §6 item 1; this task is the response.

## Schema change (DDL)

```sql
-- 1. Drop the current PK (no data in the table; safe).
ALTER TABLE price_ohlcv DROP CONSTRAINT price_ohlcv_pkey;

-- 2. Add the column. Nullable initially (no rows), then add NOT NULL
--    once the column exists and the backfill is wired up.
ALTER TABLE price_ohlcv ADD COLUMN quote_asset_id INT;

-- 3. Add the new PK.
ALTER TABLE price_ohlcv
  ADD CONSTRAINT price_ohlcv_pkey
  PRIMARY KEY (timestamp, asset_id, quote_asset_id, granularity);

-- 4. Drop the existing composite index (now redundant under the new PK).
DROP INDEX IF EXISTS idx_ohlcv_asset_gran;

-- 5. Add the new composite index for the canonical read path:
--    "OHLCV for one asset across all quotes, time range, granularity".
CREATE INDEX idx_ohlcv_asset_gran
  ON price_ohlcv (asset_id, granularity, timestamp DESC, quote_asset_id);

-- 6. Optionally, the per-pair read index:
CREATE INDEX idx_ohlcv_pair_gran
  ON price_ohlcv (asset_id, quote_asset_id, granularity, timestamp DESC);

-- 7. NOT NULL constraint, added after the backfill code paths are wired.
ALTER TABLE price_ohlcv ALTER COLUMN quote_asset_id SET NOT NULL;
```

The native partitioning (range on `timestamp`) is unaffected — PK
just gains a column.

## API projection at read time

`GET /assets/{id}/ohlcv?base_currency=USD` reads:

```sql
-- For each (timestamp, granularity), aggregate across all rows for
-- the asset into a single USD-denominated candle.
SELECT
    timestamp,
    -- VWAP across rows; open/close pick the chronologically-first /
    -- chronologically-last contributing tick:
    -- (these aggregations live in the API handler — Postgres SQL
    --  expression sketched below assumes a per-row vwap and weights
    --  by USD volume.)
    sum(volume_quote_usd) / NULLIF(sum(volume_base_in_usd), 0) AS vwap,
    sum(volume_quote_usd) AS volume_quote_usd,
    sum(volume_base) AS volume_base,
    sum(trade_count) AS trade_count
FROM price_ohlcv
WHERE asset_id = $1
  AND granularity = $2
  AND timestamp BETWEEN $3 AND $4
GROUP BY timestamp
ORDER BY timestamp;
```

(Real impl picks OHLC by chronologically-first/last contributing
ticks, which needs `argMin`/`argMax` — Postgres can do this via
`DISTINCT ON` or window functions; details in task 0012's API
spec, not here.)

`base_currency=XLM` does the same against a different projection
(divide by oracle XLM/USD or use the native-XLM-pair row directly
when it exists).

## Impact on task 0022's decode-and-bucket spec

Decode spec [§5.1](../../../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md#51-in-memory-aggregation)
and §5.3 need a small text update: the in-memory accumulator key
becomes `(asset_id_base, quote_asset_id, minute_start)` (not just
`(asset_id_base, minute_start)`), and the UPSERT SQL includes
`quote_asset_id` in the conflict target. That's a one-line spec
correction; the algorithm doesn't change.

This task's ADR lands first; the spec correction follows in
task 0012 when implementation begins (or as an in-place edit to
the archived spec with a "superseded clauses" note — to be
decided when 0012 starts).

## Impact on task 0024 (`volume_quote_usd` enrichment)

Positive. With per-native-pair rows, the enrichment pass joins
`oracle_prices` on the _quote_ side cleanly:

```sql
UPDATE price_ohlcv p
   SET volume_quote_usd = p.volume_quote * o.price_usd
  FROM oracle_prices o
 WHERE p.quote_asset_id = o.asset_id
   AND date_trunc('minute', o.timestamp) = p.timestamp
   AND p.volume_quote_usd = 0;
```

(Sketch; real impl handles minute alignment and missing-oracle
cases per 0024.)

Without `quote_asset_id` on the row, the join would have to
reverse-derive the quote from a pair table or be unable to
disambiguate; A is strictly easier for 0024.

## Impact on task 0025 (live multi-source merge)

Positive. With per-pair rows, multiple sources writing to the
same `(timestamp, asset_id, quote_asset_id, granularity)` row
collide on PK and trigger the multi-source merge correctly.
Pre-normalisation to quote-kind would have made same-quote-kind
SDEX-vs-Soroswap collisions ambiguous about _which_ native pair
each constituent represents.

## Open follow-ups (small)

- **Naming.** `base_currency` query param on `/ohlcv` is
  misleading (it's the quote currency). Worth renaming to
  `quote_currency` in a v2 of the API. Not blocking. Spawn?
- **`current_prices.price_xlm` semantics.** Currently a single
  column. With Option A, the implementation is "weighted average
  across all XLM-quoted pairs for this asset". Spec for
  Current Price Updater (task 0012's sibling) should make this
  explicit. Not blocking 0023's ADR.
