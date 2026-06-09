---
title: "Rollup-chain semantics under enriched _1m re-inserts — decision + proof plan"
type: generation
status: developing
spawns: []
tags: [clickhouse, materialized-views, rollups, replacingmergetree, aggregatingmergetree, refreshable-mv]
links:
  - "https://clickhouse.com/docs/materialized-view/incremental-materialized-view"
  - "../../../../docs/database-schema/database-schema-overview.md"
  - "../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../blocked/0026_FEATURE_volume-quote-usd-enrichment-impl/notes/G-local-prototype-spec.md"
  - "../../backlog/0051_FEATURE_clickhouse-prices-schema-and-mv-chain-migration.md"
history:
  - date: 2026-06-09
    status: developing
    who: okarcz
    note: >
      Decision + proof plan authored without a live ClickHouse. Analyses
      the draft insert-trigger MV DDL in schema-overview §3.2 against CH
      insert-block MV mechanics; finds it under-counts independent of
      enrichment; recommends a refreshable / re-aggregate-from-FINAL
      rollup. Proof execution (local docker CH) deferred to a follow-up.
---

# Rollup-chain semantics under enriched `_1m` re-inserts — decision + proof plan

## 0. TL;DR

The draft insert-trigger materialised-view rollup in
[`database-schema-overview.md` §3.2](../../../../docs/database-schema/database-schema-overview.md)
(`mv_ohlcv_1m_to_15m`, lines ~461–479) is **incorrect — and not only under
enrichment.** A ClickHouse MV fires on the *inserted block*, aggregating only
the rows in that one INSERT. Because the rollup target is
`ReplacingMergeTree(version)`, two independent failures follow:

1. **Multi-block under-count (live path, no enrichment needed).** A 15-minute
   bucket is fed by ~15 separate per-minute INSERTs. Each fires the MV, each
   emits a *partial* `_15m` row with the **same** sort key
   `(asset_id, quote_asset_id, source, 15m-bucket-start)`. `ReplacingMergeTree`
   keeps exactly one — the `max(version)` partial — so the bucket collapses to
   roughly *the last minute's contribution*, not the sum of fifteen. Volumes
   read ~1/15 of truth.
2. **Enrichment double-/mis-count (the 0059 trigger).** When task 0026
   re-INSERTs a corrected `_1m` row with `version+1`, the MV fires again and
   emits another partial for that bucket. Whatever combine-or-replace rule the
   target uses, a *restated* contribution is indistinguishable from a *new*
   one.

The root cause is a fundamental tension: **incremental combine-on-insert and
correction-by-re-insert cannot both be served by a plain insert-trigger MV.**

**Decision (recommended): make the rollups re-aggregate from `_1m FINAL`
rather than sum the insert block** — implemented as a **Refreshable
Materialized View** (ClickHouse ≥ 23.12) or, if the cluster's CH version
can't be relied on, a scheduled `INSERT … SELECT … FROM _1m FINAL` re-aggregate.
Targets stay `ReplacingMergeTree(version)` with `version = max(source.version)`.
This is correct by construction on **both** failure modes, keeps rollups
"inside ClickHouse / no Rollup Lambda" (honouring ADR 0007 §3.4), preserves the
identical-shape target tables (no `AggregateFunction` columns → read path 0040
stays `FINAL`-simple), and yields the *more accurate* `volume_quote_usd`
(Σ of per-minute enriched values, not one coarse-bucket oracle multiply).

This finding is bigger than 0059's original framing: it means **0051 must not
ship the draft insert-trigger DDL as written.** Recommend an ADR-0007 amendment
(or a new ADR) recording the refreshable-rollup decision.

---

## 1. How a ClickHouse MV actually fires (the mechanic everything hinges on)

An insert-trigger MV in ClickHouse is **not** a maintained view of the source
table. It is an **INSERT trigger**: when a block of rows is inserted into the
source table, the MV's `SELECT` runs **over just that block**, and its output
is inserted into the target table. It does **not** read the target, and it does
**not** re-read the rest of the source bucket.

