---
title: "R: Map prices-api write targets from PG/RDS to ClickHouse"
type: research
status: developing
spawned_from: ../README.md
spawns: []
tags: [clickhouse, ohlcv, schema, merge, rollups, current-prices, step-2]
links:
  - "../../../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md"
  - "../../../../../docs/prices-api-general-overview.md"
  - "../../../blocked/0038_FEATURE_prices-ledger-processor-lambda.md"
  - "./R-be-hetzner-ch-shape.md"
history:
  - date: 2026-05-18
    status: developing
    who: okarcz
    note: "Distilled from ADR 0003, ADR 0004, design doc §3 + §5.2, task 0038."
---

# R: Map prices-api write targets from PG/RDS to ClickHouse

## Purpose

Step 2 of task 0044. For each downstream write target in the current
RDS-based plan, propose a ClickHouse-native shape. The interesting
case is `price_ohlcv` because its merge semantics are non-commutative
(`open`/`close` are chronologically-positional, not sums); the others
are mostly mechanical.

This note is **not the recommendation**. It enumerates options and
flags trade-offs; the go/no-go and the final shape live in the later
`S-*` note.

---

## 0. Live-ingest contract today (from 0038)

The blocked task 0038 spec defines the per-invocation contract:

1. S3 PutObject event → fetch object → `zstd` decompress → parse
   `LedgerCloseMeta` via `xdr-parser`.
2. Run dispatch kernel from 0037 → emit `Vec<TradeTick>` per
   extractor (SDEX, Soroswap, Aquarius, Phoenix).
3. Bucket trades by `(floor_minute(closed_at), asset_id,
   quote_asset_id, '1m', source)`.
4. Emit one `INSERT … ON CONFLICT (timestamp, asset_id,
   quote_asset_id, granularity) DO UPDATE` per bucket — the merge
   formula from ADR 0004.

The "single UPSERT with ON CONFLICT DO UPDATE" pattern is what
breaks under ClickHouse. CH has no row-level UPSERT; all merging
is asynchronous and engine-driven. The rest of this note works
out the substitute.

---

## 1. CH engine primer for the merge cases we care about

| Engine | What it does on merge | Where it fits here |
|---|---|---|
| `MergeTree` | Sorts by `ORDER BY`; never dedups. | Append-only event tables (oracle_prices, trade fact table) |
| `ReplacingMergeTree(version)` | Keeps the row with the largest `version` per ORDER BY tuple. | Mutable rows with last-write-wins (`assets`, `backfill_progress`, optionally `current_prices`) |
| `SummingMergeTree([cols])` | Sums numeric columns per ORDER BY tuple. | Pure additive aggregates only — not enough for OHLCV (open/close are positional, not additive) |
| `AggregatingMergeTree` | Stores `AggregateFunction(...)` states; merges by combining states. | OHLCV with deterministic argMin/argMax/sum/min/max — the canonical fit |
| `CollapsingMergeTree` / `VersionedCollapsingMergeTree` | Cancels paired +/- rows. | Not applicable here |

**Critical caveat: eventual consistency.** CH merges in the
background; reads without `FINAL` (or without a `GROUP BY` that
re-merges) may see un-merged duplicates. Two ways to ensure
read-time correctness:

- **`SELECT … FINAL`** — forces a synchronous merge of the
  selected parts for the queried key range. Reads slower (~2-5×)
  but semantically simple.
- **`SELECT … GROUP BY <ORDER BY tuple>` with the matching
  `*Merge` combinators** — explicit re-aggregation at read time;
  faster than `FINAL` and idiomatic for `AggregatingMergeTree`.

The mapping below assumes API read paths use the `GROUP BY` form
(faster) and admin/debug queries can fall back to `FINAL`.

---

## 2. `price_ohlcv` — the central mapping

The merge formula from ADR 0004 has **five distinct aggregation
shapes** on one row:

