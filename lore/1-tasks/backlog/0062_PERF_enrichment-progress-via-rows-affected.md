---
id: "0062"
title: "Enrichment loop: drive progress from INSERT rows-affected (gated on clickhouse-crate upgrade)"
type: PERF
status: backlog
related_adr: ["0007"]
related_tasks: ["0061", "0026", "0039", "0111", "0085"]
tags: [layer-database, clickhouse, enrichment, perf, priority-medium, effort-small, blocked-on-dependency]
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-06-17
    status: backlog
    who: claude
    note: >
      Spawned from 0061 code-review finding #10 (part 2). Part 1 (materialize the
      XLM/USDC pivot reference once per run) shipped on the 0061 branch; part 2 is
      deferred because the clean fix needs a clickhouse-crate capability we don't
      have pinned.
  - date: 2026-07-21
    status: backlog
    who: okarcz
    note: >
      RE-SCOPED from priority-low to priority-medium, and confirmed real by
      measurement. count_candidates ran 545 times/day at ~11s each reading 544M
      rows during the 07-10 to 07-18 enrichment outage (prod system.query_log).
      That is roughly a THIRD of a ~35s batch, so this is a genuine contributor
      -- but not the driver: the enrichment INSERT outer scan is ~24s. Fixing
      only this leaves a 24s batch and still times out. Sequence behind 0111.
      NOTE: PR #133 (0108 grooming, unmerged) archives this task as completed
      on the strength of the crate-upgrade blocker; that closure is WRONG on
      this evidence and must be reverted before #133 merges.
---

# Enrichment loop: drive progress from INSERT rows-affected

## Summary

The two-tier enrichment pass (`ChEnrichmentPass::run_through`,
`packages/enrichment-worker/src/ch_enrich.rs`) decides whether to keep looping by
calling `count_candidates()` after **every** batch — a full-`FINAL` merge-scan of
`price_ohlcv_1m`. A pass does up to `1 + 2·max_batches` of these (~41 at the
defaults). The cheaper, idiomatic signal is **rows actually written by each
`INSERT`**: a batch that wrote 0 rows means the tier is drained, so no post-batch
count is needed. Replace the per-batch count with the INSERT's `written_rows`.

## Context

Code-review finding #10, part 2, from task **0061**. Part 1 (the dominant cost —
the pivot re-aggregating the whole XLM/USDC series under `FINAL` on every batch)
was fixed on the 0061 branch by materializing the reference once
(`pivot_ref_sql` / `materialize_pivot_ref`, commit `ab673d6`). Part 2 is the
remaining, secondary cost: the repeated `FINAL` count scans.

**Why it's deferred, not done:** ClickHouse returns rows-affected in the
`X-ClickHouse-Summary` HTTP response header (`written_rows`), but the pinned
`clickhouse` 0.13 crate's `query(...).execute()` returns `Result<()>` and discards
that header — there is no API to read it. The available workarounds were judged
not worth it for a secondary optimization:
- **Raw HTTP for the INSERTs** — forks the enrichment INSERT path off the crate,
  re-implementing URL/auth/TLS/error-mapping the crate centralizes (real bug
  surface).
- **Workspace-wide crate upgrade** — `clickhouse` is shared by `prices-clickhouse`,
  `sdex-backfill`, and `enrichment-worker`; a major bump is its own migration.

The correctness hazard in this area (#5 — concurrent live inserts inflating the
count and tripping the no-progress break) is **already fixed** on 0061 by the
`max(timestamp)` snapshot watermark, which also bounds each count scan to a fixed
population. So what remains here is **purely cost**, hence low priority.

**Gate:** do this when the `clickhouse` crate is upgraded to a version that
surfaces the query summary (verify `written_rows` is reachable from `execute()` or
an equivalent), or if profiling on a large production backfill shows the per-batch
`FINAL` counts actually dominating pass time (which would justify the raw-HTTP
route on its own).

## Implementation

- Confirm the upgraded `clickhouse` crate exposes the response summary
  (`written_rows`) from the INSERT execution path.
- In `enrich_batch`, `peg_sql`, and `pivot_sql` execution, capture `written_rows`
  per `INSERT`.
- In `run_through` (oracle tier loop) and `run_peg_pivot_tier` (peg-pivot loop),
  replace the post-batch `count_candidates()` no-progress check with
  `written_rows == 0`. Keep ONE `count_candidates()` at pass start for
  `candidates_before` / the zero-enrichment `warn!` (#7) and the stats.
- Re-check the watermark interaction: the watermark (#5) stays; it bounds the
  candidate population. Rows-affected only replaces the *progress* signal.
- Update the doc note on `count_candidates` that currently records this constraint.

## Acceptance Criteria

- [ ] Per-batch full-`FINAL` `count_candidates()` calls removed from both tier
      loops; progress driven by INSERT rows-affected.
- [ ] At most one (or a small constant) `FINAL` count per pass remains (for
      `candidates_before` + stats + the #7 zero-enrichment warn).
- [ ] No regression: oracle/peg/pivot tiers still terminate correctly; the #5
      watermark snapshot semantics preserved; idempotent re-run unchanged.
- [ ] Live-CH integration tests (`ch_enrich_it.rs`) still green, incl. the
      `oracle_budget_exhaustion_defers_instead_of_pegging` and
      `watermark_defers_candles_newer_than_the_snapshot` cases.
- [ ] `count_candidates` doc note updated to reflect the new signal.

## Re-scope 2026-07-21 — measured, real, but secondary to 0111

Prod `system.query_log` over the 07-10 → 07-18 enrichment outage:

| date | `count_candidates` runs | avg | rows read/run |
|---|---|---|---|
| 07-15 | 143 | 15.0 s | 533 M |
| 07-16 | 545 | 11.7 s | 537 M |
| 07-17 | 622 | 11.2 s | 544 M |

So the concern in this task is **confirmed**, not theoretical: the per-batch
`FINAL` count is a full-table scan costing ~11 s and reading 544M rows.

But it is **~1/3 of a ~35 s batch**. The enrichment `INSERT … SELECT` outer scan
is the other ~24 s, and for the same reason (predicates not in the sort key).
Fixing only this task leaves a 24 s batch — still over the 300 s budget at
~12 batches. Sequence this **behind [[0111]]**, and note that 0111's option 3
(a pending-work table) would remove `count_candidates` entirely, making this
task moot rather than merely deferred.

The original blocker still stands: the clean fix needs `X-ClickHouse-Summary` /
`written_rows`, which the pinned `clickhouse` 0.13 crate does not surface.

> ⚠️ **PR #133 (0108 grooming, unmerged) archives this task as `completed`.**
> That closure predates this measurement and is wrong. Revert it before #133
> merges, or this reopens as a surprise later.