This is ClickHouse's own documented definition, not an inference. From the
[official incremental-MV docs](https://clickhouse.com/docs/materialized-view/incremental-materialized-view)
(retrieved 2026-06-09):

> "a ClickHouse materialized view is just a trigger that runs a query on blocks
> of data as they're inserted into a table."

> "When new rows are inserted into this table, ClickHouse executes the
> materialized view query *only* with those newly inserted rows."

> "the source table in the materialized view's query is replaced with the
> inserted block of data."

That third quote is the mechanically decisive one: inside the MV's `SELECT`,
`FROM price_ohlcv_1m` is substituted with *just the inserted block*, so a
`GROUP BY 15m-bucket` aggregates only the rows in that one INSERT — never a
re-read of the whole bucket. (The docs note the partial results are then sent
to the target table to be "updated and merged" by the target engine — the
"What the target engine does with N same-key rows" table below is exactly that
reconciliation step.)

> **Documented vs. deduced.** The block-only trigger behaviour above (and the
> `ReplacingMergeTree` "keep one row per sort key" rule) are documented facts.
> The specific *1/N under-count* conclusion in the table below is a sound
> deduction composed from those two facts — the §5 proof plan exists to confirm
> it empirically against a real ClickHouse before 0051 commits.

Consequences for a `GROUP BY 15m-bucket` rollup:

- If all 15 minutes of a bucket arrive in **one** INSERT block → one correct
  full-bucket partial. (This is the only case the draft DDL handles.)
- If they arrive in **N** blocks → **N** partial rows, same target sort key,
  each holding a sub-sum. The target engine now has to reconcile N partials.

What the target engine does with N same-key rows:

| Target engine | Reconciliation of N partial rows | OHLC correctness | Volume sum correct? |
|---|---|---|---|
| `ReplacingMergeTree(version)` (draft) | keep 1 (max version) | only last block's open/close survive | **No — keeps 1/N** |
| `SummingMergeTree` | sum *all* numeric cols | **No — sums open/high/low/close too → garbage** | yes for volumes |
| `AggregatingMergeTree` + `*State` cols | merge agg states | **Yes** (argMin/argMax/max/min/sum compose) | yes |

So even ignoring enrichment, the draft is wrong, and the naïve fixes each have
a catch. `AggregatingMergeTree` is the only insert-trigger option that is
*structurally* correct for OHLC — but it changes the target column types to
`AggregateFunction(...)`, breaking `CREATE TABLE _15m AS _1m` and forcing every
reader (0040) onto `-Merge`/`-State` semantics. Hold that thought.

## 2. Now add enrichment (the 0059-specific failure)

Enrichment (task 0026) corrects `_1m` by **re-INSERTing the row** with
`volume_quote_usd` filled and `version = original + 1`. The `_1m`
`ReplacingMergeTree` collapses old+new to the enriched winner. Good for `_1m`.

But the re-INSERT is *an insert into the MV's source table*, so the MV fires on
it. The emitted `_15m` partial carries the corrected row's
`volume_quote_usd` **and** its `volume_base` / `volume_quote` / `trade_count`
**again**:

- **`AggregatingMergeTree` (the "correct" insert-trigger engine):** `sumState`
  **adds** the re-inserted contribution to the already-aggregated bucket →
  `volume_base`, `volume_quote`, `trade_count` **double-count**;
  `volume_quote_usd` lands at `0 (original) + U (corrected)` which is *right by
  luck only because the original was 0* — it would be wrong for any
  re-correction.
- **`ReplacingMergeTree` (draft):** the `version+1` partial *wins* the bucket
  and the rollup becomes *only the enriched minute*, discarding the other 14.

**Conclusion:** a combine-on-insert MV cannot tell a *restatement* from a *new
contribution*. Correction-by-re-insert and incremental-combine are mutually
exclusive under plain insert-trigger MVs. You must break the tension.

## 3. Decision space

### Option A — Re-aggregate from `_1m FINAL` (Refreshable MV / scheduled) ✅ recommended

Rollups stop summing the insert block and instead **recompute the whole bucket
from the deduplicated source**:

```sql
-- Refreshable MV (CH >= 23.12) — runs on a schedule, inside ClickHouse.
CREATE MATERIALIZED VIEW prices.mv_ohlcv_1m_to_15m
REFRESH EVERY 1 MINUTE        -- coarser grains can refresh less often
TO prices.price_ohlcv_15m AS
SELECT
    toStartOfInterval(timestamp, INTERVAL 15 MINUTE) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  timestamp) AS open,
    max(high) AS high,
    min(low)  AS low,
    argMax(close, timestamp) AS close,
    sum(volume_base)      AS volume_base,
    sum(volume_quote)     AS volume_quote,
    sum(volume_quote_usd) AS volume_quote_usd,
    sum(volume_quote_usd) / nullIf(sum(volume_base), 0) AS vwap,
    sum(trade_count)      AS trade_count,
    max(version)          AS version
FROM prices.price_ohlcv_1m FINAL          -- <-- post-dedup, post-enrichment
WHERE timestamp >= now() - INTERVAL 2 HOUR  -- bounded re-scan; coarser grains widen
GROUP BY timestamp, asset_id, quote_asset_id, source;
```

Why it's correct:

- Reads `_1m FINAL` → sees the **enriched, deduplicated** rows, never partials.
- Recomputes the **whole bucket** each refresh → no multi-block under-count.
- Enrichment propagates automatically on the next refresh — **no version
  puzzle, no double-count.**
- Target `ReplacingMergeTree(version)` with `version = max(source.version)`:
  each refresh re-emits the full-bucket row; a later refresh with an enriched
  (higher) source version replaces cleanly.

Trade-offs:

- **Latency** = refresh interval (acceptable: 1m for `_15m`, widen for coarser).
- **Re-scan cost** per refresh, bounded by the `WHERE` window. At the project's
  ~0.48 GB/yr for `prices.*` (storage estimate doc) this is negligible.
- **CH version dependency.** Refreshable MVs are ≥ 23.12 and were experimental
  for a while. If the BE Hetzner cluster's version is uncertain, the *same
  query* runs as a scheduled job — see Option A′.

ADR reconciliation: ADR 0007 §3.4 eliminated the **Rollup Lambda**. A
refreshable MV keeps rollups **inside ClickHouse with no external scheduler**,
so the ADR's intent holds; only the MV *flavour* changes (refreshable vs
insert-trigger). Recommend recording this as an ADR-0007 amendment.