| Column | PG merge semantics | CH aggregation primitive |
|---|---|---|
| `open` | Earliest `first_trade_at` wins | `argMin(open, first_trade_at)` |
| `close` | Latest `last_trade_at` wins | `argMax(close, last_trade_at)` |
| `high` | `GREATEST` | `max(high)` |
| `low` | `LEAST` | `min(low)` |
| `volume_base` | `SUM` | `sum(volume_base)` |
| `volume_quote_usd` | `SUM` | `sum(volume_quote_usd)` |
| `trade_count` | `SUM` | `sum(trade_count)` |
| `first_trade_at` | `LEAST` | `min(first_trade_at)` |
| `last_trade_at` | `GREATEST` | `max(last_trade_at)` |
| `vwap` | Recomputed `Σvol_quote / Σvol_base` | Derived at read time |
| `source` | `'aggregated'` if mixed, else single | Derived from `groupUniqArray` |
| `sources_seen` | `jsonb_set` per-source slot | Native CH `Map(String, Tuple(...))` or per-source rows |

This decomposes into two viable storage models. The trade-off is
which side pays the merge complexity: writer or reader.

### 2.1 Option CH-A: Single row per `(ts, asset, quote, granularity)` — `AggregatingMergeTree`

Match the PG row shape 1:1. The writer emits state-encoded values
using the `*State` combinator; CH merges states in the background;
the API reads use `*Merge` to finalize.

```sql
CREATE TABLE price_ohlcv
(
  timestamp        DateTime64(3, 'UTC'),
  asset_id         Int32,
  quote_asset_id   Int32,
  granularity      LowCardinality(String),

  open_state       AggregateFunction(argMin, Decimal128(14), DateTime64(3, 'UTC')),
  close_state      AggregateFunction(argMax, Decimal128(14), DateTime64(3, 'UTC')),
  high_state       AggregateFunction(max, Decimal128(14)),
  low_state        AggregateFunction(min, Decimal128(14)),
  volume_base_state          AggregateFunction(sum, Decimal128(14)),
  volume_quote_usd_state     AggregateFunction(sum, Decimal128(14)),
  trade_count_state          AggregateFunction(sum, UInt64),
  first_trade_at_state       AggregateFunction(min, DateTime64(3, 'UTC')),
  last_trade_at_state        AggregateFunction(max, DateTime64(3, 'UTC')),
  sources_state              AggregateFunction(groupUniqArray, LowCardinality(String))
)
ENGINE = AggregatingMergeTree
PARTITION BY toYYYYMM(timestamp)
ORDER BY (asset_id, quote_asset_id, granularity, timestamp);
```

**Writer** (per-bucket, per-source):

```sql
INSERT INTO price_ohlcv SELECT
  ts, asset_id, quote_asset_id, granularity,
  argMinState(open,  first_trade_at) AS open_state,
  argMaxState(close, last_trade_at)  AS close_state,
  maxState(high)                     AS high_state,
  minState(low)                      AS low_state,
  sumState(volume_base)              AS volume_base_state,
  sumState(volume_quote_usd)         AS volume_quote_usd_state,
  sumState(trade_count)              AS trade_count_state,
  minState(first_trade_at)           AS first_trade_at_state,
  maxState(last_trade_at)            AS last_trade_at_state,
  groupUniqArrayState(source)        AS sources_state
FROM input(...)
GROUP BY ts, asset_id, quote_asset_id, granularity;
```

**Reader** — the API query becomes (with explicit re-aggregation):

```sql
SELECT
  timestamp,
  asset_id,
  quote_asset_id,
  granularity,
  argMinMerge(open_state)              AS open,
  argMaxMerge(close_state)             AS close,
  maxMerge(high_state)                 AS high,
  minMerge(low_state)                  AS low,
  sumMerge(volume_base_state)          AS volume_base,
  sumMerge(volume_quote_usd_state)     AS volume_quote_usd,
  sumMerge(trade_count_state)          AS trade_count,
  minMerge(first_trade_at_state)       AS first_trade_at,
  maxMerge(last_trade_at_state)        AS last_trade_at,
  groupUniqArrayMerge(sources_state)   AS sources,
  sumMerge(volume_quote_usd_state) /
    nullIf(sumMerge(volume_base_state), 0) AS vwap,
  if(length(groupUniqArrayMerge(sources_state)) = 1,
     groupUniqArrayMerge(sources_state)[1],
     'aggregated')                        AS source
FROM price_ohlcv
WHERE asset_id = ? AND granularity = '1m'
  AND timestamp >= ? AND timestamp < ?
GROUP BY timestamp, asset_id, quote_asset_id, granularity;
```

