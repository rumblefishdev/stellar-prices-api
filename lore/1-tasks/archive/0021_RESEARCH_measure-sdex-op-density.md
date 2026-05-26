---
id: '0021'
title: 'Measure SDEX trade-shaped op density in CH operations_appearances'
type: RESEARCH
status: canceled
related_adr: ['0002']
related_tasks: ['0020', '0017', '0022']
tags:
  [
    layer-research,
    priority-medium,
    effort-small,
    research,
    sdex,
    backfill,
    clickhouse,
    measurement,
    stream-2,
    canceled,
  ]
links:
  - '../../2-adrs/0002_stream2-sdex-archive-backfill-independent-of-be.md'
  - '../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/S-sdex-backfill-recommendation.md'
  - '../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/I-stream2-options.md'
history:
  - date: 2026-05-12
    status: backlog
    who: okarcz
    note: >
      Spawned from task 0020 Option B/A discrimination. Single
      quantitative input — fraction of historical ledgers with at
      least one op of type ∈ {2, 3, 4, 12, 13} — decides whether
      the CH pre-filter plumbing is worth building over baseline
      Option A. Gated on task 0017's local CH being populated.
  - date: 2026-05-13
    status: canceled
    who: okarcz
    reason: pivot
    note: >
      Canceled by ADR 0002. The Option B (CH pre-filter) path was
      rejected on the grounds of BE-coupling cost vs bounded
      perf gain. With Option A locked in as the Stream 2
      architecture and BE runtime/data coupling explicitly
      removed, the trim-ratio measurement no longer has a
      decision to inform. The detailed SDEX filter/decode spec
      that this task was a partial input to is now task 0022.
---

# Measure SDEX trade-shaped op density in CH `operations_appearances`

## Summary

Run one CH query against the local CH instance populated by
task 0017 to compute the **trim ratio**: fraction of historical
Stellar ledgers (from genesis or from protocol-19 SDEX activation,
whichever is the lower bound the populated data covers) that
contain at least one operation of type ∈ {2, 3, 4, 12, 13}
(path-payment receive, manage-sell-offer, create-passive-sell-offer,
manage-buy-offer, path-payment-send).

The number decides Option B (CH pre-filter) vs Option A (no
pre-filter) for §5.6 Stream 2.

## Context

Per task 0020's I-note: trim ratio ≥ 50% → build the pre-filter
plumbing (modest win), trim ratio < 30% → skip the plumbing and
just run baseline Option A. No measurement available today
(BE has not published this number).

## Implementation

1. **Wait on task 0017.** This task is blocked until 0017's local
   CH instance is populated for at least one full epoch.
2. **Query:**

   ```sql
   WITH
     total AS (
       SELECT count(DISTINCT sequence) AS n
       FROM ledgers
       WHERE sequence BETWEEN ? AND ?
     ),
     trade_bearing AS (
       SELECT count(DISTINCT ledger_sequence) AS n
       FROM operations_appearances FINAL
       WHERE type IN (2, 3, 4, 12, 13)
         AND ledger_sequence BETWEEN ? AND ?
     )
   SELECT
     total.n           AS total_ledgers,
     trade_bearing.n   AS trade_bearing_ledgers,
     trade_bearing.n / total.n AS trim_inverse,
     1.0 - trade_bearing.n / total.n AS trim_ratio
   FROM total, trade_bearing;
   ```

3. **Repeat over decadal windows** (e.g. 2018, 2020, 2022, 2024, 2026) — trade-shaped op density has shifted across Stellar's
   history; the average over the whole range hides era-specific
   ratios. Soroban era (post 2023-11) skews differently because
   trades migrated partly to contract AMMs.
4. **Write up findings** in `notes/R-trim-ratio.md` with the
   per-era table.
5. **Decide:** if average trim ratio across the 57M-ledger range
   is ≥ 50%, recommend Option B; else recommend Option A
   without the pre-filter. Capture in a 5-line S-note.

## Acceptance Criteria

- [ ] Task 0017's local CH is populated and queryable.
- [ ] Trim ratio measured for ≥ 3 disjoint ledger windows spanning
      pre-Soroban and Soroban eras.
- [ ] Per-era table written in `notes/R-trim-ratio.md`.
- [ ] Final A-vs-B recommendation written in `notes/S-decision.md`.
- [ ] If recommendation is Option B, file an implementation
      follow-up referencing task 0012's Fargate design.
- [ ] If recommendation is Option A only, note explicitly that no
      further pre-filter work is needed.

## Notes

- Order-of-magnitude prior: most Stellar ledgers are sparse
  (1–2 transactions, often payments only). Trade-bearing ledger
  ratio could plausibly land anywhere from 10% (early years, no
  DEX activity) to 70% (peak SDEX usage circa 2021–2022). Don't
  assume one number; measure across eras.
- This is a small task — ≤ 1 hour once 0017 is queryable.
