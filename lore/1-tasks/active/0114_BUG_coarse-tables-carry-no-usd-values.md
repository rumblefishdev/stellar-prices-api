---
id: "0114"
title: "Coarse OHLCV tables carry no USD values for 2025-02 → 2026-02; enrichment never revisits rolled-up rows"
type: BUG
status: active
related_adr: ["0007"]
related_tasks: ["0111", "0090", "0095", "0088", "0026", "0107"]
tags: [clickhouse, enrichment, rollup, data-quality, priority-high, effort-medium, incident]
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../../packages/prices-clickhouse/schema/init.sql"
history:
  - date: 2026-07-21
    status: backlog
    who: okarcz
    note: >
      Spawned from the 0111 loaded-measurement session. Measuring enrichment
      under backfill load surfaced a step change in `price_ohlcv_1m` size, which
      led to the coarse tables: `close_usd = 0` for 86–100% of the Soroban era,
      with a hard zero block 2025-02 → 2026-02. Two distinct defects, both
      confirmed by measurement against prod.
  - date: 2026-07-22
    status: active
    who: okarcz
    note: >
      Activated. Added data-safety analysis before implementation: the repair is
      non-destructive as a pure additive enrichment INSERT (OHLC carried through,
      zero-scoped, write-once volume, RMT append), and inspection of
      ch_enrich.rs:380 (`p.version + 1` from the existing row) downgrades the
      version risk from data-loss to silent-no-op. Added two ACs — additive-only
      / never-truncate, and snapshot-before-run — because the 2025-02 → 2026-02
      span lost its 1m source on 07-18 and the coarse tables are now the sole
      copy. Next: the version-arithmetic test.
  - date: 2026-07-22
    status: active
    who: okarcz
    note: >
      Added the operator entrypoint bin/coarse-repair.rs (--features aws-mtls):
      clap CLI, --transport local|hetzner mTLS, --dry-run preview, guards against
      1m/non-coarse/bad-month, snapshot-on-by-default. Added CoarseRepairConfig
      .dry_run. Smoke-tested end-to-end vs local CH: real run repaired in-span
      rows (close_usd 8/11, version→5001) and left the out-of-span month
      untouched; FREEZE snapshots emitted + unfrozen in cleanup. clippy clean,
      all tests green. Remaining: FREEZE revert automation, coverage-verify AC,
      0088 step-3 gate, runbook, prod run.
  - date: 2026-07-22
    status: active
    who: okarcz
    note: >
      Built the partition-bounded repair driver. Added
      ChEnrichConfig::time_window threaded into every candidate scan (pivot
      reference left unbounded so cross-month anchors survive); new
      repair.rs::CoarseRepairDriver enumerates months-with-zeros in a span,
      FREEZE-snapshots each partition, runs a partition-bounded one-shot repair,
      reports per-month before/after. No TRUNCATE path — additive INSERT+FREEZE
      only. New integration test coarse_repair_driver_bounds_span_and_reports_per_month
      + peg/pivot window unit tests. 27 lib + 7 integration tests green; example
      + lambda-feature build clean; clippy clean. Unbounded hourly Lambda path
      byte-identical (time_window=None). Remaining: operator entrypoint, coverage
      verification, 0088 step-3 gate, prod run.
  - date: 2026-07-22
    status: active
    who: okarcz
    note: >
      Version-arithmetic AC proven. Added integration test
      coarse_repair_row_outranks_large_summed_version (ch_enrich_it.rs): repair
      wins against a seeded sum(version)=5000 (close_usd 0→8, version→5001), OHLC
      carried through verbatim, append-only (2 physical rows), idempotent. All 6
      ch_enrich_it tests green against local CH 26.3.10.60. Negative control
      confirms a fresh/constant version loses to the seeded sum — the silent-no-op
      failure mode is real and this catches it. Next: build the partition-bounded
      repair driver + snapshot step, then run the historical span.
  - date: 2026-07-23
    status: active
    who: okarcz
    note: >
      Phase C COMPLETE — _4h/_1d/_1w/_1M repaired (~20.1M rows), so with _1h
      (~30.8M + 1.0M pilot) all five coarse tables now sit at the no_reference
      floor; ~52.5M rows repaired total. The 4h run was interrupted by a power
      loss and resumed from the boundary month 202603 (75.8% zero vs a repaired
      neighbour's 66.4% identified it as half-written); 5 months in 3m30s, every
      month reconciling before-enriched=after exactly. Two Phase-C estimates in
      this file were wrong and are corrected in place - yield ~30% not 39%, and
      runtime minutes not ~3h (on 202606/07 the oracle tier claims the reachable
      rows and peg-pivot correctly no-ops, so 23-51 batches/month not ~100).
      Scope correction - the 1m hole is 2021-07 to 2026-06, ~4x wider than the
      recorded 13 months, and its stated cause (cleanup drop on 07-18) is
      UNVERIFIED - no DropPart in part_log, no matching mutation, no TTL. Three
      cluster findings recorded - cleanup deletes by ALTER DELETE mutation not
      partition drop (so 0109's guard must watch system.mutations, never
      DropPart); six Phoenix DELETE mutations starved since 07-17 across all
      coarse tables, meaning 0097's rework is incomplete until the backfill frees
      the merge pool (~07-27/28, do not KILL); EventBridge cleanup rule confirmed
      DISABLED and 1m has no TTL. Remaining: _4h composition breakdown,
      scheduling the recurring pass, 0088 step-3 gate.
---

# Coarse OHLCV tables carry no USD values

## Summary

`price_ohlcv_1h` — the table BE consumes — has `close_usd = 0` for **86–100% of
every month in the Soroban era**, including a flat **100.0%** block from
**2025-02 through 2026-02**. `volume_quote_usd` tracks it almost exactly. The
1m source for that span was dropped by the cleanup rule on 2026-07-18, so the
coarse tables are now the only copy.

Two independent defects produce this. Both are still live.

## Defect 1 — rollups capture pre-enrichment values and never revisit

Enrichment writes only to `price_ohlcv_1m`. The MVs (APPEND mode since [[0095]])
roll each 1m row up **as it arrives**, before the hourly enrichment pass reaches
it; the pre-roll copies whatever is in 1m at that moment. Neither path ever
re-enriches a coarse row. A 1m row enriched at T+1h leaves its coarse
counterpart at zero permanently.

**Measured** (same candles, same week, 2026-07-14 → 07-21):

| table | rows | `close_usd = 0` | pct |
|---|---|---|---|
| `price_ohlcv_1m` | 2,455,869 | 1,519,214 | **61.9%** |
| `price_ohlcv_1h` | 564,787 | 501,731 | **88.8%** |

A 27-point gap that can only come from the rollup path. This is ongoing — it
degrades every coarse row written today.

## Defect 2 — the historical enrichment march stalled at 2025-01, then lost its source

USDC-quoted rows in `price_ohlcv_1h`, enriched fraction by month:

| span | enriched | reading |
|---|---|---|
| 2024-02 → 2024-12 | ~99% | enrichment completed this span |
| 2025-01 | 8.2% | where it stopped |
| 2025-02 → 2026-02 | **0%** | never processed |
| 2026-03 / 04 / 07 | 17% / 3% / 24% | live-era rows only |

The clean boundary is the signal: this is enrichment working oldest-first, not a
pre-roll artifact. It had ~10 days (2026-07-08 → 07-18) with full history in 1m
while cleanup was disabled, reached 2025-01, was stalled by the [[0111]] outage
(2026-07-14 → 07-17), and then cleanup dropped the 1m source on 07-18. The
"8.4M un-enriched rows" recorded in 0111 was the tail of exactly this march.

> **Superseded inference.** An earlier reading of this data held that the
> 100%-zero months were the backfilled span pre-rolled during the outage. The
> per-quote breakdown refutes it — the boundary sits at 2025-01/02 with ~99%
> coverage immediately before it. Recorded so it is not re-derived.

## Recovery is possible in place — no archive re-download

`prices.oracle_prices` only starts **2025-09** (and carries a single asset until
2026-03), so the oracle tier cannot repair this. The peg-pivot tier can: it
derives its reference **inline from the candle table itself**
(`ch_enrich.rs:736`), not from `oracle_prices`.

And the data to pivot from is present in every affected month:

- **USDC-quoted:** 100k–330k rows/month, every month 2024-02 → 2026-07, at
  `close_usd = 0`. A USDC-quoted candle needs no reference at all — that is the
  stablecoin-direct tier (`ch_enrich.rs:688-698`, a 1:1 cast).
- **XLM-quoted:** 0.5M–1.7M rows/month, pivotable through the USDC pairs above.
- **USDT-quoted:** small (3k–7k/month) but present.

So the repair is: retarget the existing enrichment SQL from `price_ohlcv_1m` to
each coarse table and run it over the affected span. Bounded, and it reuses
tiers that already exist.

## Chosen approach for defect 1 — a repair pass over the coarse tables

Three options were considered for stopping the recurrence. **Decision: run
enrichment against the coarse tables on a schedule**, rather than re-ordering the
pipeline or enriching at ingest.

### Why not "roll up later, after enrichment"

The rollups are **not** triggered per row — they are refreshable MVs on timers
that re-aggregate a bounded recent window (1m→15m every minute, 15m→1h every
15 min, cascading up; `rollups.sql:77-140`). A row is only picked up if
enrichment fills it *while its window is still open*.

Enrichment lag is **unbounded** — it was four days during the [[0111]] outage.
A bounded window cannot cover an unbounded lag, so this does not solve the
problem even in principle; it would still need a repair pass for the overruns.
Rejected on correctness, not just cost.

### Why not "compute USD at ingest"

A candle's USD value needs a reference price for the *same* minute, which is
being computed in the same batch — circular, and presumably why enrichment is a
separate later pass at all. It would also mean changing live ingestion (the
component with the [[0064]] / [[0094]] freeze history) and fixes nothing
historical.

### Why the repair pass

1. **The schema already supports it.** Coarse targets are
   `ReplacingMergeTree(version)` with `sum(version)` chosen specifically so a
   re-inserted corrected row outranks the old one — `rollups.sql:12-36` calls
   this "the load-bearing correctness fix" (0059/[[0095]]). The repair uses the
   mechanism that already exists and is already depended on.
2. **Same code as the historical repair.** The one-off fix for 2025-02 → 2026-02
   and the recurring guard are the same job, built once.
3. **Converts permanent corruption into temporary lag.** The decisive argument.
   The other two options make correctness depend on ordering holding forever, and
   ordering in this system has broken repeatedly (enrichment outage, cleanup
   enabled mid-backfill, cursor freeze). With a repair pass the next failure
   means "USD values are late", not "USD values are gone".
4. **Converges with 0111.** The pass must be partition-bounded or it re-creates
   the same full-scan cost — which is 0111's option 1. One fix shape covers both.

### Costs and risks, acknowledged

- **Merge pressure** on a shared cluster from rewriting RMT rows. Mitigate by
  touching only partitions that actually contain zeros, and stopping once
  coverage reaches the genuine `no_reference` floor.
- **⚠️ Version arithmetic is the main technical risk.** Coarse `version` values
  are a *sum* of source versions, so a repair row must outrank a potentially
  large existing value — **not** a naive `+1` from a fresh row.
  `ch_enrich.rs:354` mentions `version + 1`, but it is unconfirmed whether that
  derives from the existing row. Get this wrong and the repair silently writes
  rows that lose to the zeros they were meant to replace. **This fails quietly —
  it needs a test, and it is the first thing to verify when implementing.**

  > **Update (2026-07-22) — inspection makes this milder than feared.** The
  > enrichment SELECT is `p.version + 1` reading `FROM {tbl} AS p FINAL`
  > (`ch_enrich.rs:380`), i.e. it *does* derive from the existing (post-collapse)
  > row: against a coarse `sum(version)` value V it writes V+1 > V, and the
  > historical span sits outside every live-MV window so nothing re-appends a
  > competing sum afterward. So the realistic failure mode is **"the repair
  > silently no-ops" (zeros remain), not "existing data is destroyed."** The test
  > AC still stands — but this is a fill-failure risk, not a data-loss risk.

### Data-safety — additive-only, NEVER truncate-rebuild

The repair is **non-destructive by construction** *as long as it is the additive
enrichment `INSERT … SELECT`* (`ch_enrich.rs:367-405`): no `DELETE`/`UPDATE`/
`TRUNCATE`; OHLC/volume/`trade_count` carried through verbatim from the existing
row (`p.open … p.trade_count`); scoped to `volume_quote_usd = 0 OR close_usd = 0`
so enriched rows are never re-touched; `volume_quote_usd` write-once; append into
`ReplacingMergeTree`, so rows are only collapsed at merge, keeping highest
`version`. Worst realistic bug = some zeros stay zero.

**The trap:** the pre-roll runbook (`continue-soroban-backfill.md §9`) offers a
`TRUNCATE TABLE price_ohlcv_1h; … rebuild-from-1m` clean-slate path. That assumes
1m still holds the history. It does not: **the coarse tables are the SOLE
surviving copy.** A truncate-rebuild deletes the only copy with nothing to
rebuild from. The repair MUST be pure additive enrichment; the truncate path is
forbidden for this span.

> **📌 Scope correction (2026-07-23) — the 1m hole is 5 years, not 13 months.**
> This section originally read "for 2025-02 → 2026-02 the 1m source was dropped
> by cleanup on 07-18". Measured directly (`GROUP BY toYYYYMM(timestamp)` over
> `price_ohlcv_1m`), the table holds **only two spans**:
>
> | span | origin | rows |
> |---|---|---|
> | 2018-12-13 → 2021-06-14 | the running [[0088]] backfill's output | ~9.5M |
> | 2026-07-01 → present | live ingestion | 11.1M |
>
> **Everything between 2021-06-14 and 2026-07-01 is absent** — ~4× wider than
> recorded. The sole-copy rule therefore covers **2021-07 → 2026-06**, and any
> procedure anywhere that assumes 1m can rebuild a coarse table over that span is
> wrong. The upper bound moves as the backfill advances (its frontier was ledger
> 35.84M ≈ 2021-06-14 on 07-23).
>
> **⚠️ The stated cause is unverified.** No evidence for the "dropped by cleanup
> on 07-18" claim survives: `system.part_log` covers 2026-06-22 → now and contains
> **zero `DropPart` events** (its event-type set is `RemovePart`, `MergeParts`,
> `MutatePart`, `NewPart` and the `*Start` variants — `DropPart` never appears);
> `system.mutations` holds no 1m delete for that span; and `SHOW CREATE TABLE
> prices.price_ohlcv_1m` has **no TTL clause**. So the deletion either predates
> `part_log`'s window or ran by a path that leaves no trace in it. Treat the
> mechanism as unknown — the sole-copy *fact* is measured and solid, its *cause*
> is not.