**Pros:**

- Storage model maps 1:1 to the PG row. Every API consumer that
  thinks in `(timestamp, asset, quote, granularity)` terms still
  works.
- All ADR 0004 semantics preserved exactly: deterministic `open` /
  `close`, accurate min/max, additive sums, per-source breakdown
  recoverable from `groupUniqArray` (count and identity, though
  not per-source volumes — see CH-B for that case).
- Writer is a single `INSERT` per minute-bucket; no UPSERT
  dance.

**Cons:**

- `AggregateFunction` states are opaque binary blobs — debugging
  by direct SQL is harder.
- `sources_seen`'s rich per-source breakdown (volume per source,
  per-source first/last trade) is **not preserved** here.
  `groupUniqArray` gives the set of sources but not per-source
  numerics. If the API needs `sources_seen` shape (it does, per
  schema doc §3.3), this option needs a sidecar table or
  composite key shift — at which point CH-B is just simpler.

### 2.2 Option CH-B: One row per `(ts, asset, quote, granularity, source)` — `ReplacingMergeTree`

Push `source` into the ORDER BY tuple. Each writer emits one row
per minute it touches a pair. Cross-source aggregation moves to
read-time `GROUP BY`.

```sql
CREATE TABLE price_ohlcv
(
  timestamp        DateTime64(3, 'UTC'),
  asset_id         Int32,
  quote_asset_id   Int32,
  granularity      LowCardinality(String),
  source           LowCardinality(String),

  open             Decimal128(14),
  high             Decimal128(14),
  low              Decimal128(14),
  close            Decimal128(14),
  volume_base      Decimal128(14),
  volume_quote_usd Decimal128(14),
  trade_count      UInt64,
  vwap             Decimal128(14),
  first_trade_at   DateTime64(3, 'UTC'),
  last_trade_at    DateTime64(3, 'UTC'),

  version          DateTime64(3, 'UTC')   -- writer's wall-clock at insert
)
ENGINE = ReplacingMergeTree(version)
PARTITION BY toYYYYMM(timestamp)
ORDER BY (asset_id, quote_asset_id, granularity, source, timestamp);
```

**Writer.** The Lambda holds the in-memory per-(key, source)
accumulator (same as the PG flow), but the flush emits one row:

```sql
INSERT INTO price_ohlcv VALUES (
  ts, asset_id, quote_asset_id, '1m', 'sdex',
  open, high, low, close, volume_base, volume_quote_usd, trade_count,
  volume_quote_usd / nullIf(volume_base, 0),  -- vwap precomputed
  first_trade_at, last_trade_at,
  now64()  -- version
);
```

Idempotency: a second invocation for the same `(asset, quote,
minute, source)` writes a row with a higher `version`; the
`ReplacingMergeTree` keeps the latest at merge time.

**Reader.** The API query does the source merge in SQL:

```sql
WITH per_source AS (
  SELECT *
  FROM price_ohlcv FINAL  -- or use argMax + GROUP BY explicitly
  WHERE asset_id = ?
    AND granularity = '1m'
    AND timestamp >= ?
    AND timestamp <  ?
)
SELECT
  timestamp,
  asset_id,
  quote_asset_id,
  argMin(open,  first_trade_at) AS open,
  argMax(close, last_trade_at)  AS close,
  max(high)                     AS high,
  min(low)                      AS low,
  sum(volume_base)              AS volume_base,
  sum(volume_quote_usd)         AS volume_quote_usd,
  sum(trade_count)              AS trade_count,
  sum(volume_quote_usd) / nullIf(sum(volume_base), 0) AS vwap,
  min(first_trade_at) AS first_trade_at,
  max(last_trade_at)  AS last_trade_at,
  if(uniqExact(source) = 1, anyLast(source), 'aggregated') AS source,
  mapFromArrays(
    groupArray(source),
    groupArray((volume_base, volume_quote_usd, trade_count, first_trade_at, last_trade_at))
  )                              AS sources_seen
FROM per_source
GROUP BY timestamp, asset_id, quote_asset_id;
```

