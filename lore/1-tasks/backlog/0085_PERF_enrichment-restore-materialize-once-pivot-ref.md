---
id: "0085"
title: "Restore enrichment materialize-once for the XLM/USDC pivot ref before the 0053 backfill (per-batch re-aggregation risks the 300s timeout)"
type: REFACTOR
status: backlog
related_adr: ["0007"]
related_tasks: ["0083", "0053", "0026", "0084"]
tags: [layer-indexing, priority-medium, effort-small, rust, clickhouse, enrichment, performance, post-deploy]
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-07-06
    status: backlog
    who: okarcz
    note: >
      Spawned from the 0083 / PR #86 code review. That PR made enrichment
      grant-free by computing the XLM/USDC pivot reference inline as an ASOF-join
      subquery — correct + validated, but it reverses review #10's materialize-once
      and re-aggregates the ref per batch. Cheap now (prod has no backfill; the
      XLM/USDC slice is tiny), but grows with backfill depth × batch count.
  - date: 2026-07-20
    status: backlog
    who: okarcz
    note: >
      **Gate has EXPIRED — this is now overdue, not pending.** The task says
      "do this before the 0053 backfill runs"; that backfill has since run (the
      Soroban-era combined range completed 2026-07-15, and the pre-Soroban SDEX
      tail is in flight), so the multi-year XLM/USDC slice this was sized against
      now exists. Flagged during the 0108 grooming sweep.
      Re-verified unchanged: the pivot reference is still built inline per batch
      as an ASOF LEFT JOIN subquery (enrichment-worker/src/ch_enrich.rs:719-751),
      so ref work remains O(slice × batches) — up to max_batches=20 per hourly
      pass and unbounded in one_shot. The 300s Lambda-timeout risk this task
      predicted is now live rather than theoretical.
      Also noted while reading: the comment at ch_enrich.rs:293-294 still claims
      the reference is "materialized once in run_peg_pivot_tier", contradicting
      the actual code — fix that alongside.
---

# Restore materialize-once for the enrichment pivot reference

## Summary

PR #86 (task 0083) unblocked enrichment by inlining the volume-weighted XLM/USDC
reference as an `ASOF LEFT JOIN` subquery — no `CREATE TABLE` grant needed. The
trade-off: the reference is re-aggregated **once per batch** instead of once per
run, so total ref work is `O(slice × batches)` (up to `max_batches = 20` per
hourly pass; **unbounded in `one_shot`**). Harmless while XLM/USDC history is
small, but the 0053 historical backfill will grow the slice to millions of
1-minute rows and could push the enrichment Lambda past its **300s timeout** —
the same failure class the supply worker already hits (0084).

**Do this before the 0053 backfill runs.** (No urgency until then.)

## Options (all keep zero broad `CREATE TABLE` grant)

1. **Fetch the ref series into memory once per run**, then feed each batch's ASOF
   join from it. Keeps materialize-once with zero grants; the ref is one row per
   XLM/USDC minute — bounded, but a multi-year series is large, so stream/window it.
2. **Session-scoped `CREATE TEMPORARY TABLE`** — needs only the lighter
   `CREATE TEMPORARY TABLE` privilege (not `CREATE TABLE ON prices.*`), a clean
   posture BE would likely grant; restores the original once-per-run scan. Requires
   pinning the ClickHouse HTTP session across the batch statements.
3. Widen `batch_size` / cut batch count so the constant re-aggregation amortizes
   (partial mitigation only).

A `WITH` CTE does **not** help — ClickHouse inlines CTEs, so it re-runs per query.

## Also from the review (fold in)

- **Snapshot consistency (low):** the per-batch recompute reads the *live* table,
  so a mid-pass live-processor write to a still-mutating watermark bucket can
  enrich two candles at the same ref timestamp inconsistently within one run.
  Materialize-once (any option above) restores the frozen snapshot and closes this.

## Acceptance Criteria

- [ ] Pivot reference computed once per run (not per batch), still grant-free
      (or via `CREATE TEMPORARY TABLE` only).
- [ ] A full-backlog `one_shot` drain over a realistic multi-year XLM/USDC slice
      completes well within the Lambda budget (measure).
- [ ] Frozen-snapshot restored (no cross-batch `close_usd` drift on a mutating tip).