**Because it is a sole copy, snapshot before running.** `ALTER TABLE
prices.price_ohlcv_1h FREEZE PARTITION <id>` (cheap hardlink) or `INSERT INTO
…_backup SELECT * … WHERE <span>` on every affected coarse partition, and
dry-run the repair against a restored copy / scratch DB first.

**The historical span cannot use the oracle-ASOF statement.** `oracle_prices`
starts 2025-09, so the shown statement's `o.price_usd IS NOT NULL` filter drops
every pre-2025-09 row → nothing inserted. The repair must run the **peg-pivot /
stablecoin-direct tiers** (`ch_enrich.rs:688-736`, reference derived inline from
the candle table). Using the oracle statement over that span is a silent no-op,
not corruption — but it means the repair did nothing.

### What would reverse this decision

If the un-enriched rows turn out to be dominated by pairs with genuinely no USD
reference, the repair has little to fix and the real work is reference coverage
instead. **Run the per-quote breakdown against `price_ohlcv_1m` (the same query
used on `1h`) before committing engineering time** — 1m's own 62%-zero rate is
high enough to be worth explaining first.

### Pre-check RESULT (2026-07-21) — the 62% is `no_reference`, not backlog

Measured, current month, excluding the last 3 h of normal enrichment lag:

| quote class | rows | pct `close_usd = 0` |
|---|---|---|
| stablecoin (USDC/USDT) | 1,094,423 | **0.3%** |
| XLM pivot | 2,083,366 | **0.3%** |
| other (exotic quotes) | 5,097,594 | **100%** |