**Pros:**

- Writer is trivially idempotent: re-emit the same row, dedup
  on merge. Matches the S3-event retry semantics of the live
  ingest model.
- **`sources_seen` is recoverable in full** — per-source volume,
  per-source trade count, per-source first/last trade — because
  it is literally the underlying row set.
- Debuggable: rows are flat, no `AggregateFunction` state blobs.
- Per-source attribution is the natural shape; the "single row
  with source = 'aggregated'" idiom is a read-time projection,
  not a storage concern.
- Cleanest fit for CH-native rollups (§3 below): a SummingMergeTree
  MV over `(ts, asset, quote, granularity)` falls out trivially.

**Cons:**

- Row count is multiplied by source-cardinality. With ~4 sources
  (SDEX + Soroswap + Aquarius + Phoenix), upper bound is 4× the
  PG row count. For ~8 GB/year projected in PG, that is ~32 GB.
  Still tiny by CH standards (the BE indexer is in the hundreds
  of GB).
- API read path is more SQL. Mitigated by encapsulating it in
  views.
- `FINAL` cost on reads — though `ReplacingMergeTree`'s `FINAL`
  is cheap when the merge has converged (which it does for
  rows older than the per-part merge interval, typically
  minutes).

### 2.3 Recommendation seed (defer final pick to S-*)

**Working hypothesis: CH-B (one row per source).** Rationale:

1. `sources_seen` precision is required by the existing API
   schema (§3.3 of design doc) and only CH-B preserves it
   without sidecar tables.
2. Writer simplicity matters more than reader simplicity for an
   S3-event-driven Lambda — retries are the common case.
3. CH-B composes naturally with rollups (§3); CH-A requires
   chained `AggregatingMergeTree` MV's, which are stricter about
   `*State` discipline.
4. Storage cost difference is irrelevant at projected volumes.

CH-A is the right answer if `sources_seen` precision is dropped
from the contract. Surface as open question.

---

## 3. Rollups (15m → 1h → 4h → 1d → 1w → 1M)

PG today: separate **OHLCV Rollup** Lambda, EventBridge rate(15
min), rolls 1m → ... → 1M into the same `price_ohlcv` table with
different `granularity` values.

**CH-native alternative.** Chain of materialized views, each
reading from the prior granularity and writing aggregated rows
into the next.

Sketch under the CH-B model:

```sql
CREATE MATERIALIZED VIEW price_ohlcv_15m_mv
TO price_ohlcv_15m
AS SELECT
  toStartOfInterval(timestamp, INTERVAL 15 MINUTE) AS timestamp,
  asset_id,
  quote_asset_id,
  '15m'                       AS granularity,
  source,
  argMin(open,  first_trade_at) AS open,
  argMax(close, last_trade_at)  AS close,
  max(high)                     AS high,
  min(low)                      AS low,
  sum(volume_base)              AS volume_base,
  sum(volume_quote_usd)         AS volume_quote_usd,
  sum(trade_count)              AS trade_count,
  sum(volume_quote_usd) / nullIf(sum(volume_base), 0) AS vwap,
  min(first_trade_at)           AS first_trade_at,
  max(last_trade_at)            AS last_trade_at,
  max(version)                  AS version
FROM price_ohlcv
WHERE granularity = '1m'
GROUP BY toStartOfInterval(timestamp, INTERVAL 15 MINUTE),
         asset_id, quote_asset_id, source;
```

…and analogous MVs `15m → 1h → 4h → 1d → 1w → 1M`.

**Pros:**

- **Eliminates the OHLCV Rollup Lambda entirely.** CH does the
  rollup as part of insert flow. One less Lambda to provision,
  deploy, and monitor.
- Rollup latency drops from 15-minute scheduled cadence to
  insert-time + merge-window (~seconds).
- Same merge math expressed declaratively.

**Cons:**

- Coupling: the MV chain is part of CH schema. Changes require
  CH DDL coordination (touches schema-ownership boundary —
  step 5).
