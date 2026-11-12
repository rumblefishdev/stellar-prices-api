---
id: "0149"
title: "Two writers with incompatible version arithmetic own close_usd — the 0114 sweep's repair is overwritten inside the MV window"
type: BUG
status: completed
assignee: akot
related_adr: ["0287", "0317", "0292"]
related_tasks: ["0144", "0146", "0148", "0114", "0095", "0143", "0286", "0203", "0151", "0228"]
tags:
  ["priority-medium", "effort-medium", "clickhouse", "data-correctness", "materialized-view", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/repair.rs"
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
history:
  - date: "2026-10-06"
    status: completed
    who: akot
    note: >
      Closed as superseded by [[0286]] (ADR 0287 §5), like [[0146]].
      Re-measured on prod: 0 coarse rows on any of the six tiers hold a USD
      value their children do not support, so TEST E's case cannot recur. The
      version arithmetic is unchanged and documented in ADR 0317. Further
      paths are hypothetical; no follow-up tasks, by Adam's decision. No code
      change, no regression test.
  - date: "2026-10-06"
    status: active
    who: akot
    note: "Activated and assigned to akot to work through the options."
  - date: "2026-08-05"
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0144]] future work (phase 7) — finding 3ii-b, confirmed on
      the prod CH pin (TEST E). Deliberately ordered after [[0146]], which
      removes most of its consequence.
---

# Sweep vs MV: incompatible version arithmetic on `close_usd`

## Summary

Two writers update `close_usd` on the same coarse rows and claim the row by
different rules:

- The [[0114]] coarse sweep wins by **`version + 1`** (`repair.rs:20-22`).
- The rollup MVs write at **`version = sum(version)`** over their sub-rows —
  and every enrichment event underneath adds 1 to that sum.

Two enrichment events and the MV overtakes the sweep. Worse, the MV gets far
more attempts: `mv_ohlcv_15m_to_1h` re-appends the same hour **every 15 minutes
for 8 hours** (`REFRESH EVERY 15 MINUTE`, window `now() - INTERVAL 8 HOUR`).
The sweep gets one.

Confirmed on CH **26.3.10.60** ([[0144]] `repro/04_sweep_durability.sql`,
TEST E):

```
after the sweep repairs it        close_usd 0.171   version 401
after the next MV refresh         close_usd 0       version 402
what the view publishes now       (empty)
```

So the repair path we already built is defeated inside the re-aggregation
window — the bucket vanishes from `price_usd_series*` again, which is BE's
observation exactly.

## Why this is ordered late

**[[0146]] removes most of the consequence.** Once the MVs use `argMaxIf`, the
re-append writes a *correct* value rather than a zero — so being overtaken by
the MV stops being harmful inside the window. And outside the window the MV
does not write at all, so [[0148]]'s historical sweep is unaffected either way.

That leaves this as a latent trap rather than a live defect: **two writers with
incompatible version arithmetic on one column**, with no mechanism preventing
the next such writer from colliding the same way. Worth closing on its own
terms, not urgent.

Fixing only this and not [[0146]] would leave the zero-propagation completely
intact — the sweep would win a race to publish a value the MV is about to
recompute as zero anyway.

## Implementation

Options to weigh, none obviously correct yet:

- Give the sweep a version domain the MV cannot reach (e.g. a large offset), at
  the cost of making `version` no longer a plain event count.
- Have the sweep write through the same path as the MV so there is one writer.
- Make the sweep skip rows still inside an MV's re-aggregation window entirely,
  and rely on [[0146]] for those — smallest change, but encodes the window
  boundaries in two places.
- Reconsider whether `close_usd` should be MV-carried at all rather than always
  derived by the sweep.

Related: [[0143]] records that there is no `DEPENDS ON` anywhere in the cascade,
so ordering between tiers is already unsynchronised; any fix here should not
assume refresh ordering it does not control.

## Re-measured on prod, 2026-10-06

Measured as `dev_read` (read-only), 08:23–08:40 UTC. The queries are kept
outside the repo.

**What changed since this task was written.**