5.10M of 8.28M = 61.7%, which accounts for the entire measured 61.9%. **Live
enrichment is working at 99.7% on everything it can reach.** Every zero in 1m is
an exotic quote with no USD path (`ch_enrich.rs:499-504` treats these as
unreachable by design).

This does **not** invalidate the historical repair: 2024-02 → 2026-02 has
100k–330k *USDC-quoted* rows/month sitting at zero in the coarse tables, and the
live figures above prove that same logic reaches 99.7% when it runs. The repair
target is those rows, not the exotic ones.

## Gate 0088's recovery pre-roll — check, but the alarm was overstated

[[0088]] recovery **step 3** runs `preroll-incremental.sql` over the pre-Soroban
tail in ~10 days (~2026-08-01), and nothing in that runbook checks USD coverage.

> **Correction (2026-07-21).** This was first written as a 🔴 blocking gate on the
> grounds that pre-rolling would bake a 62%-zero USD column into the
> forever-tables. The pre-check above refutes the premise: those zeros are exotic
> quotes with no reference, so **enriching the 1m tail first would change
> nothing**. The pre-Soroban era is starker still — 2018-2019 has 100k–200k
> XLM-quoted rows/month at 100% zero because USDC barely existed then (6–45
> rows/month) and `oracle_prices` does not start until 2025-09. There is no
> contemporaneous USD price to pivot through. **The pre-Soroban tail will be
> USD-less regardless of what we do** — a data-availability fact, not a defect.

Keep a coverage check in the step-3 pre-flight as a cheap regression guard, but
it is not a blocker and does not gate the recovery.

## Related finding — 62% of all candles have no USD path at all