- Backfill story: rolling up backfill data requires either the
  MV to be in place when backfill writes, OR a one-shot
  `INSERT INTO price_ohlcv_15m_mv SELECT …` to populate after
  the fact. Doable but non-trivial.

**Alternative.** Keep the Lambda but point it at CH (read from
`price_ohlcv WHERE granularity = '1m' AND timestamp >= ...` and
INSERT into the same table with rollup granularity). Mechanical
port, no schema coupling improvement, but easier migration.

**Recommendation seed.** MV chain. The Lambda-savings + lower
latency outweigh the schema-coupling cost; the coupling is
addressed by the answer to step 5 anyway (separate `prices`
database with its own migration tooling).

---

## 4. `current_prices` (VWAP across sources, 24h window)

PG today: `current_prices` table, one row per `asset_id`,
rewritten every minute by the **Current Price Updater** Lambda
reading `price_ohlcv` over a rolling 24h window.

Three CH-native options:

### 4.1 CH-MV-A: Materialized view aggregating 24h on the fly

A `LiveView`-style MV that maintains `(asset_id) → 24h-aggregate`
in real time. CH supports this via `Refreshable Materialized
View` (CH 24.x+) or a periodic refresh.

**Pros:** No external store. Same query path as everything else.

**Cons:** `Refreshable MV` is relatively new; behavior under
high write rate not battle-tested. 24h window is bigger than
typical incremental MV use cases.

### 4.2 CH-MV-B: Read-time aggregation (no `current_prices` table)

Drop the table; serve `/assets/{id}` by aggregating the last 24h
of `price_ohlcv` at request time. Cache the result at API
Gateway (already in the design — built-in response cache).

**Pros:** Strict source-of-truth; no stale `current_prices`.

**Cons:** Per-request CH read latency for `/assets` (a list
endpoint). Pagination over 24h aggregates is harder than over
a pre-computed table.

### 4.3 CH-MV-C: Keep the Lambda, write into a `ReplacingMergeTree(updated_at)` table on CH

Mechanical port: same Lambda, points at CH, writes the same
shape into a CH table. Cleanest migration story.

**Pros:** Minimal change to the existing Lambda design. Read
path is identical (point lookup by `asset_id`). Sorting indexes
(for `/assets` list endpoint sorts) are achievable in CH via
projections.

**Cons:** Still requires the Lambda; no architectural saving.

**Recommendation seed.** **CH-MV-C** in the first cut. It's the
lowest-risk migration. Revisit moving to **CH-MV-A** (refreshable
MV) as a follow-up if Lambda count is a meaningful cost.

---

## 5. Low-write-volume relational tables

`assets`, `backfill_progress`, and to a lesser extent `oracle_prices`
have different shapes from the time-series tables.

### 5.1 `oracle_prices`

Time-series, append-mostly, partitioned by `timestamp` in PG.
**Native CH fit** — `MergeTree` with `ORDER BY (asset_id,
oracle_name, timestamp)` and `PARTITION BY toYYYYMM(timestamp)`.
Equivalent storage cost; identical read patterns.

```sql
CREATE TABLE oracle_prices
(
  timestamp     DateTime64(3, 'UTC'),
  asset_id      Int32,
  oracle_name   LowCardinality(String),
  price_usd     Decimal128(14),
  raw_data      String  -- JSON-as-string; CH JSONExtract* at read time
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(timestamp)
ORDER BY (asset_id, oracle_name, timestamp);
```

### 5.2 `assets` (mutable registry)

Small (~10³ rows), occasionally mutated by the Asset Discovery
Lambda. CH is not great at mutations; the right idioms are:

- **`ReplacingMergeTree(updated_at)`** — last-write-wins on the
  PK. Updates land as INSERTs with a newer `updated_at`. Reads
  use `FINAL` (cheap on a small table) or `argMax(...)
  GROUP BY asset_id`.
- **CH `Dictionary` backed by an external source** — for hot
  point-lookup reads with sub-millisecond latency. Source could
  be a CSV/Parquet snapshot in S3, refreshed periodically.

For ~10³ rows even un-optimized `ReplacingMergeTree` reads are
sub-millisecond.

### 5.3 `backfill_progress` (2 mutable rows)

`ReplacingMergeTree(updated_at)` with the same pattern. Two rows.
Trivial.

