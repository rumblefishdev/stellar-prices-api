---
id: "0111"
title: "Enrichment re-scans the whole table every batch — 545M rows/batch, caused a 4-day production outage"
type: PERF
status: backlog
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