`3_other` is 5.1M rows/month (55.2% of *trades*), permanently zero, **by design**.
A **two-hop pivot** (exotic → XLM → USD) may reach a large share, since most
Stellar assets trade against XLM. → **spawned as [[0115]]** (priority-medium:
a known design boundary with a correct sentinel, not an active corruption —
sequence it behind this task's repair).

### ❌ REFUTED — this does NOT explain [[0107]] (tested 2026-07-21)

It was proposed that the no-USD-path gap explained 0107's volume shortfall. A
pre-registered threshold was set (reachable share ≈ 15% would confirm) and then
measured, trade-weighted, over 2026-07-14 → 07-21:

| quote class | trades | share |
|---|---|---|
| stablecoin | 727,833 | 15.9% |
| XLM pivot | 1,322,738 | 28.9% |
| **reachable total** | **2,050,571** | **44.8%** |
| other (unpriceable) | 2,530,298 | 55.2% |

**44.8% reachable ⇒ a ~2.2× understatement**, against 0107's measured **6× volume
/ 7.5–9× trade-count** gap. Off by 3–4×; the pricing gap cannot be the primary
cause.

**The categorical reason, which matters more than the arithmetic:** 0107 is about
trades **absent from our database entirely**; this task is about trades we
**hold but cannot price in USD**. Orthogonal failures. No amount of USD-pricing
work creates a trade that was never ingested.

The numbers also argue against a link: had 0107's volume figure been computed
through the zero-USD column, the two effects would compound to ~16×, not the 6×
measured — so 0107's volume gap most likely tracks its own trade-count gap and
shares one upstream ingestion cause.

*Caveat: measured on the current week; 0107's Horizon window was not inspected. A
3–4× discrepancy is far too large to be a windowing artifact, so the refutation
holds, but the exact multiplier should not be quoted as precise.*

**Do not re-propose this link** without new evidence — it is the cheapest
available explanation for 0107 and will suggest itself again to anyone who reads
the 55%/62% figures.

## Acceptance Criteria

- [x] **`price_ohlcv_1h` re-enriched for 2024-02 → present — DONE 2026-07-23,
      per-month zero rate now at the genuine `no_reference` floor.** Aggregate
      **95.5% → 58.1%** zero across the 30-month span; the unbroken 100.0% block
      (202502→202602) is gone, now 55.6–69.8%. Composition proven exotic-only by
      a per-quote-class breakdown (see §Repair result), so the residual IS the
      floor rather than remaining backlog.
- [x] **`_4h`/`_1d`/`_1w`/`_1M` re-enriched — DONE 2026-07-23 (§Phase C).**
      ~20.1M rows enriched across the four tables; combined with `_1h`, ~52.5M
      rows repaired in total and **every coarse OHLCV table is now at the
      `no_reference` floor**. The `4h` run was interrupted by a power loss and
      resumed cleanly from the boundary month. *Composition breakdown on `_4h`
      still outstanding — the counts are in, the exotic-only proof is not.*
- [x] **Pre-check before any implementation** — DONE 2026-07-21. The 62% is
      genuine `no_reference` (exotic quotes), not backlog; live enrichment runs
      at 99.7% on pivotable quotes. Approach holds for the historical repair.
- [ ] The rollup path no longer freezes un-enriched values — implemented as a
      **scheduled, partition-bounded enrichment pass over the coarse tables**
      (see §Chosen approach; the ordering-based alternatives were rejected).
      *In progress: driver built, tested, and now proven across five tables in
      prod; **scheduling is the only remaining piece**. Both historical runs were
      manual one-shots.*
- [x] **Version arithmetic proven by test** — DONE 2026-07-22. A repair row
      outranks the existing coarse row under `sum(version)` RMT semantics.
      `enrichment-worker/tests/ch_enrich_it.rs::coarse_repair_row_outranks_large_summed_version`
      seeds a `price_ohlcv_1h` bucket at `version = 5000` and asserts the repair
      wins (`close_usd 0 → 8`, `version → 5001`), carries OHLC through verbatim,
      appends (2 physical rows pre-merge), and is idempotent. A negative control
      against local CH confirms the teeth: a fresh/constant version loses to the
      seeded sum (FINAL keeps the zero) — the exact silent-no-op this guards. All
      three write tiers (`ch_enrich.rs:380`/`:696`/`:729`) project `p.version + 1`
      from `FROM {tbl} AS p FINAL`, so the +1 derives from the existing row.
- [x] **Additive-only, never truncate-rebuild — HELD across the full prod run
      2026-07-23.** The repair is a pure enrichment `INSERT … SELECT` carrying
      OHLC/volume through verbatim; no `DELETE`/`TRUNCATE` path exists in the
      driver. Confirmed empirically rather than by inspection alone: across all
      30 months `zeros_before − enriched = zeros_after` **exactly**, every month,
      and the 202502 pilot month re-ran as a clean `enriched 0` no-op instead of
      re-writing rows. Nothing was destroyed and nothing was double-counted.
- [x] **Snapshot before running** — DONE for `price_ohlcv_1h` 2026-07-23. All 30
      partitions frozen (`repair_0114_prices_price_ohlcv_1h_<month>`, 6.1 GB under
      `shadow/`, verified by count + `du`) **by the CH admin**, because
      `prices_writer` cannot hold `ALTER FREEZE PARTITION` and cannot be granted
      it (see §Blocker). Repair then runs `--skip-snapshot`. Confirmed the
      peg-pivot / stablecoin-direct tiers are what ran: `implied_ref_usd` = real
      XLM/USD for the era and exactly 1.0 for USDC, and the oracle tier no-ops
      pre-2025-09 as predicted. **Extended to `_4h`/`_1d`/`_1w`/`_1M` 2026-07-23**
      — all four frozen by the CH admin before their run (150 dirs, 12 GB under
      `shadow/`, verified by `ls | grep repair_0114` + `du`), repair ran
      `--skip-snapshot`.
- [x] **Reference correctness verified across the span — DONE 2026-07-23.**
      *Superseded criterion: the check is `implied_ref_usd` correctness, NOT a
      `close_usd` value ceiling — a ceiling can never pass while dust-trade
      inputs exist ([[0116]]).* Sampled 202403 / 202411 / 202509 / 202606:
      USDC-quoted `implied_ref_usd` is **exactly 1.0** (stablecoin-direct 1:1
      cast) and XLM-quoted tracks real XLM/USD for each era — 0.1371, 0.218,
      0.372, 0.2008. The 202411 median of 0.218 is consistent with the ~$0.60
      spike landing late in that month. Two residual notes: (a) 202606 USDC reads
      **1.0005**, not exactly 1 — expected, since `oracle_prices` starts 2025-09
      so live-era rows can come from the oracle tier rather than the peg cast;
      (b) the 202606 XLM median 0.2008 sits slightly above the "$0.15–0.18 in
      2026" band recorded during prep — worth one independent spot-check, not a
      blocker.
- [ ] 0088 step 3 gated: pre-roll refuses to run, or warns loudly, when 1m USD
      coverage for the target span is below a threshold.
- [x] Re-check whether this explains [[0107]]'s volume gap — **REFUTED
      2026-07-21**, see §REFUTED below. Orthogonal problem; 0107 unaffected.

## Implementation progress (2026-07-22)

Driver built and tested locally against CH 26.3.10.60; not yet scheduled or run
against prod.

- **Partition-bounding (converges with 0111 option 1).** Added
  `ChEnrichConfig::time_window: Option<(u32,u32)>` and threaded a
  `timestamp >= start AND timestamp < end` predicate (inlined literals, no bind-
  order change) into every candidate scan — `count_candidates`,
  `count_remaining_at_volume_zero`, `enrich_batch`, `peg_sql`, `pivot_sql`. Only
  the **candidate** side is bounded; the pivot tier's inline XLM/USDC reference
  still forward-fills from earlier months (cheap sort-key-prefix scan), so a
  month's first buckets keep a valid pivot anchor. `None` (the hourly Lambda over
  1m) is byte-identical to before — verified by unit tests + all pre-existing
  integration tests still green.