### 5.4 Alternative: external small-table store

If we want to **decouple** prices-api's mutable state from BE's
CH cluster (because it touches the schema-ownership boundary
question), an external store is viable:

- **DynamoDB.** AWS-native, no cluster to manage, single-digit-ms
  point reads. Natural fit for the API Lambda runtime.
- **A small RDS Postgres.** Re-introduces the thing this refactor
  is trying to avoid.
- **S3 + JSON file refreshed periodically.** Crude but works for
  `assets` (read-mostly); not for `backfill_progress` (writes
  every push).

**Recommendation seed.** Put `assets` and `backfill_progress`
into CH as `ReplacingMergeTree(updated_at)` rows in the same
`prices` database. Avoid introducing DynamoDB for two tiny
tables — that's tech-stack creep. If schema-ownership concerns
end up dominant (step 5 outcome), revisit and move them to
DynamoDB.

---

## 6. Summary mapping table

| RDS target (PG plan) | CH target | Engine | Lambda change |
|---|---|---|---|
| `price_ohlcv` (1m) | `price_ohlcv` (per-source rows) | `ReplacingMergeTree(version)` | Ledger Processor: `INSERT` instead of `UPSERT` |
| `price_ohlcv` (15m..1M rollups) | MV chain `_15m_mv` → `_1h_mv` → … | each MV writes into a `ReplacingMergeTree` | **Rollup Lambda eliminated** |
| `current_prices` | `current_prices` (one row per asset) | `ReplacingMergeTree(updated_at)` | Current Price Updater retained, retargeted |
| `oracle_prices` | `oracle_prices` | `MergeTree` | Oracle Fetcher retained, retargeted |
| `assets` | `assets` (registry) | `ReplacingMergeTree(updated_at)` | Asset Discovery retained, retargeted |
| `backfill_progress` | `backfill_progress` | `ReplacingMergeTree(updated_at)` | sdex-cloud-push retargeted (see ADR 0005) |

**Cleanup Worker.** PG retention logic (`DROP TABLE` per
month-partition; `DELETE WHERE granularity='1m' AND timestamp <
now()-7d`) translates to CH:

- Partition drop: `ALTER TABLE price_ohlcv DROP PARTITION '202601'`
  — identical semantics.
- Granularity-bounded delete: CH `ALTER TABLE … DELETE` is a
  mutation (heavy). Better idiom: store fine-grained granularities
  in separate tables (one table per granularity) so the drop is
  by partition only. **This is a schema-design refinement** that
  argues for splitting `price_ohlcv` by granularity.

Capture as open question.

---

## 7. Open questions surfaced by step 2 (forwarded to README)

6. **`sources_seen` precision.** Is the per-source per-minute
   breakdown actually load-bearing for the API, or is the set of
   sources enough? Answer changes the CH-A vs. CH-B pick.
7. **One table per granularity vs. all-in-one.** CH cleanup
   semantics favor splitting `price_ohlcv` into `price_ohlcv_1m`,
   `price_ohlcv_15m`, …, `price_ohlcv_1M`. The PG plan has one
   partitioned table. Pick.
8. **Rollup ownership.** MV chain (CH-side, declarative) vs.
   periodic Lambda (compute-side, imperative). MV is strictly
   better on latency + Lambda count, but ties rollup logic to
   CH schema — coupling with step 5's schema-ownership outcome.
9. **`current_prices` materialization path.** Lambda-driven port
   first (CH-MV-C) vs. refreshable MV (CH-MV-A). Risk vs.
   architectural saving trade.
10. **Small-table location.** `assets` and `backfill_progress`
    in CH vs. DynamoDB. Defer until step 5 (schema-ownership)
    decides; default CH.

## 8. What step 2 does NOT cover

- The actual `init.sql` (DDL) for the chosen shape — that's a
  G-note in the implementation task that gets spawned if the
  recommendation is "go".
- mTLS connection pooling / driver choice (`clickhouse-rs`,
  `klickhouse`, etc.) — step 4 (auth and network).
- Schema-ownership and migration tooling — step 5.
- Performance budgets and capacity sizing — step 6 / 7.