### Option A′ — Scheduled re-aggregate job (fallback if CH version can't guarantee refreshable MVs)

Identical `INSERT INTO _15m SELECT … FROM _1m FINAL … GROUP BY …` driven by a
tiny scheduler. **Caveat:** this resurrects a scheduled rollup step, partially
undoing ADR 0007 §3.4. If used, fold it into the existing **0039 `rollup-worker`**
rather than adding a new Lambda, and amend ADR 0007 explicitly. Prefer A.

### Option B — `AggregatingMergeTree` insert-trigger MVs

Structurally correct for multi-block aggregation, but (a) **double-counts under
enrichment re-inserts** (§2), (b) changes target tables to `AggregateFunction`
columns — breaks `CREATE TABLE _Ng AS _1m`, the backfill direct-write path
(§3.2 "backfill writes directly to higher granularities"), and forces 0040
readers onto `-Merge`. Rejected: solves the easy half, fails the enrichment
half, and taxes every reader.

### Option C — Enrich each granularity independently (no propagation)

Run the oracle ASOF join against `_15m`, `_1h`, … directly; nothing propagates.
Rejected: (a) still needs correct `volume_quote` rollups (same multi-block
problem one column over), (b) **less accurate** — one oracle price per coarse
bucket vs Σ of per-minute enriched values, (c) N× the enrichment passes.

### Option D — `VersionedCollapsingMergeTree` with cancel rows

Enrichment emits a `-1` cancel of the original plus a `+1` corrected row so the
sum nets out. Rejected: the cancel row must reproduce the original bucket
contribution *exactly*; fragile, hard to make idempotent, heavy operational
burden.