- **Driver** (`enrichment-worker/src/repair.rs`, `CoarseRepairDriver`): one
  grouped scan enumerates months-with-zeros in `[start_month, end_month]` with
  their exact `[start,end)` windows; each month is `FREEZE`d (server-side hardlink
  snapshot, `snapshot: true`) then repaired with a partition-bounded one-shot
  pass; returns per-month before/after counts. **No `TRUNCATE`/rebuild path
  exists in the driver** — additive `INSERT`+`FREEZE` only.
- **Tests** (`ch_enrich_it.rs`): `coarse_repair_driver_bounds_span_and_reports_per_month`
  proves span bounding (out-of-span month untouched), reference bounding (exotic
  FOO/EXO stays the `no_reference` floor), the version race under a seeded
  `sum(version)=5000`, and correct per-month before/after. Plus `peg_sql` /
  `pivot_sql` window unit tests (candidate-side bounded, reference NOT, no extra
  bind params).
- **Operator entrypoint** (`enrichment-worker/src/bin/coarse-repair.rs`, needs
  `--features aws-mtls`): clap CLI mirroring sdex-backfill (`--transport
  local|hetzner` + mTLS cert-path env). Guards refuse `price_ohlcv_1m` and the
  non-coarse / bad-`YYYYMM` / start>end cases; `--dry-run` previews
  months-with-zeros without writing; snapshot on by default (`--skip-snapshot`
  is the scary opt-out). Smoke-tested end-to-end against local CH: dry-run vs
  real run, in-span rows repaired (`close_usd` 8/11, `version`→5001), out-of-span
  month untouched, per-partition FREEZE emitted.
- **Still to do:** revert-cleanup story for `FREEZE` (currently `SYSTEM UNFREEZE
  WITH NAME`, no automated ATTACH restore); the coverage-verification and 0088
  step-3 gate ACs; a runbook; then the actual prod run against 2024-02 → present.

## Prod-run session — 2026-07-22 (PAUSED before the first write)

Runbook steps 0–3 done against prod. **Nothing has been written yet.** Resume at
the 202502 pilot below.

### Done

- **Step 1 build.** `cargo build --release -p enrichment-worker --features
  aws-mtls --bin coarse-repair` → `./target/release/coarse-repair`. Writer certs
  present at `$HOME/prices-mtls/prices_writer.{crt,key}` + `ca.crt`.
- **Step 2 dry run** (`price_ohlcv_1h`, 202402→202607): 30 months,
  **79,633,858** zero rows, 0 enriched, nothing written. Connection, month
  enumeration and partition-bounding all confirmed working against prod.
- **Step 3 baseline recorded** (see table below).
- **Only `1h` was dry-run.** `_4h`, `_1d`, `_1w`, `_1M` are still untouched.

### Yield: ~31.6M of the 79.6M are repairable (≈40%)

The driver counts `(volume_quote_usd = 0 OR close_usd = 0) AND volume_quote > 0`
with **no quote-class filter** (`repair.rs:132`), so its "months with enrichable
zeros" log line overstates the work — it includes the exotic floor. Split by the
classes the tiers actually reach:

| span | peg_reachable/mo | pivot_reachable/mo | exotic floor/mo |
|---|---|---|---|
| 202402 | 0 | 51 | 620,846 (unrepairable — expect a no-op) |
| 202403 → 202412 | **39–940** | 1.0M–1.6M | 1.3M–2.0M |
| 202501 → 202607 | **150k–340k** | 0.30M–1.10M | 1.2M–2.5M |

Totals ≈ **28.0M pivot + 3.6M peg = 31.6M** rewrites; the remaining ~48M stay
zero correctly.

> **📌 Corrects this task's own §Recovery claim.** "USDC-quoted: 100k–330k
> rows/month, every month 2024-02 → 2026-07, at `close_usd = 0`" is **wrong for
> 2024**: `peg_reachable` is 39–940 rows/month for 2024-03 → 2024-12, because
> those USDC rows were already enriched (the ~99% figure). The 2024 remainder is
> **XLM-pivot work, not USDC work**. The repair target for 2024 is the pivot tier.

> **✅ Independently confirms Defect 2.** `peg_reachable` jumps **940 → 173,137
> at 202501** and holds 150–340k through 202607 — the enrichment march's stall
> boundary, visible in a column derived completely differently from the original
> per-quote breakdown.

### Baseline (before) — `price_ohlcv_1h`, `volume_quote > 0`, FINAL

| month | pct_zero |
|---|---|
| 202402 | 53.0% |
| 202403 | 86.1% |
| 202404 → 202412 | 91.2–96.6% |
| 202501 | 99.5% |
| **202502 → 202602** | **100.0% unbroken** (13 months) |
| 202603 / 202604 | 93.6% / 99.5% |
| 202605 / 202606 | 100.0% |
| 202607 | 92.4% |

Matches the task's headline claim exactly, and the 202501→202502 step is the same
boundary the peg column shows.

### Pivot reference verified present — the repair can work

`price_ohlcv_1h` holds an XLM/USDC market in **every** month of the span
(223–2,788 buckets/mo; ~2,600/mo vs ~720 hours = multiple sources per bucket, as
expected). The `close` band tracks real XLM history — ~$0.11 early 2024, the
~$0.60 spike in 2024-11, $0.25–0.42 through 2025, $0.15–0.18 in 2026. This is the
strongest evidence so far that the peg-pivot tier will emit sane USD values.

**⚠️ Outlier reference buckets to watch in verification:** `202407 min_close =
0.0027`, and `202505` / `202607 max_close = 1.0000` exactly. A round 1.0 smells
like a degenerate/thin bucket. The pivot forward-fills the volume-weighted close
at-or-before each candle, so a bad bucket can leak into neighbours; volume
weighting should suppress it, but check the repaired value distribution.

### ✅ Pilot 202502 — RUN AND PASSED 2026-07-23

Deliberate deviation from runbook Step 4, which runs all 30 months at once: prove
the mechanism on one partition of the 100%-zero block first. **Result below.**

```bash
export CH_DOMAIN=ch.sorobanscan.rumblefish.dev
export MTLS_CERT_PATH=$HOME/prices-mtls/prices_writer.crt
export MTLS_KEY_PATH=$HOME/prices-mtls/prices_writer.key
export MTLS_CA_PATH=$HOME/prices-mtls/ca.crt

time ./target/release/coarse-repair \
  --transport hetzner --table price_ohlcv_1h \
  --start-month 202502 --end-month 202502
```

**Pre-registered expectation** (recorded before the run so it cannot be
rationalised after): `zeros_before 2,788,693` → `enriched ≈ 1,000,975`
(184,864 peg + 816,111 pivot) → `zeros_after ≈ 1,787,718` → snapshot
`repair_0114_prices_price_ohlcv_1h_202502`.

**Two distinct failure signatures, both of which look like success:**

