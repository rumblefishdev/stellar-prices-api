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
  - date: 2026-08-21
    status: active
    who: okarcz
    note: >
      OPTION 1 CHOSEN and the implementation plan written, after [[0215]]'s Caddy
      fix cleared the sequencing gate and gave a clean baseline. Key finding that
      shrinks the work: the partition-bounding mechanism ALREADY EXISTS, is
      tested, and runs in production — ChEnrichConfig::time_window threaded by
      window_pred(), set per month by the 0114 coarse-repair driver, with the
      candidate-side-only bound already designed so the pivot reference still
      forward-fills. The scheduled 1m pass simply sets it to None, and main.rs:58
      states that as intent. So this is "drive the existing bounding from the
      scheduled pass", not "build bounding". Design is a live pass bounded to 2
      partitions plus a frontier-driven historical sweep, with the frontier
      ADVISORY (every month re-confirmed by a cheap bounded count) so wrong state
      can never silently skip rows — the failure class 0215 just cost 26 days on.
      The pre-2021 unpriceable floor then needs no hard-coded cutoff: those
      months make no progress and mark themselves exhausted. ⚠️ One statement
      still unmeasured — watermark()'s `SELECT max(timestamp)` runs every pass and
      every measurement so far filtered query_log on INSERT INTO, so it has never
      been looked at. Branch perf/0111_partition-bounded-enrichment-passes;
      prepare-only, nothing deployed.
  - date: 2026-08-21
    status: active
    who: okarcz
    note: >
      Built and merged (PR #242, 7eff9c3) but NOT deployed. Pre-deploy
      re-measurement falsified this file's own "the live window keeps up fine
      (2.81K unpriced)": that figure is XLM-quoted only, conflated with the
      all-quotes era figure of 10.44M of 17.05M (61.2%). Measured over 48h in
      4h buckets, the unpriced share is FLAT at 50-58% with no age-decay — a
      bucket 44h old looks like one 4h old, so enrichment never returns to
      those rows. ~45% of the 26,900 rolling-4h total is the permanent
      exotic-quote floor (long tail, no leg over 17%), but ~2,370/4h are
      USDC(3)- and XLM(4)-quoted, i.e. should-be-priceable — ~14,200/day. Also
      established that EnrichmentBacklogAlarm reads OK on NO DATA
      (treatMissingData NOT_BREACHING) because pass_metrics only publishes
      after pass.run() returns and the Lambda is killed mid-pass — the 0204
      pattern. Net effect: the chosen design is unchanged, Phase 1's payoff is
      larger than recorded (it un-starves the live window, not just speeds it),
      and AC 6's "floor" is ~27K not 2.81K. Deploy deliberately deferred ~2
      days (costs ~$0.29, 3.4% of the live backlog, 1.1% of the historical one)
      pending an explanation for the unexplained RollupFreshnessProbeFunction
      code-asset replacement in the cdk diff.
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

⚠️ **The scan is not merely slow — it makes the pass FAIL, every single time.**
The pass runs hourly at `max_batches = 20`, so 480 batches/day are budgeted; it
achieves **72**. The reason is not the 300 s timeout (there is not one
`Task timed out` in 48 h): **the XLM pivot errors with
`Clickhouse(BadResponse(""))` on every invocation**, and `?` aborts the pass.
The 72/day is 3 invocation attempts/hour — one EventBridge trigger plus two
Lambda async retries — each dying after a single peg + XLM pivot. See [[0215]],
which carries the CloudWatch timeline.

The XLM pivot is the **only** statement over ~30 s (45.6 s, against peg 14.4 s
and oracle 26.4 s, both of which survive), so a duration-linked read/idle
timeout would explain why exactly this one dies. If that holds, bounding the
scan so a batch costs ~1 s (the drained-state figure) does more than speed the
drain up — it stops the pass failing at all, lets the **unchanged** Lambda reach
its configured 20 batches → ~4.8M/day → **~116 days**, and lets the USDT pivot
run for the first time. No config change needed.

🔴 **Nothing about this was visible from the data.** ClickHouse completes the
abandoned statement server-side and logs `QueryFinish`, so the 10,000 rows land
and `written_rows`, `query_log` and the rollup alarms all read normal while the
pass has not completed successfully in at least two days. The enrichment errors
alarm that should have caught it is latched — [[0214]].

⚠️ Corrects [[0209]]: `pivot_written = 0` is false for XLM, which writes ~700K
rows/day. It holds only for USDT — see [[0215]].

## ✅ Re-measured 2026-08-21 **after** the 0215 Caddy fix — the pass now completes its statements and the accounting closes exactly

Triggered by `prices-production-enrichment-duration-near-timeout` firing at
16:18 UTC on two consecutive datapoints of **300,000 ms and 300,338 ms** — i.e.
not "near" the timeout, *at* it. This is the flip side of [[0215]]: with Caddy's
30 s cut removed the 44.9 s XLM pivot survives, so the pass proceeds through
batches instead of aborting, and runs the Lambda clock out instead.

`system.query_log`, 2026-08-21 14:00-16:30 UTC, enrichment INSERTs split by tier:

| tier | runs | errors | avg | total | rows read | written |
|---|---|---|---|---|---|---|
| pivot XLM | 18 | **0** | 44.93 s | 808.7 s | 685.01 M | 180,000 |
| oracle | 22 | **0** | 26.21 s | 576.6 s | 735.78 M | 17,615 |
| peg | 18 | **0** | 14.50 s | 261.0 s | 420.93 M | 22,248 |
| pivot USDT | 10 | **0** | 13.61 s | 136.1 s | 425.86 M | 2,051 |
| `count_candidates` | 40 | 0 | 9.27 s | 371.0 s | 735.15 M | — |

**Per-invocation cost decomposes exactly:** an oracle batch is 26.21 + 9.27 =
35.5 s, a peg-pivot batch is 14.5 + 44.93 + 13.61 + 9.27 = 82.3 s. Three oracle
batches (106 s) plus 2.4 peg-pivot batches (198 s) = **304 s**. Seven and a half
invocations × 5.4 batches = **the 40 counts observed**. Nothing is unexplained.

> **It budgets `max_batches` = 20 per tier — 40 batches — and achieves 5.4.**

### Four things this changes

- ✅ **Zero errors on all four tiers.** [[0215]]'s fix is holding; the 44.9 s
  pivot no longer dies at 30.0 s.
- ✅ **The USDT pivot is no longer dark** — 2,051 rows written over 10 runs, and
  *not* limit-bound. Corrects [[0209]] and the standing "pivot has never priced
  a 1m row" note; that was true when measured and is now false.
- ⚠️ **`count_remaining_at_volume_zero` does not appear in `query_log` at all.**
  It runs once per pass, at the end. Its absence is the proof that **no pass
  completed** in 2.5 hours — so the spec §5 metrics are not being published
  either, which is its own blind spot on top of [[0214]].
- 🔴 **The oracle tier is 27% of all enrichment time** — 576.6 s reading 735.78 M
  rows per statement to write ~800 rows a batch that all live in the last few
  hours. Phase 1 bounds this too.

### ⛔ REFUTED: `watermark()` is not a hidden third scan

The plan's "one unmeasured statement to check first" is measured and the
suspicion is **wrong**. `SELECT toUnixTimestamp(max(timestamp))` reads **294
rows in 0.00 s** (0.02-0.05 s over 24 h). `timestamp` is not a sort-key prefix,
but `PARTITION BY toYYYYMM(timestamp)` gives every part a `minmax_timestamp`
index and ClickHouse answers the aggregate from part metadata without touching
data. A window predicate here buys nothing; do not add one. Recorded in the
`watermark()` doc comment so this is not re-derived a third time.

### The drain rate, re-based

180,000 XLM rows per 2.5 h = **1.73 M/day**, up from the 660-720 K/day measured
pre-fix — again, because the pass now gets further before dying. Against
556.78 M that is **~322 days**, so the conclusion is unchanged: it does not
finish. Phase 1 alone sends this to **zero** until the Phase 2 sweep lands,
which is why the two should land together.

## ⚠️ Re-measured 2026-08-21 evening — the live window does NOT "keep up fine"

Prompted by a pre-deploy question ("is it safe to leave this undeployed for two
days?"). The answer is yes, but reaching it falsified a claim this file makes
twice.

### The conflation

This file states the live window has **2.81 K** unpriced and "keeps up fine".
That figure is **XLM-quoted only** — it comes from the by-year table under *The
backlog is XLM*, whose scope is `quote_asset_id = 4`. The all-quotes figure sits
in the era table one section earlier: **10.44 M of 17.05 M (61.2%)**. The two
were read as though they shared a scope. They do not, and the era figure is the
one that matches the live table.

### The measurement

`price_ohlcv_1m FINAL`, 48 h to 2026-08-21 ~20:45 UTC, `timestamp`-bounded so it
prunes to the current partition — sub-second, no full scan:

| bucket (UTC) | candidates | unpriced | share |
|---|---|---|---|
| 08-19 20:00 | 60,855 | 35,521 | **58.4%** |
| 08-20 00:00 | 49,629 | 25,237 | 50.9% |
| 08-20 04:00 | 28,911 | 13,692 | 47.4% |
| 08-20 08:00 | 87,843 | 44,047 | 50.1% |
| 08-20 12:00 | 71,176 | 41,785 | 58.7% |
| 08-20 16:00 | 101,096 | 58,455 | 57.8% |
| 08-20 20:00 | 41,703 | 23,472 | 56.3% |
| 08-21 00:00 | 48,588 | 24,360 | 50.1% |
| 08-21 04:00 | 39,853 | 19,694 | 49.4% |
| 08-21 08:00 | 73,175 | 39,566 | 54.1% |
| 08-21 12:00 | 39,311 | 20,134 | 51.2% |
| 08-21 16:00 | 47,675 | 26,994 | 56.6% |
| 08-21 20:00 | 9,010 | 6,882 | _(partial bucket)_ |

**There is no decay.** A bucket 44 h old — 44 hourly passes' worth of chances —
carries the same unpriced share as one 4 h old, and is in fact the highest of the
complete buckets. Enrichment does not come back for these rows. Age-decay is the
discriminator: lag drains, a floor does not.

### Which legs, rolling 4 h (26,900 total)

| `quote_asset_id` | unpriced |
|---|---|
| 10 (yXLM) | 4,518 |
| **4 (XLM)** | **1,632** |
| 32 (yUSDC) | 1,631 |
| 40 (XRP) | 1,506 |
| 28 | 1,372 |
| 11 | 1,260 |
| 1221 | 829 |
| **3 (USDC)** | **738** |
| 201223 | 723 |
| 14 (SHX) | 673 |

Top 10 = 14,882, so **~45% sits in a tail below rank 10** and no single leg
exceeds 17%. That long tail is the permanent **exotic-quote floor** (quote ∉
{USDC, USDT, XLM}, no oracle) that [[0056]] redesigned the stall alarm to
exclude. It never drains by design, and it is what makes the aggregate decay
above look flat — it masks whatever the priceable legs do. Most of the 26,900 is
expected and is not this task's to fix.

⚠️ **But two legs are not floor.** `3` (USDC) and `4` (XLM) are the pivot
reference pair itself (`asset_id = 4 AND quote_asset_id = 3`). USDC-quoted
candles are peg-tier arithmetic; XLM-quoted ones are precisely what `pivot_sql`
exists to price. Together ~2,370 per 4 h ≈ **14,200/day of should-be-priceable
rows**. Note 1,632 XLM-quoted unpriced in *four hours* against this file's
"2,810 for all of 2026" — those two cannot both be true.

⏳ **Not yet discriminated** — the one query not run: whether legs 3 and 4 decay
with age once the floor is stripped out. Flat across 44 h ⇒ priceable live rows
are being starved by the 300 s kill. Decaying ⇒ ordinary lag, and only the floor
remains. The identities of 10/28/11/1221/201223 are also unconfirmed — read off
this file rather than `prices.assets`, and [[0139]] warns against trusting an
`asset_id`.

### 🔴 The stall alarm reads OK because nothing is measured

`pass_metrics` publishes only *after* `pass.run()` returns (`main.rs:275`), and
the Lambda is killed mid-pass — so `EnrichmentRowsEnriched` and
`EnrichmentRowsRemainingRecent` have **no datapoints at all**.
`EnrichmentBacklogAlarm` is `treatMissingData: NOT_BREACHING`
(`observability-stack.ts:381`), so no data reads as **OK**. Same pattern as the
seven alarms in [[0204]]. This sharpens the §5-metrics blind spot noted above
from "the metrics are missing" to "the alarm that should catch this is green
*because* they are missing".

Interim manual check while undeployed — baseline **~27 K**, investigate above
~50 K:

```sql
SELECT count() AS recent_unpriced
FROM prices.price_ohlcv_1m FINAL
WHERE volume_quote_usd = 0 AND volume_quote > 0
  AND timestamp >= now() - INTERVAL 4 HOUR
```

### What this changes for the fix

**Phase 1's payoff is larger than this file claims.** It is framed purely as
"stop dragging the live window through a 736 M-row scan". But post-fix the live
pass reaches its full `max_batches` × `batch_size` = 200 K rows/pass ≈
**4.8 M/day**, against a live inflow of ~180 K/day unpriced. That clears the
inflow with headroom *and* eats into the 10.44 M live-window backlog. The live
window is not merely being carried inefficiently — it is being **starved**, and
Phase 1 fixes that too.

Nothing here changes the chosen design or the phase ordering. It raises the
payoff and adds a second thing to verify after deploy.

### Deploy timing

Two days undeployed costs ~$0.29 of Lambda, ~360 K live rows (**3.4%** of the
10.44 M live backlog) and ~6 M historical rows (**1.1%** of 556.78 M). Judged
acceptable against the unexplained `RollupFreshnessProbeFunction` code-asset
replacement in the `cdk diff`: deploying an unexplained artifact into the stack
that also carries `CleanupRule` ([[0200]]) is the larger risk.

## What this does to the options

**Option 1 wins, for a better reason than the one recorded above.** The live
window is 17.05M rows and keeps up fine (2.81K unpriced). It is being dragged
through a 736M-row scan purely to serve a historical drain. Splitting the live
pass from a separate bounded historical sweep is the whole fix; partition
pruning is the mechanism, not the argument.

⚠️ **"keeps up fine (2.81K unpriced)" is FALSE** — corrected in the 2026-08-21
evening re-measurement above. 2.81K is the XLM-only figure; all quote legs give
**10.44M of 17.05M (61.2%)**, with no age-decay over 48 h. The conclusion here
survives and strengthens: the live window is not merely carried inefficiently,
it is starved.

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

## ✅ CHOSEN: option 1, via option A — implementation plan (2026-08-21)

### The mechanism already exists and is in production

`ChEnrichConfig::time_window: Option<(u32, u32)>` (`ch_enrich.rs:298`) is
threaded into every candidate scan by `window_pred()` (`ch_enrich.rs:504`) and is
covered by tests (`peg_sql_threads_the_partition_window_into_the_outer_where`,
`pivot_sql_bounds_only_the_candidate_side_not_the_reference`). `repair.rs:317`
sets it per month for the coarse tables. Its own doc comment already names this
task as the reason it exists.

The scheduled 1m pass simply never sets it — `main.rs:58`:

```rust
// The scheduled Lambda always runs the unbounded hourly pass over
// price_ohlcv_1m; the partition-bounded window is only set by the 0114
// coarse-repair driver (operator-run), never here.
time_window: None,
```

**That comment is the defect, written down as intent.** So this is not "build
partition bounding" — it is "drive the existing, tested partition bounding from
the scheduled pass", which is also what the reuse-tested-code working agreement
requires.

⚠️ Critically, `time_window` bounds **only the candidate side**. The pivot tier's
inline XLM/USDC reference still forward-fills from earlier months, so a month's
first buckets keep a valid anchor. That correctness property is already designed
and tested — do not re-derive it.

### The shape: split the two jobs that are currently welded together

One unbounded pass serves two populations with opposite needs:

| | rows | unpriced | needs |
|---|---|---|---|
| live window (`202607+`) | 17.05 M | ~~2.81 K~~ **10.44 M** | **freshness**, ⚠️ **starved** |
| historical (`< 202403`) | 689.64 M | 646.25 M | **throughput** |

<sub>*The 2.81 K was XLM-quoted only and is corrected above (2026-08-21 evening).
The live window is starved, not keeping up — which does not change the split,
only how much Phase 1 is worth.*</sub>

The live window is dragged through a 736 M-row scan purely to serve the drain.
Splitting them is the whole fix.

### Phase 0 — schema: `prices.enrichment_frontier`

Fourth instance of an established pattern in this schema — `ingest_cursor`
(0064), `backfill_progress`, `discovery_state` are all tiny
`ReplacingMergeTree` state tables in `prices`, live on prod and written by our
workers.

```
tbl        LowCardinality(String)   -- price_ohlcv_1m, and the coarse tiers later
month      UInt32                   -- YYYYMM, == the partition id under toYYYYMM
state      Enum8('pending','exhausted')
zeros_seen UInt64                   -- at last sweep, informational
swept_at   DateTime
version    UInt64                   -- monotonic; RMT(version) keeps the highest
ENGINE = ReplacingMergeTree(version) ORDER BY (tbl, month)
```

- **Size:** ~102 partitions × 6 tiers ≈ 600 rows, tens of bytes each. Under
  100 KB permanently, on a disk where we are 3.3% of a volume BE owns.
- **Created by hand once**, like every other table here — there is no migration
  runner, prod schema is applied manually and [[0142]] verified prod currently
  matches `init.sql`. The worker needs no DDL grant.
- **No MV, not in the rollup chain, not in the cleanup worker's retention list.**
  It touches none of the machinery that has broken before.

### Phase 1 — bound the live pass (`main.rs`)

Set `time_window` to the current **and previous** monthly partition on the
scheduled pass. Two partitions, not 102. Env-overridable
(`ENRICH_LIVE_PARTITIONS`, default 2) so it can be widened without a code change
if ingest ever lands late rows.

This alone is most of the win, and it bounds the **oracle tier** too — which
today burns 105-108 s of every 300 s invocation draining 499-3,564 candidates out
of a 737 M-row scan (measured [[0215]]).

### Phase 2 — the historical sweep, frontier-driven

A driver in the shape of `CoarseRepairDriver` (`repair.rs`), which already has
every piece: per-month `time_window`, `one_shot: false` for bounded batches, and
a `deadline: Option<Instant>` so a slow catch-up cannot run past the Lambda
timeout.

Per invocation, after the live pass:

1. Read the frontier; pick the oldest `pending` month for this table.
2. **Confirm with a partition-bounded `count_candidates`** (~0.2-0.3 s).
3. Run `ChEnrichmentPass` with that month's `time_window`, bounded batches,
   deadline-aware.
4. Write the frontier: no progress → `exhausted`; otherwise update `zeros_seen`.

🔴 **The frontier is ADVISORY, never authoritative.** Step 2 is what makes it so.
A wrong frontier then costs one cheap query, never skipped rows — and skipped
rows that look healthy are precisely the failure class that cost 26 days in
[[0215]]. Add a slow-cadence (daily) full re-enumeration to correct drift, so the
state can only ever be a performance hint.

⚠️ **Concurrency.** Three attempts run per hour via async retry. `version` is
monotonic-forward (`ingest_cursor`'s deliberate design, including that a rewind
then needs an explicit `DELETE`), and because the frontier is advisory a race
costs duplicated work, not lost work.

### Phase 3 — the unpriceable floor falls out for free

5.34 M rows predate the XLM/USDC reference market (first candle 2021-02) and can
never be priced by this design. They make no progress, so step 4 marks those
months `exhausted` on first visit and the sweep never revisits them. **No
hard-coded cutoff constant** — which is the main correctness advantage over
bounded re-enumeration. Record the expected figure so a finished drain is not
misread as an unfinished one (AC 4 below).

### Phase 4 — make the drain observable

- Publish frontier position and `pending` month count, so drain progress is a
  metric rather than an archaeology dig.
- A **per-leg** no-progress signal. Today one leg's progress masks another's
  stall — see [[0219]], where the peg leg writes a constant 1,236 rows/batch
  while `run_peg_pivot_tier`'s guard only breaks when the *overall* count stops
  falling, which the XLM pivot's 10,000/batch prevents.

### Phase 5 — tests

Window threading is already covered. Add: frontier advance and exhaust; a
no-progress month marked exhausted; monotonic version under concurrent writes;
the live pass pruning to exactly the configured partition count.

### ⚠️ One unmeasured statement to check first

`watermark()` (`ch_enrich.rs:521`) runs `SELECT max(timestamp)` on every pass.
`timestamp` is **4th** in the sort key, not a prefix. Every measurement in this
task and in [[0215]] filtered `query_log` on `INSERT INTO`, so this SELECT has
never been looked at. Measure it before assuming the live pass is cheap — it may
be a third full scan hiding in plain sight.

### Sequencing note

Bounding the 1m pass is also what unblocks [[0218]]: the coarse sweep sits behind
`main.rs:167`'s `?` with `budget = min(120 s, deadline − now − 60 s)`, and a pass
that runs to the hard deadline leaves that at 0. It cannot do useful work until
this lands.

**Deploy is out of scope for this branch** — prepare only, per the standing rule.

## Acceptance Criteria

> **Restated 2026-08-21.** ACs 1 and 4 were written when the backlog was
> believed to be **8.4M rows**. It is **656.69M**, of which **5.34M are
> permanently unpriceable**. As originally worded neither could ever be
> satisfied, so the task could never close — which is part of why it kept
> sliding. The originals are preserved below each replacement.

- [ ] **A bounded pass runs to completion inside the Lambda budget**, measured
      in `system.query_log` — not inferred from a drained-state query.

      The signal is exact and cannot be faked by a partial pass:
      `count_remaining_at_volume_zero` (`SELECT count() AS total, countIf(...)`)
      is issued **once per pass, at the very end**. On 2026-08-21 it appears
      **zero times in 2.5 hours** — no pass completes at all, which is also why
      the spec §5 metrics are not being published. Target: ~3/hour.

      <sub>*Was: "a full `one_shot` drain over the current backlog completes
      inside the Lambda budget." 556.78M XLM candidates will not drain in 300 s
      under any scan strategy. The chosen design drains over months by
      construction, so this demanded something the design never promised.*</sub>

- [ ] **Per-batch rows-read is bounded and does not scale with total table
      size**, verified in `system.query_log`, not by wall-clock on a quiet
      cluster. Baseline to beat (2026-08-21): oracle **26.21 s / 735.78M**, XLM
      pivot **44.93 s / 685.01M**, peg **14.50 s / 420.93M**,
      `count_candidates` **9.27 s / 735.15M**. Target ≈ **17M** on every tier.

- [ ] **Re-measured while the cluster is actively writing** — a quiet cluster is
      how this was missed. The drained-state numbers (0.3 s) are 80× faster than
      the same query under load. A measurement taken against an idle cluster
      does not satisfy this AC no matter how good it looks.

- [ ] **The drain demonstrably walks, and its terminal state is declared.**
      `prices.enrichment_frontier` advances oldest-first across invocations, and
      months predating the XLM/USDC reference market reach `exhausted`.

      🔴 **A finished drain leaves a NON-ZERO residual of ~5.34M rows.** They
      predate the reference market (its first candle is 2021-02) and are
      permanently unpriceable by this design. That figure is recorded here so a
      *finished* drain is not misread as an unfinished one — the specific
      mistake this AC now exists to prevent.

      <sub>*Was: "the 8.4M rows currently sitting un-enriched are drained, and
      the count is recorded before/after." Wrong figure by ~78×, and it implied
      a zero residual that the design cannot reach.*</sub>

- [ ] **0026's `EnrichmentPassDurationMs` stays well clear of 300 s for a week**
      spanning active backfill, and
      `prices-production-enrichment-duration-near-timeout` returns to OK and
      stays there. That alarm firing at **300,000 / 300,338 ms** on 2026-08-21
      is what re-opened this task.

- [ ] **The live window is not starved by the historical sweep.**
      `EnrichmentRowsRemainingRecent` stays at its floor. The sweep is
      best-effort and time-budgeted specifically so it can never regress the
      live pass; this is the check that the budgeting works.

      🔴 **"its floor" is not 2.81 K.** Measured 2026-08-21 evening: **~27 K per
      rolling 4 h**, of which roughly half is the permanent exotic-quote floor
      and ~2,370 are should-be-priceable USDC/XLM legs. Judge this AC against
      ~27 K falling toward the exotic-only residual (~12-15 K), never against
      zero. And the metric itself is currently **unpublished** — the pass is
      killed before `pass_metrics` runs — so first prove it has datapoints at
      all, then judge its value.

      ⚠️ Note `EnrichmentRowsRemainingAtVolumeZero` changes meaning under a
      bounded pass — it becomes window-scoped and will drop from ~656M to the
      live window's count. No alarm consumes it (the stall alarm watches
      `Recent`), but on a dashboard it will look like a dramatic fix that has
      not happened.

**Baseline to judge the fix against.** Two are on record and they disagree,
which is the point:

| | measured | per batch | note |
|---|---|---|---|
| drained state | 2026-07-21 | ~4,500 ms/pass | **quiet cluster — do not judge against this** |
| loaded, pre-0215-fix | 2026-07-21 | ~35 s | pass aborted at 30 s |
| loaded, post-0215-fix | 2026-08-21 | **35.5 s oracle / 82.3 s peg-pivot** | current; pass runs to the 300 s wall |

A fix is demonstrated when the **loaded** figure stays low, never when the quiet
one does.

## Out of scope

- The alarm blind spot that let this run four days unnoticed — that is [[0112]].
- The `price_ohlcv_15m` pre-roll INSERT (217 s avg / 301 s max, 177M rows) —
  **now [[0113]]**. Note the description above was wrong when first written: it
  called this "the same 300 s wall", implying a Lambda timeout. The pre-roll is
  operator-run SQL, not a Lambda, so no timeout applies; the real exposure is
  memory (2,938 MB of a 5.59 GiB quota). 0113 carries the corrected framing.
