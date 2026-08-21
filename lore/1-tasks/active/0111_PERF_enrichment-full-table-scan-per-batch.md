---
id: "0111"
title: "Enrichment re-scans the whole table every batch — 545M rows/batch, caused a 4-day production outage"
type: PERF
status: active
related_adr: ["0007"]
related_tasks: ["0026", "0062", "0085", "0112", "0088", "0209", "0212", "0215"]
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
  - date: 2026-08-06
    status: active
    who: okarcz
    note: >
      **The consumer has now sized what sits behind this task.** BE measured all
      52,369 classic pools: only **44.4% have both legs priceable** on the 48h
      window their headline TVL uses. They falsified the recency explanation
      themselves (worst-leg staleness ≤2d 44.7% → ≤7d 46.3%, i.e. **+1.6pp**
      from a 3.5× looser rule) and concluded the limit is the quote-asset
      restriction — which is [[0154]], which is gated on this. BE rank the pivot
      step above the materialised table they originally requested (now
      [[0150]], dropped to priority-low). See
      `0144/notes/S-be-0199-response-received.md`.
  - date: 2026-08-10
    status: active
    who: okarcz
    note: >
      Added option 5 to the implementation list - rate-driven discovery. Raised
      while explaining 0167: if usd_rate holds rates as first-class rows,
      enrichment can ask "which rates are new since I last ran?" instead of
      "which candles are missing a price?", and only the second requires reading
      every candle. That is option 3's insight using a queue we are already
      building, rather than a new pending-work table.
      Recorded WITH its three limits so it is not mistaken for a shortcut: a new
      rate does not name the candles needing it (and quote_asset_id is only the
      2nd sort-key column, so that lookup has no clean index path); it does
      nothing for the existing backlog of zeros; and it CANNOT be used to skip
      this task, because full rate coverage needs 0154's pivot tiers and 0154 is
      hard-blocked behind 0111. 0167 landed only the peg subset.
      Also flagged the misreading to avoid: this is NOT "compute close_usd from
      usd_rate at read time" - that is the refactor 0151 rejected, and it would
      not even be cheap, since the pivot rate is itself derived from candles so
      the scan moves rather than disappears.
      The options list predates usd_rate existing at all, which is why it needed
      amending rather than just re-reading.
  - date: 2026-08-20
    status: active
    who: okarcz
    note: >
      ⚠️ THIS TASK NOW BLOCKS SOMETHING. It has kept sliding on the argument that
      it blocks nothing. Measured on prod while diagnosing 0209: the peg-pivot
      tier hands off 657,234,896 candidates, drains ~9,800 per step and ROSE by
      2,682 in under an hour. Because pivot_sql is ORDER BY timestamp ASC, the
      backlog means recent candles are never reached — and USDT has no oracle
      fallback, so its entire quote leg has been unpriced since 2026-08-13. See
      0209 and 0212.
  - date: 2026-08-21
    status: active
    who: okarcz
    note: >
      RE-MEASURED on prod, and the task's target changes. The scan is worse than
      the July figure that framed this task — `price_ohlcv_1m` is now 736.46M
      rows / 18.44 GiB across 102 partitions and the enrichment INSERT reads
      739.68M per batch, i.e. the whole table, tracking its growth exactly
      (728.80M on 08-07 → 739.68M on 08-20). But the backlog is **XLM, not
      USDT**: 556.78M of the 656.69M candidates are XLM-quoted
      (`quote_asset_id = 4`), 85% of the total, and USDT does not appear in the
      top 15 legs at all. Every option in the list should be costed against that
      figure. Two structural facts that were not in the file before. **97.4% of
      the table is 0088 backfill history** (689.64M rows before 202403 against a
      17.05M live window), so the live pass is dragged through a 736M-row scan
      to serve a historical drain that should be a separate bounded sweep —
      which is a sharper argument for option 1 than the partition-pruning one
      already recorded. And **the XLM/USDC reference market starts 2021-02**, so
      the ~5.34M XLM-quoted candles from 2015-2020 can never be priced by
      `pivot_sql` and must be DECLARED as a bounded floor; otherwise AC 4 ("the
      backlog is drained") cannot terminate — the same trap
      [[close-usd-zero-as-missing-defect-class]] already records. The backlog
      concentrates in 2022 (131.57M) and 2023 (388.56M), both fully priceable.
      ⚠️ Also corrects [[0209]]: its 657M figure is the WHOLE-TABLE candidate
      count, not a USDT one, and its "pivot is behind a backlog" root cause is
      falsified — see 0215.
---

> **Why this is queued ahead of its own cost case:** the perf argument for 0111
> has always been "cost scales with table size, not era" — real but not urgent.
> The reason to do it now is [[0154]] behind it, and the reason 0154 matters is
> that **more than half of BE's pools have no headline TVL without it**.

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


## Re-measured 2026-08-21 — the target is XLM, and the scan is worse

Measured from `system.parts` and `system.query_log` (no scan of the hot table),
then two bounded `FINAL` scans. Supersedes the 2026-07-21 drained-state figures.

### The scan

| | 2026-07-21 (drained) | 2026-08-21 |
|---|---|---|
| table | 14.0M rows | **736.46M rows / 18.44 GiB / 102 partitions / 328 parts** |
| `INSERT … SELECT` | 0.6-0.7 s, ~11-14M rows | **26.4 s, 739.68M rows, 1.08-1.45 GiB peak** |
| `count_candidates` | 0.2-0.3 s | 6.7-9.0 s, 539M rows |
| `pivot_sql` (XLM) | — | **45.6 s, 688M rows, 1.22 GiB peak** |
| `peg_sql` | — | 14.2-15.3 s, 425M rows |

`read_rows` tracks total table size exactly (728.80M on 08-07 → 739.68M on
08-20, +10.9M in 13 days), which is the structural proof the July measurement
relied on: this is the query's input, not the cluster's mood. **Zero exceptions
in 14 days** — enrichment is not failing, it is simply never going to finish,
which is why this task keeps sliding.

⚠️ Peak memory is already 1.22-1.45 GiB. [[0154]] and option 5 both add a join
to this pass; there is less headroom here than the options list assumes.

### The backlog is XLM

| era | rows | unenriched |
|---|---|---|
| historical (0088, `< 202403`) | 689.64M | 646.25M (93.7%) |
| live window (`202607+`) | 17.05M | 10.44M (61.2%) |

**656.69M candidates total** — this is 0209's "657,234,896", now identified as a
whole-table figure rather than a USDT one.

Top unpriced quote legs (`close_usd = 0 AND volume_quote > 0`):

| quote | `quote_asset_id` | unpriced |
|---|---|---|
| **XLM** | **4** | **556.78M** |
| yXLM | 10 | 12.42M |
| yUSDC | 32 | 3.86M |
| SHX | 14 | 3.67M |
| XRP | 40 | 3.61M |

USDT (`111`) is not in the top 15. `FINAL` also collapses 736.46M raw rows to
706.69M, so ~29.8M superseded versions are re-read on every scan.

### ⚠️ A bounded floor that must be declared, or AC 4 cannot terminate

`pivot_sql` prices an XLM-quoted candle from the XLM/USDC market within
`pivot_window_s`. **That market's first candle is 2021-02** (11,648 that month,
rising to 44,328 by 202201). Everything older has no reference and is
permanently unpriceable by this design:

| year | XLM-quoted unpriced | |
|---|---|---|
| 2015-2020 | **5.34M** | ⛔ predates the reference — permanently unpriceable |
| 2021 | 189.91K | drained (reference exists from Feb) |
| 2022 | 131.57M | drain front is here |
| 2023 | **388.56M** | untouched |
| 2024 | 31.12M | untouched |
| 2026 | 2.81K | live window, keeping up |

The floor is small (<1%) and bounded, so the fear that the drain cannot finish
is **refuted** — but it must be named and excluded from AC 4, not left to make a
finished drain look unfinished. Note the mechanism: pre-2021 rows fail
`r.usd IS NOT NULL` and the window predicate, so they are skipped rather than
blocking; the oldest *matchable* candle is 2021-02, which is why 2021 is drained
and the front sits in 2022.

#### The drain rate — measured, and the scan is throttling it

`written_rows` from `system.query_log`, 7 days to 2026-08-21:

| stmt | runs/day | rows written/day | per run |
|---|---|---|---|
| **XLM pivot** | 66-72 | **660-720K** | **exactly 10,000 — `batch_size`, every batch** |
| peg | 66-72 | 82-89K | ~1,230 |
| oracle | 177-196 | 105-213K | ~550-1,100 |

The XLM pivot writes **exactly `batch_size` on every single run** (17 runs →
170K, 67 → 670K, 72 → 720K — perfectly linear). It is `LIMIT`-bound every time
and has never once exhausted its candidates. Peg and oracle sit far below the
same limit, so they are not limit-bound and are essentially caught up.
**The XLM pivot is the sole bottleneck.**