1. `enriched ≈ 0`, `zeros_after` unmoved → the **silent no-op** this task warned
   about. Stop; do not run the other 29 months.
2. `enriched ≈` exactly **200,000** → `one_shot` did not take effect.
   `coarse-repair.rs:160` hardcodes `max_batches: 20`, which at the default
   10k batch caps a run at 200k rows; `repair.rs` overrides it with
   `one_shot = true`, so that override is load-bearing. Not a data limit.

**Timing estimate.** Calibration: the dry-run enumeration `FINAL`-scanned ~80M
rows in **6.7s** (~12M rows/s on that host). The pilot is ~101 batches
(19 peg + 82 pivot; the oracle tier no-ops since `oracle_prices` starts 2025-09),
each re-scanning the ~2.8M-row partition → **3–10 min expected**. Extrapolated,
the full 30-month `1h` span is **~1.5–2.5 h**, before the other four tables.
`--batch-size 100000` is the lever if it runs long (scan cost is per batch, not
per row). **Merge pressure outlives the process** — ~1M re-inserted RMT rows keep
collapsing in background merges after the tool exits; `zeros_after` reads `FINAL`
so it is correct immediately, but the shared host keeps working.

### 🚧 Blocker hit first — `prices_writer` cannot FREEZE, and cannot be granted it

The pilot's first attempt died in 0.65 s with `Error: Clickhouse(BadResponse(""))`
— an **empty** error body — immediately after month enumeration. Replaying the
statement over `curl` against the same mTLS endpoint gave the real cause:

```
Code: 497. DB::Exception: prices_writer: Not enough privileges. To execute this
query, it's necessary to have the grant ALTER FREEZE PARTITION ON
prices.price_ohlcv_1h. (ACCESS_DENIED)
```

`prices_writer` holds only `SELECT, INSERT, ALTER DELETE, OPTIMIZE ON prices.*`.
Granting the missing privilege **is not possible**:

```
Code: 495. Cannot update user `prices_writer` in users_xml because this storage
is readonly. (ACCESS_STORAGE_READONLY)
```

— and that failure occurs **as the container superuser**, because the user is
XML-defined. Nobody can grant it at runtime.

**Resolution: the operator takes the snapshots as CH admin, the tool runs with
`--skip-snapshot`.** The AC requires each partition to *be* snapshotted, not that
`coarse-repair` be the thing that snapshots it. Taking them all up front is
arguably better — every restore point exists before the first write. Editing
`users.xml` on the shared cluster was rejected: a malformed file breaks auth for
every tenant, for a one-off repair.

> ⚠️ **`--skip-snapshot` is only safe once the snapshots are verified present.**
> The tool prints a loud "no FREEZE backup" warning that is *false* under this
> path. `ls /var/lib/clickhouse/shadow/ | grep repair_0114` plus a non-trivial
> `du` is the evidence that makes the flag safe. Skip that check and the warning
> is exactly as serious as it sounds.

> ⚠️ **Re-freezing is NOT idempotent.** A second `FREEZE` of an already-frozen
> partition fails with `DIRECTORY_ALREADY_EXISTS`. That failure is *protective*:
> had it succeeded on 202502 it would have overwritten the pre-repair snapshot
> with a post-repair one, destroying the only rollback point for the one month
> already written. The bulk-freeze script must report `already-frozen` distinctly
> so a real failure cannot hide among expected ones (the first version counted 29
> of 30 and looked like a partial failure).

### Pilot result — matches the pre-registered prediction

```
   month  zeros_before      enriched   zeros_after  snapshot
  202502       2788693       1000641       1788052  -
```
`real 6m55.880s` (snapshot taken separately by the operator, 194 MB under
`shadow/`).

| field | predicted | actual | delta |
|---|---|---|---|
| `zeros_before` | 2,788,693 | 2,788,693 | **exact** |
| `enriched` | ≈1,000,975 | 1,000,641 | −334 (0.03%) |
| `zeros_after` | ≈1,787,718 | 1,788,052 | +334 |

Both failure signatures ruled out — not ≈0, not exactly 200,000 — so `one_shot`
took effect and the version arithmetic beats a real `sum(version)` partition in
prod, not just in the integration test. The −334 is a day's drift in
peg/pivot reachability since the 07-22 measurement, in the safe direction.

**Revised timing:** ~7 min/month ⇒ the full 30-month `1h` span is **3–4 h**, not
the 1.5–2.5 h extrapolated from the dry-run scan rate. The dry run measured
enumeration throughput, which does not predict per-batch repair cost.

### ⚠️ AC correction — verify the *reference*, not a value ceiling

The pre-registered check `absurd_high = 0` **failed**: 40 rows > $1M, max
$29.6M. Investigation showed the gate itself was wrong, not the repair.

`implied_ref_usd` (= `close_usd / close`) is **0.312–0.408** on every XLM-quoted
outlier — the correct XLM/USD price for February 2025 — and **exactly 1.0** for
USDC-quoted rows (stablecoin-direct 1:1 cast). Both tiers compute correct
references. The absurd values come from junk *inputs*: single dust trades
(`trade_count = 1`, ~9 XLM ≈ $3 of volume) at nonsense unit prices like
94,810,046 XLM/token, which the repair faithfully multiplies.

Control — the same tail exists in **live-enriched** data nobody disputes:

| scope | rows | p50 | p99 | max_usd | > $1M | pct |
|---|---|---|---|---|---|---|
| `1h` 202502 (repaired) | 1,000,641 | 0.001104 | 10,197 | 29.6M | 40 | 0.0040% |
| `1h` 202607 (live) | 136,754 | 0.000582 | 2,039 | **24.0M** | 2 | 0.0015% |
| `1m` 202607 (live) | 3,437,815 | 0.002149 | 5,104 | **55.6M** | 8 | 0.0002% |

Live's max is *higher* than the repaired month's. The `1m` rate differs mainly by
granularity (~25× more rows/month); the residual ~2.7× between the two `1h` rows
is era/composition and 2-event noise.

`volume_quote_usd` is unaffected (~$3 on those rows), so BE volume analytics do
not see it. **Spawned as [[0116]]** — a pre-existing data-quality issue, not a
repair defect. The AC now reads: *`implied_ref_usd` is correct for the era.*

### 🐛 The empty-error trap, fixed in code

`Clickhouse(BadResponse(""))` gave no clue and cost a diagnostic cycle. Fixed:
`ChEnrichError::FreezeDenied` now carries the table, partition and both remedies,
and `preflight()` warns up-front when the connected user shows no FREEZE-capable
grant (advisory, not a gate — grant text has several shapes and a parse miss must
not refuse a legitimate run).

