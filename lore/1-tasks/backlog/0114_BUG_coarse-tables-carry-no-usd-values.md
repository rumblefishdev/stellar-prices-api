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

## 🔴 Time-sensitive: gate 0088's recovery pre-roll

[[0088]] recovery **step 3** runs `preroll-incremental.sql` over the pre-Soroban
tail in ~10 days (~2026-08-01). Defect 1 means it will bake whatever coverage
`price_ohlcv_1m` has at that moment into the coarse tables — currently **62%
zero** — for the data 0088 spent two weeks rebuilding. There is no coverage
check anywhere in that runbook.

**Either** enrich the 1m tail before pre-rolling, **or** accept the gap and plan
a coarse-table repair pass after. Not deciding is the one option that silently
loses it.

## Acceptance Criteria

- [ ] Coarse tables re-enriched for 2024-02 → present; per-month `close_usd = 0`
      rate drops to the genuine `no_reference` floor (exotic quotes only), not
      86–100%.
- [ ] The rollup path no longer freezes un-enriched values — either enrichment
      runs before rollup, or a repair pass runs after, or the coarse tables are
      enriched in place on a schedule. Pick one and make it structural.
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
