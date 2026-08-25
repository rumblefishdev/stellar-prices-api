---
id: "0111"
title: "Enrichment re-scans the whole table every batch — 545M rows/batch, caused a 4-day production outage"
type: PERF
status: completed
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
  - date: 2026-08-24
    status: completed
    who: okarcz
    note: >
      DEPLOYED 2026-08-24 08:03:24 UTC and verified the same morning. 5 of 6 ACs
      pass on measured production data; AC 5 (the week-long duration soak to
      ~2026-08-31) is DEFERRED to 0220, which is active and checked daily.
      AC 2: oracle 26.21s/735.78M -> 0.62s/13.00M, pivot XLM 44.93s/685.01M ->
      0.52s/10.53M, count_candidates 9.27s/735.15M -> 0.08s/4.27M, all under the
      ~17M target. AC 3: measured while the cluster wrote 226,254,819 rows in 30
      minutes, so this is not the quiet-cluster illusion that hid the defect
      twice. AC 1: one completed pass per hour where there were zero in 2.5h;
      Invocations/Errors went 3/3 to 1/0. AC 4: frontier walks oldest-first,
      201511/201512/201601 all exhausted, no hard-coded cutoff. AC 6: legs 3 and
      4 at 12 and 17 over 40h. The alarm returned to OK at 08:31.
      Blocker resolved: the unexplained RollupFreshnessProbeFunction diff was
      cargo FEATURE UNIFICATION, not a stale prod build - probe built alone is
      byte-identical to prod, built with the other nine crates it is +433 KB.
      Side effects: 0130 cleared (coarse sweep now sweeps all 6 tables, 0 failed)
      and 0218 AC 1 met. Peak memory 1.08-1.45 GiB -> 517-581 MiB, which
      obsoletes this file's headroom warning for 0154.
  - date: 2026-08-24
    status: active
    who: okarcz
    note: >
      Ran the leg-level discriminator the 2026-08-21 entry called for and it
      REFUTED that entry's starvation conclusion. Legs 3 (USDC) and 4 (XLM) —
      identities now confirmed against prices.assets — decay to zero within ~4h:
      over a 40h window excluding the freshest 8h, XLM is 12 unpriced of 138,722
      and USDC is 0 of 90,672. Every other quote leg is unpriced at EXACTLY
      100%, with no middle ground, which is the signature of a structural gap
      (no oracle, no pivot reference) rather than a backlog — the permanent
      exotic-quote floor of 0056, ~88K rows/day against ~138K/day priceable.
      USDT (111) is absent from the top 20, so live USDT volume is negligible
      and the peg tier has nothing to do in the live window. yXLM (44,432) and
      yUSDC (21,996) are 45% of the floor and belong to 0154, not here. Both
      measurements are correct: ~42 of the 08-21 window's 48h predate the 0215
      Caddy fix, when the pass died at 30.0s and the live window really was
      starved, and the aggregates reconcile (51.2% on 08-22 20:00, inside the
      08-21 50-58% band) because the floor is half the candidates and masks a
      priceable half that goes to zero. Net: the starvation was real and 0215
      already fixed it — it was never 0111's scan cost. Phase 1's payoff
      reverts to the original case (the 300,000ms alarm, 5.4 of 40 budgeted
      batches, the ~322-day drain, 0218 blocked behind it). AC 6 rewritten to be
      leg-scoped, because EnrichmentRowsRemainingRecent is ~100% floor and would
      have passed a starved pass. Still not deployed; the cdk diff blocker
      stands.
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

> 🗄️ **Superseded 2026-08-24 as a verdict, kept as a record.** Everything
> measured here is accurate *for the regime it was taken in* — ~42 of its 48 h
> predate the [[0215]] Caddy fix, when the pass died at 30.0 s on every
> invocation. Its **starvation conclusion was falsified** by the leg-level
> discriminator it itself called for; see *Re-measured 2026-08-24* below. Read
> this section for the pre-fix baseline, not for what is true now.

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

✅ **Discriminated 2026-08-24 — legs 3 and 4 DECAY, to zero, within ~4 h.** The
question posed here ("flat ⇒ starved, decaying ⇒ ordinary lag") was the right
one and it returned *decaying*. The starvation reading below is therefore
**refuted for the post-0215 regime**; it was real when measured and [[0215]]
fixed it. Leg identities are also now confirmed against `prices.assets`
(`3` = USDC, `4` = XLM — the cohorts were right). See the next section.

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