- The version arithmetic is unchanged. The sweep still writes `version + 1`
  (`repair.rs:22`, `ch_enrich.rs:2446/2490/2717`). Every MV still writes
  `sum(version)` (`rollups.sql:151-163`), and 0286 kept it.
- [[0146]] was closed as superseded by [[0286]]. Since 2026-09-22 a coarse
  `close_usd` is `close × the latest priced child's rate` (ADR 0287 §5,
  `rollups.sql:65-75`). The MV writes 0 only when no child is priced, not
  whenever the last child is unpriced. TEST E's step 4 now yields the same value
  the sweep wrote.
- [[0143]]/[[0203]] (live 2026-10-05) added a third writer: six reconcile MVs
  with a 7-day window, hourly. They write only buckets whose `trade_count` or
  `volume_base` disagree, so they do not touch a sweep repair by themselves.
- ADR 0317 "Version analysis" already describes the arithmetic (sweep `+1`,
  `coarse-repair` `+2`, ties go to the last insert) and accepts that a
  re-derived `close_usd` "can briefly go to 0 … until the next sweep pass".
- The one design that would resolve this by construction, a USD rate table with
  `close_usd` derived ([[0151]]), was rejected in ADR 0292.
- The fast windows are not all short. 1w re-rolls 60 days and 1M re-rolls
  **400 days**, daily. The recurring sweep covers 2 months
  (`COARSE_SWEEP_LOOKBACK_MONTHS`).

**Exposure.** A row is at risk when it holds a USD value that its children do
not support, because the next MV write that wins would replace it.

| tier (fast window) | rows | priced | last written by sweep | priced, no priced child |
|---|---|---|---|---|
| 15m (2 h) | 10,716 | 2,236 | 0 | 0 |
| 1h (8 h) | 25,444 | 5,948 | 11 | 0 |
| 4h (1 d) | 46,901 | 11,220 | 35 | 0 |
| 1d (7 d) | 117,586 | 29,977 | 241 | 0 |
| 1w (60 d) | 251,015 | 88,003 | 905 | 0 |
| 1M (400 d) | 564,712 | 275,966 | 448 | 0 |

- "Last written by sweep" is `version > sum(children)`. Each of these rows holds
  the USD value its children already give, so the sweep only got there first.
  The same holds for `volume_quote_usd`: 0 rows priced over unpriced children,
  on every tier.
- Residue at the 1e-12 floor, not this defect:
  - 1d: one row priced at 1.0e-12 over children below the floor, and 15 rows
    whose `close_usd` is ~1e-13.
  - 1M: 1,302 rows at `close_usd = 0` whose 1d children are all below the floor
    (max 9.9e-13), all last written by the MV. That is the floor working as
    `rollups.sql:70-73` intends.

**Verdict.** TEST E's case cannot recur. Its zero came from the MV carrying the
last child's `close_usd` when that child was unpriced. Since 0286 the MV
re-prices the bucket from the latest priced child, so it writes the same value
the sweep wrote. The race still happens, because the version arithmetic is
unchanged, but losing it costs nothing.

## Closing decision (2026-10-06, Adam)

Closed as superseded by [[0286]] (ADR 0287 §5), like [[0146]]. The measurement
above is the evidence. Other ways the race could matter again are hypothetical,
and we deliberately create no tasks for them:

- a coarse-only price correction inside an MV window;
- a coarse tier pricing where its children cannot;
- a regression of the rate form;
- a new writer with its own version arithmetic.

## Acceptance Criteria

- [x] A single documented owner for `close_usd` on coarse rows, or a version
      scheme under which the sweep's repair provably survives an MV refresh.
      Met differently: the repair is still overwritten, but with the same
      value. The arithmetic and the accepted brief zero are documented in ADR
      0317 "Version analysis"; the rate form in ADR 0287 §5.
- [ ] Regression test on CH 26.3.10.60 reproducing TEST E and showing the
      repair now survives. Not written, by decision; the prod measurement of
      2026-10-06 stands in for it.
- [x] The `version` column's semantics restated in the schema header if they
      change. They did not change.
- [x] Confirmed not to reintroduce the [[0095]] replace-mode invariants problem.
      No schema or MV change was made; every MV is still `APPEND`.