> **556.78M ÷ 720K/day ≈ 774 days (~2.1 years)**, before counting growth (0209
> measured the candidate set rising 2,682 in under an hour, ~64K/day, which
> pushes it past 850).

⚠️ **The scan is not merely slow — it throttles the drain to 15% of the rate the
deployed config already asks for.** The pass runs hourly at `max_batches = 20`,
so 480 batches/day are budgeted; it achieves **72**. Each batch costs ~90 s of
statements (45.6 s pivot + 26.4 s oracle + 14.4 s peg, plus count scans) against
a 300 s timeout, so ~3 batches land per invocation instead of 20. Bounding the
scan so a batch costs ~1 s (the drained-state figure) lets the **unchanged**
Lambda reach its configured 20 → ~4.8M/day → **~116 days**. That is the payoff
to cost option 1 against, and it needs no config change.

⚠️ Corrects [[0209]]: `pivot_written = 0` is false for XLM, which writes ~700K
rows/day. It holds only for USDT — see [[0215]].

## What this does to the options

**Option 1 wins, for a better reason than the one recorded above.** The live
window is 17.05M rows and keeps up fine (2.81K unpriced). It is being dragged
through a 736M-row scan purely to serve a historical drain. Splitting the live
pass from a separate bounded historical sweep is the whole fix; partition
pruning is the mechanism, not the argument.

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
5. **Rate-driven discovery — a variant of 3 that did not exist when this list was
   written.** Added 2026-08-10, after [[0167]] built `prices.usd_rate`.

   The defect is not that the arithmetic is expensive — it is that *discovering
   what to work on* costs a full scan. Enrichment asks **"which candles are
   missing a price?"**, which is only answerable by reading every candle
   (`FINAL WHERE volume_quote_usd = 0`). With rates stored as first-class rows it
   can instead ask **"which rates are new since I last ran?"** — a small query
   against a small table — and then resolve the bounded set of candles quoted in
   those assets over that window.

   This is option 3's insight with a queue we are already building for other
   reasons, rather than a new pending-work table written at ingest. The two are
   not exclusive: a rate watermark covers *"a new/corrected rate arrived"*, an
   ingest queue covers *"a new candle arrived"*, and enrichment needs both edges.

   ⚠️ **Three things that stop this being a free win — check them before costing
   it:**

   - **A new rate does not name the candles that need it.** You still resolve
     "candles quoted in asset X within window W". Bounded and cheap versus a full
     scan, but not free, and `quote_asset_id` is only the **second** sort-key
     column on `price_ohlcv_1m`, so that lookup has no clean index path — the
     same projection-cost unknown [[0151]] flagged and [[0167]] deliberately did
     not pull forward.
   - **It does nothing for the existing backlog.** A queue helps from the moment
     you start queueing; the accumulated zeros still need one bounded sweep,
     which is option 1.
   - ⚠️ **Sequencing trap — this cannot be used to skip this task.** Driving
     enrichment entirely from `usd_rate` needs rates for *all* quote assets, which
     is [[0154]]'s pivot tiers, and **0154 is hard-blocked behind THIS task** on
     cost. [[0167]] has landed only the peg subset (USDC/USDT, `method='oracle'`).
     So this is a fix to sequence *with* 0111, not an alternative to it.

   ⚠️ **Do NOT read this as "compute `close_usd` from the rate table at read
   time".** That is the schema-wide refactor [[0151]] rejected, and it would not
   even be cheap: the pivot rate is itself derived from candles (the XLM/USDC
   vwap), so the scan moves rather than disappears, and the cost lands on every
   read of a surface BE already calls slow. `close_usd` stays stored,
   non-nullable, written in place.

Options 1 and 3 are complementary; 2 may be a fast stopgap; 5 is 3's cheaper
half if [[0167]]'s table proves out.

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