> ⛔ **This subsection's conclusion is WITHDRAWN — see *Re-measured 2026-08-24*.**
> It argued Phase 1's payoff is larger than the file claims because Phase 1 also
> un-starves the live window. The live window is not starved post-0215, so the
> payoff reverts to the original, smaller case. Preserved because the reasoning
> is sound *given* a starved live window, and a future regression would make it
> live again.

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

## ✅ Re-measured 2026-08-24 — the live window is NOT starved; every leg is 0% or 100%

Runs the discriminator the previous section called for and could not run: **do
legs 3 and 4 decay with age once the exotic floor is stripped out?** They do —
to zero, within about four hours. The starvation reading is **refuted for the
post-0215 regime**.

Both queries are `timestamp`-bounded so they prune to the current partition —
sub-second, no full scan. Predicate is enrichment's real
`CANDIDATE_PRED` (`ch_enrich.rs:209`),
`(volume_quote_usd = 0 OR close_usd = 0) AND volume_quote > 0`, which is
**broader** than the narrower `volume_quote_usd = 0` the 08-21 tables used — so
these counts read slightly higher by construction. That is a predicate
difference, not a regression.

### Decay by cohort, 4 h buckets, 48 h to 2026-08-24 ~08:00 UTC

| bucket (UTC) | USDC cand | unpriced | % | XLM cand | unpriced | % | floor cand | unpriced | % |
|---|---|---|---|---|---|---|---|---|---|
| 08-22 04:00 | 3,993 | 0 | **0** | 5,128 | 0 | **0** | 10,403 | 10,361 | 99.6 |
| 08-22 08:00 | 13,480 | 0 | **0** | 19,879 | 0 | **0** | 46,103 | 45,957 | 99.7 |
| 08-22 12:00 | 11,566 | 0 | **0** | 16,665 | 0 | **0** | 32,983 | 32,902 | 99.8 |
| 08-22 16:00 | 8,592 | 0 | **0** | 15,025 | 2 | **0** | 35,328 | 35,261 | 99.8 |
| 08-22 20:00 | 6,590 | 0 | **0** | 9,592 | 3 | **0** | 17,086 | 17,035 | 99.7 |
| 08-23 00:00 | 9,146 | 0 | **0** | 14,048 | 1 | **0** | 22,029 | 21,955 | 99.7 |
| 08-23 04:00 | 8,853 | 0 | **0** | 13,667 | 5 | **0** | 25,885 | 25,803 | 99.7 |
| 08-23 08:00 | 9,412 | 0 | **0** | 13,376 | 1 | **0** | 19,826 | 19,752 | 99.6 |
| 08-23 12:00 | 9,735 | 0 | **0** | 14,924 | 0 | **0** | 27,030 | 26,948 | 99.7 |
| 08-23 16:00 | 3,976 | 0 | **0** | 7,896 | 0 | **0** | 16,054 | 16,048 | 100 |
| 08-23 20:00 | 6,055 | 4 | 0.1 | 10,136 | 4 | **0** | 20,645 | 20,616 | 99.9 |
| 08-24 00:00 | 8,808 | 8 | 0.1 | 12,465 | 1 | **0** | 23,828 | 23,766 | 99.7 |
| 08-24 04:00 | 4,819 | 1,323 | _27.5_ | 8,468 | 1,737 | _20.5_ | 15,043 | 15,030 | 99.9 |

The newest bucket is **lag, not floor** — it is mid-drain and always will be.
Everything older than it is at zero.

### Per leg, 40 h window with the newest 8 h excluded

Excluding the freshest 8 h removes the draining buckets, so anything non-zero
here is genuine floor rather than lag:

| leg | `quote_asset_id` | candidates | unpriced | % |
|---|---|---|---|---|
| **XLM** | 4 | 138,722 | **12** | **0.0** |
| **USDC** | 3 | 90,672 | **0** | **0.0** |
| yXLM | 10 | 44,432 | 44,432 | **100** |
| yUSDC | 32 | 21,996 | 21,996 | **100** |
| XRP | 40 | 15,175 | 15,175 | **100** |
| SHX | 14 | 9,413 | 9,413 | **100** |
| YxT | 2 | 9,040 | 9,040 | **100** |
| VELO | 28 | 8,772 | 8,772 | **100** |
| LIBRE | 11 | 6,848 | 6,848 | **100** |
| USTRY, sUSD, bubba, xLMNR, RIO, SSLX, m0N33Tr33, xRUNES, XXA, XTROOP | — | 2.0-4.6 K each | all | **100** |

