---
id: "0114"
title: "Coarse OHLCV tables carry no USD values for 2025-02 → 2026-02; enrichment never revisits rolled-up rows"
type: BUG
status: backlog
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

`3_other` is 5.1M rows/month, permanently zero, **by design**. Whether that is
acceptable is a product question nobody has asked. A **two-hop pivot** (exotic →
XLM → USD) would likely reach a large share, since most Stellar assets trade
against XLM. This is plausibly a bigger coverage lever than the historical
repair, and it needs its own task.

**Strong [[0107]] candidate.** If 62% of candles carry zero USD volume, any USD
volume total is missing the majority of trades — against a measured gap of ~1/6
versus Horizon. Not yet checked for volume-weighting; test before believing it.

## Acceptance Criteria

- [ ] Coarse tables re-enriched for 2024-02 → present; per-month `close_usd = 0`
      rate drops to the genuine `no_reference` floor (exotic quotes only), not
      86–100%.
- [x] **Pre-check before any implementation** — DONE 2026-07-21. The 62% is
      genuine `no_reference` (exotic quotes), not backlog; live enrichment runs
      at 99.7% on pivotable quotes. Approach holds for the historical repair.
- [ ] The rollup path no longer freezes un-enriched values — implemented as a
      **scheduled, partition-bounded enrichment pass over the coarse tables**
      (see §Chosen approach; the ordering-based alternatives were rejected).
- [ ] **Version arithmetic proven by test:** a repair row outranks the existing
      coarse row under `sum(version)` RMT semantics. This fails silently, so a
      passing test is the only acceptable evidence — not a manual spot-check.
- [ ] `price_ohlcv_1h` / `1d` USD coverage verified for a sample of liquid pairs
      against an independent source before declaring the repair good.
- [ ] 0088 step 3 gated: pre-roll refuses to run, or warns loudly, when 1m USD
      coverage for the target span is below a threshold.
- [ ] Re-check whether this explains [[0107]]'s volume gap (see below).

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
