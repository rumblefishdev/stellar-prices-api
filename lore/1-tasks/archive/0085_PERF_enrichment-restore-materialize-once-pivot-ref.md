---
id: "0085"
title: "Restore enrichment materialize-once for the XLM/USDC pivot ref before the 0053 backfill (per-batch re-aggregation risks the 300s timeout)"
type: REFACTOR
status: completed
related_adr: ["0007"]
related_tasks: ["0083", "0053", "0026", "0084", "0111", "0062"]
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
  - date: 2026-07-21
    status: completed
    who: okarcz
    note: >
      CLOSED as not-the-bottleneck. Measured against prod CH: the pivot ref
      subquery is 0.029s (sort-key prefix on asset_id/quote_asset_id), vs the
      enrichment INSERT outer scan at 24s/batch reading 545M rows and
      count_candidates at 11s/544M in system.query_log over the 07-10 to 07-18
      outage. Implementing this as written would have shipped and left the
      4-day outage in place. Real cause + fix -> 0111; stale comment at
      ch_enrich.rs:292-293 and the snapshot-consistency point carried there.
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

- [~] Pivot reference computed once per run (not per batch), still grant-free
      (or via `CREATE TEMPORARY TABLE` only). **Not done — no longer worth
      doing**, see §Closure.
- [x] A full-backlog `one_shot` drain over a realistic multi-year XLM/USDC slice
      completes well within the Lambda budget (measure). **Measured 2026-07-21:
      the ref subquery is 0.029 s. It was never the constraint.**
- [ ] Frozen-snapshot restored (no cross-batch `close_usd` drift on a mutating
      tip). **Carried to [[0111]]** — still a real (low) consistency point, but
      it rides on whatever scan strategy 0111 lands, not on this fix.

## Closure — measured not the bottleneck (2026-07-21)

The premise was sound but the arithmetic was never checked. Measured against
prod ClickHouse:

| statement | measured |
|---|---|
| **pivot reference subquery (this task)** | **0.029 s** |
| `count_candidates` ([[0062]]) | 0.265 s |
| enrichment `INSERT … SELECT` outer scan | 0.315 s |

The reference filters `asset_id = 4 AND quote_asset_id = 3`, which **is** a
sort-key prefix on `(asset_id, quote_asset_id, source, timestamp)` — so each
batch reads only that pair's granules, exactly as `ch_enrich.rs:711` claims.
Re-running it 20× per pass costs ~0.6 s. Materialising it once saves nothing
measurable.

Meanwhile prod `system.query_log` for the 07-10 → 07-18 outage shows the real
cost: the enrichment INSERT at **24 s/batch reading 545M rows**, plus
`count_candidates` at **11 s reading 544M rows** — both full-table `FINAL`
scans, because their predicates are *not* in the sort key. That is [[0111]].

**Had this task been implemented as written, it would have shipped and left the
outage in place.** The task predicted the right symptom (300 s timeout under
backfill depth) and the wrong cause.

### What survives

- The **stale comment** at `ch_enrich.rs:292-293` still claims the reference is
  *"materializ[ed] once in `Self::run_peg_pivot_tier`"*, which contradicts the
  code. Carried to [[0111]] as a doc fix; it is wrong regardless of strategy.
- The **snapshot-consistency** point above → [[0111]].

Superseded by [[0111]]. Do not re-open without new measurement.