**Chosen: A** (with A′ as the version-constrained fallback).

## 4. Contract handed to task 0051

0051 owns the DDL; this note fixes the semantics it must implement:

- [ ] Rollup MVs **re-aggregate from `<source>_Ng FINAL`**, not from the insert
      block. Refreshable MV (Option A) preferred; scheduled re-aggregate (A′)
      only if the cluster CH version forces it.
- [ ] Each rollup `SELECT` projects `version = max(source.version)` and
      `sum(volume_quote)` **in addition to** `sum(volume_quote_usd)` (so the
      native quote volume is available at every grain — see task 0058).
- [ ] Rollup targets remain `ReplacingMergeTree(version)`, identical shape to
      `_1m` (no `AggregateFunction` columns).
- [ ] Refresh windows bounded per grain (`_15m` ~minutes, `_1M` ~hourly/daily);
      document each.
- [ ] The draft insert-trigger `mv_ohlcv_1m_to_15m` in schema-overview §3.2 is
      **superseded**; update that doc when 0051 lands the real DDL.
- [ ] ADR-0007 amendment (or new ADR) records refreshable-vs-insert-trigger.

## 5. Proof plan (local docker ClickHouse — execution deferred)

Goal: empirically demonstrate the draft's under-count, the enrichment
double-count, and the Option-A fix, on a minimal `_1m → _15m` chain. No live
infra; a throwaway `clickhouse/clickhouse-server` container.

**Fixtures.** One `(asset_id, quote_asset_id, source)` series; a single 15-minute
bucket of 15 one-minute rows; per-minute `volume_base = 10`, `volume_quote = 50`,
`volume_quote_usd = 0`; versions `v = minute_index` (monotonic).

**Experiment 1 — reproduce the draft bug.**
1. Create `_1m`, `_15m` (`ReplacingMergeTree(version)`) and the **draft
   insert-trigger** `mv_ohlcv_1m_to_15m`.
2. Insert the 15 rows in **15 separate INSERT statements** (simulating live
   per-minute arrival).
3. `SELECT … FROM _15m FINAL`. **Expect** `volume_base ≈ 10` (one minute), not
   `150` → confirms multi-block under-count.

**Experiment 2 — enrichment on the draft.**
4. Re-INSERT minute-7 with `volume_quote_usd = 500`, `version = 7_000+1`.
5. `SELECT … FROM _15m FINAL`. Record the (wrong) result → confirms restatement
   is mishandled.

**Experiment 3 — Option A fix.**
6. Drop the insert-trigger MV; create the **refreshable** MV reading
   `_1m FINAL` (or run the equivalent `INSERT … SELECT … FROM _1m FINAL` to
   simulate a refresh tick deterministically).
7. Re-run inserts (Exp 1) + the enrichment re-insert (Exp 2), trigger a refresh.
8. **Assert** `_15m FINAL` shows `volume_base = 150`, `volume_quote = 750`,
   `volume_quote_usd = 500`, `open = minute_1.open`, `close = minute_15.close`,
   `version = max(version)` — correct full bucket **and** enriched.

**Experiment 4 — chain depth.** Extend `_15m → _1h` to show the refresh pattern
composes across ≥ 2 hops (each grain reads the previous grain `FINAL`).

**Deliverables of the proof step (separate follow-up):** a
`proof/` dir with `schema.sql`, `seed.sql`, `assert.sql`, a `docker-compose` or
`run.sh`, and a short results table pasted back into this note (promoting it
`developing → mature`). This becomes the basis for AC #3's integration test.

## 6. Open questions for the BE cross-team sync

- What ClickHouse version runs on the shared Hetzner cluster? (Gates A vs A′.)
- Are refreshable MVs enabled / acceptable on the shared cluster, or is a
  scheduled re-aggregate (folded into 0039 `rollup-worker`) preferred operationally?
- Refresh cadence per grain that BE is comfortable with vs. read-freshness SLA.
