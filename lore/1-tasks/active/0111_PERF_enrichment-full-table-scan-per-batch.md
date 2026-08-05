---
id: "0111"
title: "Enrichment re-scans the whole table every batch — 545M rows/batch, caused a 4-day production outage"
type: PERF
status: active
related_adr: ["0007"]
related_tasks: ["0026", "0062", "0085", "0112", "0088"]
tags: [layer-indexing, clickhouse, enrichment, perf, priority-high, effort-medium, incident]
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-07-21
    status: backlog
    who: okarcz
    note: >
      Spawned from the enrichment timeout investigation. Root cause located in
      prod `system.query_log`, not inferred: both per-batch statements read
      490–545M rows each because their predicates are not in the sort key.
      Supersedes 0085 (closed — the pivot ref it targets measured 0.029s) and
      demotes 0062 to secondary (~11s of a ~35s batch).
  - date: 2026-07-21
    status: active
    who: okarcz
    note: >
      Promoted to active. Time-boxed: the ACs require re-measuring under the
      active backfill write load from 0088, which ends when its pass 2 finishes
      (~2026-07-31). Measure before choosing among the four options.
  - date: 2026-08-05
    status: active
    who: okarcz
    note: >
      **This task now blocks the largest win in the [[0144]] plan.** 0144's
      phase 0 measured that ~68% of every OHLCV tier has no USD price — 100.0%
      of exotic-quote rows, 71.9% of `_1d`, stable for 24 months — and the cause
      is the enrichment resolver's reach, not any read-surface bug. The fix is
      [[0154]] (a second pivot hop), which adds a join to this pass. Decided
      2026-08-05: **0111 ships before 0154.** So the existing framing —
      "not acutely urgent, cost scales with table size not era" — remains
      accurate about the *risk* and is now wrong about the *priority*: it is on
      the critical path of the biggest coverage improvement available to BE.
      Note also that the 0144 measurements re-confirmed the hourly cadence
      (`rate(1 hour)`, deployed rule matches `production.json`, no drift), so
      this task's schedule assumptions are unchanged.
---

# Enrichment re-scans the whole table every batch

## Summary

Every enrichment batch reads the **entire `price_ohlcv_1m` table twice** — once
for `count_candidates()`, once for the enrichment `INSERT … SELECT`'s outer scan.
At 545M rows that is ~35 s per batch, so a pass gets through ~8 of its 20 batches
before the 300 s Lambda timeout kills it.

This is not a latent risk. It **took enrichment down for four days** (2026-07-14
→ 07-17, 72/72 invocations failing daily) and degraded it either side, 07-10 →
07-18. No USD prices were written during that window and no alarm fired
(see [[0112]]).

## Evidence (prod `system.query_log`, measured 2026-07-21)

The enrichment `INSERT … SELECT` — the dominant cost:

| date | runs | avg | max | rows read/run |
|---|---|---|---|---|
| 07-13 | 164 | 21.4 s | 43.3 s | 411 M |
| 07-14 | 119 | 30.9 s | 43.3 s | 491 M |
| 07-15 | 138 | **31.3 s** | **62.9 s** | 533 M |
| 07-16 | 525 | 24.4 s | 38.8 s | 538 M |
| 07-17 | 620 | 23.9 s | 39.2 s | 545 M |

`count_candidates()` — secondary, ~1/3 of the cost:

| date | runs | avg | rows read/run |
|---|---|---|---|
| 07-15 | 143 | 15.0 s | 533 M |
| 07-16 | 545 | 11.7 s | 537 M |
| 07-17 | 622 | 11.2 s | 544 M |

Per batch ≈ **24 s + 11 s ≈ 35 s**, matching the CloudWatch
`EnrichmentAvgBatchDurationMs` peak of 43,249 ms. Run counts corroborate:
07-17 had 620 INSERTs ≈ 24 invocations × 3 retries × ~8 batches.

## Re-measured under load, 2026-07-21 — the framing above is incomplete

Measured while the tail backfill was actively writing (7,336 inserts/30 min), so
this is **not** the quiet-cluster illusion the ACs warn about:

| | before 2026-07-18 03:00 | after |
|---|---|---|
| `INSERT … SELECT` | 24–26 s, 549 M rows | **0.6–0.7 s, ~11–14 M rows** |
| `count_candidates` | 11–12 s, 549 M rows | **0.2–0.3 s, ~11–14 M rows** |
| runs/hr | 24–27 (3 retries × ~8 batches) | 20–21 (full pass), then 4–6 |

Per-batch went 35 s → ~1 s in one hour. `read_rows` is structural, not
load-dependent, so this is a change in the query's input, not the cluster's mood.

**Cause: `price_ohlcv_1m` returned to its designed size.** It is a rolling 7-day
window — history lives in the coarse tables (`init.sql:129-134`). Cleanup was
disabled ~07-08 for the backfill, so 1m grew to 545 M; [[0090]] re-enabled it
07-15; it fired 07-18 03:00 and dropped everything >7 d. The table is now 14.0 M
rows (3.69 M pre-Soroban + 10.3 M current month), matching the measured
`read_rows` exactly.