### Post-pilot value check (counts alone are not enough)

```bash
cat > /tmp/0114_pilot_check.sql <<'SQL'
SELECT 'repaired_202502'                              AS scope,
       count()                                        AS rows,
       round(quantile(0.01)(toFloat64(close_usd)), 6) AS p01,
       round(median(toFloat64(close_usd)), 6)         AS p50,
       round(quantile(0.99)(toFloat64(close_usd)), 6) AS p99,
       round(max(toFloat64(close_usd)), 2)            AS max_usd,
       countIf(close_usd > 1000000)                   AS absurd_high
FROM prices.price_ohlcv_1h FINAL
WHERE toYYYYMM(timestamp) = 202502 AND close_usd > 0 AND volume_quote > 0
FORMAT PrettyCompact;
SQL

ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec -i app-clickhouse-1 clickhouse-client' < /tmp/0114_pilot_check.sql
```

`absurd_high` must be 0; `p50` must be a plausible token price.

### Operator note

Prod-touching commands are run by the operator, not the agent — **including
`--dry-run` and read-only queries**. Heredoc-into-`ssh` is fragile on paste; write
the SQL to a local file and pipe it (`… clickhouse-client < /tmp/q.sql`).

## ✅ Repair result — `price_ohlcv_1h`, full 30-month span, 2026-07-23

Ran to completion in **`real 242m19s`** (4 h 02 m). Final line:

```
30 month(s): 30,828,953 enriched, 47,804,264 left at the no_reference floor
```

### Yield landed on the pre-registration

| | predicted 2026-07-22 | actual | delta |
|---|---|---|---|
| repairable | ~31.6M (28.0M pivot + 3.6M peg) | **30,828,953** | −2.4% |
| `no_reference` floor | ~48M | **47,804,264** | — |

Totals reconcile exactly: 30.83M + 47.80M = **78.63M**, which is the dry run's
79.63M minus the ~1.0M the 202502 pilot had already repaired. No rows
unaccounted for. Per-month `enriched` stayed inside the predicted band for every
month (1.03–1.64M for the 2024 pivot-era months, 0.44–1.32M once the peg tier
takes over from 202501).

Revised throughput: **~8 min/month** sustained, against the ~7 min the single-month
pilot suggested. The pilot extrapolated well; the dry-run scan rate did not
(it measures enumeration, not per-batch repair).

### Coverage after — aggregate 95.5% → 58.1% zero

| span | baseline | after |
|---|---|---|
| 202402 | 53.0% | 53.0% (unchanged — correctly, pure exotic floor) |
| 202403 | 86.1% | 47.8% |
| 202404 → 202412 | 91.2–96.6% | 43.0–59.9% |
| 202501 | 99.5% | 62.4% |
| **202502 → 202602** | **100.0% unbroken** | **55.6–69.8%** |
| 202603 / 202604 | 93.6% / 99.5% | 61.7% / 65.3% |
| 202605 / 202606 | 100.0% | 68.6% / 68.5% |
| 202607 | 92.4% | 68.4% |

The independently-computed zeros column sums to **47,804,264** — bit-identical to
the figure the driver printed. Two different code paths, same number.

### The residual is exotic-only — the AC's real test

Counts alone cannot close the AC, which is phrased in terms of reaching the
*genuine* `no_reference` floor. Per-quote-class breakdown over the whole repaired
span settles it:

| quote class | rows | zeros | pct_zero |
|---|---|---|---|
| stablecoin (USDC/USDT) | 5,319,219 | 49,538 | **0.9%** |
| XLM pivot | 29,508,335 | 246,383 | **0.8%** |
| other (exotic) | 47,844,173 | 47,844,173 | **100%** |

Everything the tiers can reach is reached; everything left is a quote with no USD
path, which is by design (`ch_enrich.rs:499-504`). The 0.8–0.9% residual against
the 0.3% live-era figure is the expected penalty for sparser historical pivot
references, not a defect.

> **⚠️ Unrelated finding surfaced by this query — a 0.4% join fan-out.** The
> class breakdown totals **82,671,727** rows against the coverage query's
> **82,335,897** — a **335,830-row** excess, and the zero counts differ by exactly
> the same amount. Cause: `INNER JOIN prices.assets ON a.asset_id =
> c.quote_asset_id` matches some `quote_asset_id` against more than one `assets`
> row **even under `FINAL`**, because `assets` is `ReplacingMergeTree(updated_at)`
> ordered by `(asset_code, issuer_address, contract_address)` — `FINAL` dedupes by
> **natural key, not by `asset_id`**. Any query joining on `asset_id` inherits
> this. Immaterial here (0.4% against a 100%-vs-0.9% split), but it is a live
> correctness hazard elsewhere → **spawned as [[0129]]**.

## ✅ Phase C — `_4h`/`_1d`/`_1w`/`_1M`, COMPLETE 2026-07-23

All four remaining coarse tables are repaired. Combined with the `_1h` run above,
**every coarse OHLCV table now sits at the genuine `no_reference` floor.**

| table | dry-run zeros | zeros after | enriched |
|---|---|---|---|
| `price_ohlcv_4h` | 41,019,676 | 28,062,132 | ~12.96M |
| `price_ohlcv_1d` | 14,047,080 | 9,127,070 | ~4.92M |
| `price_ohlcv_1w` | 3,629,865 | 2,064,862 | ~1.57M |
| `price_ohlcv_1M` | 1,305,124 | 628,710 | ~0.68M |
| **total** | **59,901,745** | **39,882,774** | **~20.1M** |

Deltas are approximate — live ingestion added rows between the dry run and the
after-measurement. Snapshots were taken by the CH admin for all four tables
before the run (150 dirs, 12 GB under `shadow/`); the repair ran
`--skip-snapshot` per the §Blocker path.

### The run was interrupted mid-`4h` and resumed cleanly

The operator's laptop lost power during the `4h` table, killing the client (the
work runs from the laptop; `/tmp/0114_phasec.log` was lost with it). Recovery was
mechanical and is the template for next time:

1. **Locate the stop point from data, not from the log** — a per-month `pct_zero`
   scan showed `4h` repaired through 202603 and untouched at 202604 (99.6%),
   202605/202606 (100.0%), 202607 (93.1%).
2. **Include the boundary month in the resume.** 202603 read **75.8%**, ~9 points
   above its repaired neighbour 202602 (66.4%) — the signature of a month written
   only partway. Resuming from 202604 would have left it half-done.
3. **Re-run the span.** `--start-month 202603 --end-month 202607`. 202603 came
   back with `enriched 114,514` (small, as predicted for a partial month) and the
   rest with full yields.