**There is no middle ground.** A leg is either fully priced or fully unpriced —
100.0%, not 99% or 95%, on every exotic leg. That is the signature of a
structural gap (no oracle, no pivot reference) rather than a backlog, and it
matches [[0056]]'s exotic-quote floor exactly.

Floor ≈ **88 K rows/day** against ≈ **138 K/day** of priceable inflow.

### Three things this settles

- ✅ **Leg identities confirmed against `prices.assets`** — `3` = USDC,
  `4` = XLM. [[0139]]'s warning was heeded; the cohorts were right. The 08-21
  guesses for 10/28/11 were also right (yXLM / VELO / LIBRE); `1221` = yBASH and
  `201223` = TGM.
- ✅ **USDT (`111`) does not appear in the top 20 at all.** Live USDT-quoted
  volume is negligible, so the peg tier has essentially nothing to do in the live
  window. This is why the 08-21 "floor" cohort read ~100% even though USDT is
  peg-priceable — there is almost no USDT in it.
- ⚠️ **yXLM (44,432) and yUSDC (21,996) are 45% of the floor** and the two
  largest legs in it — yXLM alone is a third of XLM's own volume. They are
  wrapped versions of the exact two assets we *can* price, unpriceable only
  because no pivot tier covers them. That is [[0154]]'s scope, **not a defect
  here**, and no task is spawned for it.

### Why this and the 08-21 measurement are both correct

They measured different regimes, and the aggregate reconciles exactly.

The 08-21 window ran 08-19 20:00 → 08-21 20:45; the [[0215]] Caddy fix landed
before 14:00 on 08-21, so **~42 of those 48 h are pre-fix**, when the pass died
at 30.0 s on every invocation and the live window genuinely *was* starved.

Take 08-22 20:00 from the cohort table: 17,038 unpriced of 33,268 candidates =
**51.2%**, sitting squarely inside 08-21's 50-58% band. The aggregate looks flat
because the floor is roughly half the candidates at ~99.7% and never moves —
masking a priceable half that goes to zero. The 08-21 section *identified* that
masking effect and then drew the opposite conclusion from it without testing.
This is that test.

> **The starvation was real, and [[0215]] already fixed it. It was never 0111's
> scan cost.** Post-0215 the live priceable legs keep up fine even with the
> unbounded 736 M-row scan.

### What it changes for the fix

**Phase 1's payoff reverts to the original, smaller case.** Every reason it is
still needed is untouched:

- `prices-production-enrichment-duration-near-timeout` firing at
  **300,000 / 300,338 ms** — the thing that re-opened this task.