**So the exposure is not "enrichment is slow" — it is:**

> Enrichment's per-batch cost scales with `price_ohlcv_1m` size, which is bounded
> only by the cleanup rule — and every [[0088]] backfill or recovery requires
> disabling that rule for days.

That coupling is **live right now**: cleanup has been off since 07-20 and stays
off until ~2026-08-01, and the table is already climbing (11.3 M → 14.2 M over
3.5 days). Expect a recurrence before the recovery ends; 0112's
`-duration-near-timeout` alarm should now catch it.

This **demotes option 2** (skip-index/projection — it optimises a state that only
exists during backfills) and **promotes option 1** (partition-bounded passes —
partition pruning is exactly what makes cost independent of how many historical
partitions are sitting in the table).

⚠️ Chasing this step change is what surfaced [[0114]] — the coarse tables carry
no USD values for 2025-02 → 2026-02. That is the more serious defect and
outranks this task.

## Root cause

`price_ohlcv_1m` is `ORDER BY (asset_id, quote_asset_id, source, timestamp)`
(`packages/prices-clickhouse/schema/init.sql:122`).

Both hot statements filter on `close_usd = 0` / `volume_quote_usd = 0` /
`volume_quote > 0`. **None of those is in the sort key**, so ClickHouse has no
index to narrow with and full-scans with `FINAL` every time — re-deriving the
~8.4M-row backlog from 545M rows, per batch, twice.

## What this is NOT

Recorded because two plausible diagnoses were measured and refuted; do not
re-litigate them without new evidence.

- **NOT the pivot reference ([[0085]], now closed).** It filters
  `asset_id = 4 AND quote_asset_id = 3` — a genuine sort-key prefix — and
  measures **0.029 s**, under a tenth of a percent of the 24 s INSERT.
  Materialising it once saves nothing measurable.
- **NOT primarily `count_candidates` ([[0062]]).** Real, but 11 s of 35 s.
  Fixing only it leaves a 24 s batch: still a timeout.
- **NOT merge pressure / unmerged parts.** Hypothesised, then undercut: part
  counts are 3–10 per partition while the tail backfill is actively writing,
  and the same queries return in 0.3 s today against the same 532M-row table.

## Implementation — options to cost

1. **Bound each pass to a time window / partition** so a scan touches one
   monthly partition rather than all of them. `timestamp` is in the sort key but
   only 4th, so a partition predicate (`PARTITION BY toYYYYMM(timestamp)`) is the
   stronger lever — it prunes whole parts.
2. **Skip-index or projection on the USD-zero predicate**, giving the existing
   query an index to narrow with. Cheapest change to the worker (none), but adds
   write-side cost on a shared cluster — measure before committing.
3. **A pending-work table** written at ingest, so enrichment reads a small queue
   instead of re-deriving the backlog from a full scan. Biggest change, best
   asymptotics, and it also removes `count_candidates` entirely (subsumes 0062).
4. **Drop `FINAL` from the hot loop** where dedup is not load-bearing — the
   `version` column already orders writes.

Options 1 and 3 are complementary; 2 may be a fast stopgap.

## Acceptance Criteria

- [ ] A full `one_shot` drain over the current backlog completes inside the
      Lambda budget, measured — not inferred from a drained-state query.
- [ ] Per-batch rows-read is bounded and does not scale with total table size;
      verified in `system.query_log`, not by wall-clock on a quiet cluster.
- [ ] Re-measured **while the pre-Soroban tail backfill is actively writing** —
      a quiet cluster is how this was missed. The drained-state numbers (0.3 s)
      are 80× faster than the same query under load.
- [ ] The 8.4M rows currently sitting un-enriched are drained, and the count is
      recorded before/after.
- [ ] 0026's `EnrichmentPassDurationMs` stays well clear of 300 s for a week
      spanning active backfill.

**Baseline to judge the fix against** (drained state, measured 2026-07-21 via the
alarms deployed in [[0112]]): enrichment runs at **~4,500 ms against a 240,000 ms
warning threshold — about 2% of the timeout budget**, 1 invocation/hour on
schedule. That is a *quiet-cluster* number: the same work cost ~35 s/batch under
backfill load. A fix is only demonstrated when the loaded figure stays low, not
when the quiet one does.

`prices-production-enrichment-duration-near-timeout` now fires at 80% of the
timeout, so a recurrence surfaces as days of warning rather than a silent
outage — but it does not prevent one.

## Out of scope

- The alarm blind spot that let this run four days unnoticed — that is [[0112]].
- The `price_ohlcv_15m` pre-roll INSERT (217 s avg / 301 s max, 177M rows) —
  **now [[0113]]**. Note the description above was wrong when first written: it
  called this "the same 300 s wall", implying a Lambda timeout. The pre-roll is
  operator-run SQL, not a Lambda, so no timeout applies; the real exposure is
  memory (2,938 MB of a 5.59 GiB quota). 0113 carries the corrected framing.