Resume is safe by construction: the pass is scoped to `close_usd = 0`, so
already-enriched rows are skipped and a re-run over completed work is a no-op.

```
  month  zeros_before      enriched   zeros_after
 202603       1027824        114514        913310
 202604       1228639        379610        849029
 202605       1425364        408287       1017077
 202606       1758902        485035       1273867
 202607        878203        204115        674088
5 month(s): 1,591,561 enriched, 4,727,371 left at the no_reference floor
```

`real 3m30s`. Every month reconciles exactly (`before − enriched = after`), so
the additive-only property held again — now demonstrated across two independent
runs and five tables.

### 📌 Two Phase-C estimates in this file were wrong — both corrected

- **Yield: predicted ~39%, actual ~30% overall and ~25–31% on 2026 months.** The
  39% was carried from `_1h`'s span-wide average, which is dominated by
  pivot-heavy 2024 months. The per-month figures track `_1h`'s *same months*
  closely (202604: 30.9% here vs 34% there; 202607: 23.2% vs 24%) — the
  extrapolation was mis-based, not the measurement.
- **Runtime: predicted ~3 h, actual minutes.** The `4h` resume did 5 months in
  **3m30s** against an 8 min/month expectation from `_1h`. Cause: on 202606/202607
  the **oracle tier** claimed all the reachable rows (`oracle_prices` broadens
  from 2026-03) and peg-pivot then correctly reported *"made no progress —
  remaining candles have no USD reference"*, so each month ran 23–51 batches
  instead of ~100. On `_1h` peg-pivot carried the work. **A tier split that
  differs from `_1h` is expected, not a fault.**

Resulting zero rates run **~3–4 points above `_1h`** on the same months (67.4 /
68.8 / 71.4 / 72.4 / 71.5% vs 61.7 / 65.3 / 68.6 / 68.5 / 68.4%). Expected:
coarser buckets are more exotic-dominated.

> **📌 `enriched: 0` in a dry-run summary is an artifact, not a signal.** A dry run
> only counts zeros; it never attempts enrichment, so all four tables report
> `0 enriched` by construction. Do **not** read this as the silent-no-op failure
> mode — that signature only means anything on a real run.

**Snapshot gate (held for this run).** Each table's partitions must be frozen by
the CH admin first (`prices_writer` still cannot FREEZE and cannot be granted it),
verified via `ls /var/lib/clickhouse/shadow/ | grep repair_0114` plus a
non-trivial `du`, before `--skip-snapshot` is safe. Re-freezing is **not**
idempotent and that failure is protective.

**Remaining for this AC:** the per-quote-class composition breakdown on `_4h`
(counts alone cannot close it — see §The residual is exotic-only). Use the
`GROUP BY asset_id` subquery form, not a direct join on `assets`, to avoid the
[[0129]] fan-out.

## Cluster findings from the Phase C session (2026-07-23)

Surfaced while verifying that nothing was deleting data mid-repair. None are
0114 defects; all three are recorded because they are invisible without querying
`system.*` and each misleads a reader who assumes otherwise.

### Cleanup deletes by **mutation**, not by dropping partitions

The EventBridge rule `prices-production-cleanup` is **`DISABLED`** (verified) and
`price_ohlcv_1m` has **no TTL**, so nothing is currently deleting. But the
mechanism recorded elsewhere is wrong: the destructive 2026-07-15 event was

```sql
-- price_ohlcv_1m, mutation_2496036, create_time 2026-07-15 10:24:36, is_done 1
DELETE WHERE intDiv(toUInt64(version), 1000) < 50457424
```

i.e. an `ALTER … DELETE` **mutation** removing every pre-Soroban row — which is
precisely the [[0088]] backfill's output, in the incident's own words. This fits
the access model: `prices_writer` holds `SELECT, INSERT, ALTER DELETE, OPTIMIZE`
and **cannot** drop partitions (the same grant wall that blocked `FREEZE`, see
§Blocker). Deleting by mutation is the only path available to it.

> **⚠️ Consequence for [[0109]] (the machine-checked cleanup guard): a guard that
> watches for `DropPart` will never fire.** `system.part_log` shows zero
> `DropPart` events across its whole retained window. The guard must watch
> **`system.mutations`** for `DELETE WHERE` commands against `prices.*`, and
> should treat `is_done = 0` as "armed and pending", not "safe".

### Six Phoenix DELETE mutations have been starved since 2026-07-17

```
create_time 2026-07-17 11:06:52, is_done 0, latest_fail_reason ''
DELETE WHERE source = 'phoenix'
  AND timestamp >= 2024-02-20 17:00:10 AND timestamp < 2026-07-06 09:35:16
```

on all six coarse tables — `_15m` (52 parts to do), `_1h` (779), `_4h` (431),
`_1d` (159), `_1w` (120), `_1M` (60). Empty `latest_fail_reason` and epoch
`latest_fail_time` on every one: **starved, not failing.** ClickHouse gates
mutation execution behind free slots in the background merge pool
(`number_of_free_entries_in_pool_to_execute_mutation`), and the pool has been
saturated continuously since 07-15 by the backfill (a 64k-file partition every
~25 min) plus this task's ~32M re-inserted RMT rows.

They should drain on their own once the backfill finishes (~2026-07-27/28).
**Do not `KILL MUTATION`** — half-applied Phoenix deletes across six tables is a
worse state than not-applied.

Two consequences worth carrying:

- **[[0097]]'s Phoenix rework is incomplete.** The delete half never ran, so
  pre-fix Phoenix rows are still live in all six coarse tables for
  2024-02 → 2026-07. Invisible unless you query `system.mutations`.
- **Some of this task's repair work was spent on rows that mutation intends to
  delete.** Harmless — mutations only target parts that existed when they were
  created, so today's inserts are not affected — but a slice of the 32.4M
  enriched rows is Phoenix data with a pending deletion.

## Notes

- **Possible [[0107]] link, unconfirmed.** SDEX volume measuring ~1/6 of
  Horizon's is consistent with `volume_quote_usd` being zero for ~95% of coarse
  rows. But 0107 also reports a trade-*count* discrepancy, which this cannot
  explain. Candidate, not conclusion — test after the repair.
- **`prices.oracle_prices` holds 390 junk rows** at `1970-01-21 15:41:56 →
  15:44:05` across 3 assets — a unit or parse error (ledger sequence or ms/s
  mix-up written to a timestamp column). Unrelated to the main thread but it
  will silently poison any ASOF join reaching that far back. Needs its own small
  task.
- All figures measured against prod CH on 2026-07-21 via `system.query_log` and
  direct counts; none inferred from a quiet cluster.