- The pass budgets **40 batches** per invocation and achieves **5.4**.
- The historical drain needs **~322 days** at the measured 1.73 M/day.
- [[0218]] is blocked behind it (`main.rs:167`'s budget goes to 0).

What it does change is **AC 6, which now has a far sharper check** — see below.
And it raises a risk the starvation reading was masking: the Phase 2 historical
sweep is the one thing that *could* starve a live window which is currently
healthy. AC 6 is exactly that guard, and it is now able to detect it.

## What this does to the options

**Option 1 wins, for a better reason than the one recorded above.** The live
window is 17.05M rows and keeps up fine (2.81K unpriced). It is being dragged
through a 736M-row scan purely to serve a historical drain. Splitting the live
pass from a separate bounded historical sweep is the whole fix; partition
pruning is the mechanism, not the argument.

⚠️ **"2.81K unpriced" is still the wrong figure — but "keeps up fine" is
RIGHT.** 2.81 K is XLM-only; all quote legs give **10.44 M of 17.05 M (61.2%)**.
That 61.2% is however **almost entirely the permanent exotic-quote floor**: the
two priceable legs measured 2026-08-24 sit at **0.0% unpriced** (XLM 12 of
138,722; USDC 0 of 90,672) while every exotic leg sits at **exactly 100%**. The
08-21 "it is starved" correction applied to the pre-[[0215]] regime and is
withdrawn. The conclusion in this paragraph survives on its original terms: the
live window is carried *inefficiently*, and splitting it from the drain is still
the whole fix.

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
| live window (`202607+`) | 17.05 M | ~~2.81 K~~ **10.44 M**, ~all floor | **freshness** |
| historical (`< 202403`) | 689.64 M | 646.25 M | **throughput** |

<sub>*The 2.81 K was XLM-quoted only. The all-quotes figure is 10.44 M, but
measured 2026-08-24 that is **almost entirely the permanent exotic-quote floor**
— the priceable legs (USDC, XLM) run at 0.0% unpriced. The live window is
carried inefficiently, not starved; the 08-21 starvation reading applied to the
pre-[[0215]] regime and is withdrawn. Neither figure changes the split.*</sub>

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

- [x] **A bounded pass runs to completion inside the Lambda budget**, measured
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

- [x] **Per-batch rows-read is bounded and does not scale with total table
      size**, verified in `system.query_log`, not by wall-clock on a quiet
      cluster. Baseline to beat (2026-08-21): oracle **26.21 s / 735.78M**, XLM
      pivot **44.93 s / 685.01M**, peg **14.50 s / 420.93M**,
      `count_candidates` **9.27 s / 735.15M**. Target ≈ **17M** on every tier.

- [x] **Re-measured while the cluster is actively writing** — a quiet cluster is
      how this was missed. The drained-state numbers (0.3 s) are 80× faster than
      the same query under load. A measurement taken against an idle cluster
      does not satisfy this AC no matter how good it looks.

- [x] **The drain demonstrably walks, and its terminal state is declared.**
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

- [ ] **(DEFERRED to [[0220]])** **0026's `EnrichmentPassDurationMs` stays well clear of 300 s for a week**
      spanning active backfill, and
      `prices-production-enrichment-duration-near-timeout` returns to OK and
      stays there. That alarm firing at **300,000 / 300,338 ms** on 2026-08-21
      is what re-opened this task.

- [x] **The live window is not starved by the historical sweep.**
      `EnrichmentRowsRemainingRecent` stays at its floor. The sweep is
      best-effort and time-budgeted specifically so it can never regress the
      live pass; this is the check that the budgeting works.

🔴 **Judge this AC per-leg, NOT on
      `EnrichmentRowsRemainingRecent`.** That metric is ~100% permanent
      exotic-quote floor (~88 K rows/day) and will read about the same whether
      the fix works or not, so it cannot detect a starved live pass. It is also
      currently **unpublished** — the pass is killed before `pass_metrics` runs
      — so first prove it has datapoints at all.

      The real check, measured 2026-08-24 and now the baseline:

      | cohort | healthy value |
      |---|---|
      | USDC (`quote_asset_id = 3`) | **0 unpriced** outside the newest 4 h bucket |
      | XLM (`quote_asset_id = 4`) | **~0** (12 of 138,722 over 40 h) |
      | every other leg | **100% unpriced — unchanged, by design** |

      If the Phase 2 historical sweep starves the live pass, USDC and XLM lift
      off zero and it is unmistakable. Use the leg-level query from
      *Re-measured 2026-08-24*, with the newest 8 h excluded so lag is not read
      as floor.

      <sub>*Was: judge against ~27 K falling toward ~12-15 K. That was derived
      from the withdrawn starvation reading and would have passed a starved
      pass, since the floor dominates the aggregate.*</sub>

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

## ✅ Completed 2026-08-24 — deployed and verified

Deployed to production **2026-08-24 08:03:24 UTC**
(`make -C infra deploy-production-eventbridge`, EventBridge stack only).
Shipped: `ENRICH_LIVE_PARTITIONS=2`, `ENRICH_HISTORICAL_SWEEP=true`,
`ENRICH_HISTORICAL_MAX_MONTHS=1`, `ENRICH_HISTORICAL_TIME_BUDGET_SECS=120`.
`CleanupRule` verified DISABLED before and after.

### AC 2 — per-tier cost, measured post-deploy in `system.query_log`

| tier | before (2026-08-21) | after | reduction |
|---|---|---|---|
| oracle | 26.21 s / 735.78 M | **0.62 s / 13.00 M** | 57x rows, 42x time |
| pivot XLM | 44.93 s / 685.01 M | **0.52 s / 10.53 M** | 65x rows, 86x time |
| `count_candidates` | 9.27 s / 735.15 M | **0.08 s / 4.27 M** | **172x rows**, 116x time |

Target was ~17 M. Every tier came in **under** it.

### AC 3 — measured under real load, not on a quiet cluster

**6,539 inserts writing 226,254,819 rows in the preceding 30 minutes.** This is
the AC that the July measurement failed silently; it is properly satisfied here.

### AC 1 — a pass completes, every hour

`enrichment pass complete` (carrying `candidates_before`/`candidates_after`,
i.e. `count_remaining_at_volume_zero`) appears **once per invocation**:
08:17:27 enriched=3,600 dur=7,213 ms; 09:17:25 enriched=6,078 dur=5,220 ms;
10:17:27 enriched=4,569 dur=7,319 ms. It appeared **zero** times in 2.5 h before.

⚠️ The AC's "~3/hour" target assumed 3 attempts/hour. Post-fix the Lambda runs
**1 invocation with 0 errors** instead of 3 with 3, so 1 completed pass/hour is
the better outcome, not a miss. Invocations/Errors went 3/3 → 1/0.

### AC 4 — the frontier walks and self-exhausts

| month | state | zeros_seen | swept_at |
|---|---|---|---|
| 201511 | `exhausted` | 19 | 08:17:28 |
| 201512 | `exhausted` | 4 | 09:17:26 |
| 201601 | `exhausted` | 3 | 10:17:28 |

One month per invocation, oldest-first, `deadline_hit=false` every run. Each
pre-2021 month is marked `exhausted` on first visit — **Phase 3 working with no
hard-coded cutoff**, which was the main correctness argument for this design.

### AC 6 — the live window is not starved

Legs 3 (USDC) and 4 (XLM) over a 40 h window excluding the freshest 8 h:
**XLM = 17, USDC = 12**, against ~138,722 and ~90,672 candidates (~0.01%).
Judged per-leg, not on `EnrichmentRowsRemainingRecent`, which is ~100% permanent
exotic-quote floor and would pass a starved pass.

### Issues encountered

- 🔴 **The `cdk diff` blocker was NOT a stale prod build.** It showed an
  unexplained `RollupFreshnessProbeFunction` code-asset replacement and held the
  deploy for three days. Root cause is **cargo feature unification**: the probe
  built alone hashes `19455046…` / 12,039,704 B — **byte-identical to prod** —
  while the same source built alongside the other nine crates gives
  `0414a138…` / 12,473,200 B (+433 KB). Same `rustc`, same dep versions. The
  control that proved it: `cleanup-worker` rebuilt byte-identical to prod.
  Recorded in memory as `lambda-asset-diff-is-feature-unification`.
- ⚠️ The prediction that `init.sql` being `include_str!`-ed would ripple into
  several lambdas was **false** — it is dead-code-eliminated from every crate
  except `enrichment-worker`.

### Side effects — two other tasks cleared

- ✅ **[[0130]]** — the coarse sweep could never scan `price_ohlcv_15m` (Caddy
  504). Now `tables_swept=6, tables_failed=0`.
- ✅ **[[0218]] AC 1** — the coarse sweep had never executed in production. It
  now runs every invocation, ~205 K rows enriched. 0218 stays open for ACs 2-4.

### 🎁 Memory headroom for [[0154]]

Peak memory is now **517-581 MiB**, against **1.08-1.45 GiB** before. This file's
warning that "0154 and option 5 both add a join to this pass; there is less
headroom here than the options list assumes" is **obsolete** — there is ~2.5x
more headroom than when it was written.

## Out of scope

- The alarm blind spot that let this run four days unnoticed — that is [[0112]].
- The `price_ohlcv_15m` pre-roll INSERT (217 s avg / 301 s max, 177M rows) —
  **now [[0113]]**. Note the description above was wrong when first written: it
  called this "the same 300 s wall", implying a Lambda timeout. The pre-roll is
  operator-run SQL, not a Lambda, so no timeout applies; the real exposure is
  memory (2,938 MB of a 5.59 GiB quota). 0113 carries the corrected framing.
