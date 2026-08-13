---
id: "0201"
title: "32M pre-Soroban coarse candles carry close_usd = 0 — the 0088 backfill wrote them, the 0114 repair started three years too late"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0088", "0114", "0182", "0145", "0111"]
tags:
  ["priority-medium", "effort-large", "clickhouse", "data-correctness", "enrichment", "backfill", "operational"]
milestone: 2
links:
  - "../../../docs/runbooks/repair-coarse-usd-values.md"
history:
  - date: 2026-08-13
    status: backlog
    who: okarcz
    note: >
      Spawned from 0182's dry run. Measuring what the reset command would touch
      showed that 99.7% of it is not USDT at all: 31,982,165 fillable XLM-quoted
      rows on price_ohlcv_1h alone, all below 2024-02, all at close_usd = 0.
      That is the 0088 pre-Soroban backfill's output, which no enrichment pass
      has ever covered — 0114's repair span began at 2024-02. Filed separately
      because it is a campaign the size of 0114 itself, and absorbing it into a
      567k-row USDT correction would have been an unrecorded 10-15 h prod run.
---

# 32M pre-Soroban coarse candles were never enriched

## Summary

The [[0088]] backfill recovered pre-Soroban SDEX history and rolled it into the
coarse forever-tables. Those rows landed with `close_usd = 0`, and nothing has
ever gone back to price them: the [[0114]] repair — the tool that exists for
exactly this — was run over **2024-02 → 2026-07**, and this data is *older* than
its start month. So it has sat at zero since the backfill completed.

They are **fillable**, not exotic: the quote leg is XLM, which the pivot tier
prices from XLM's own USDC market. This is missing data with a known recipe, not
a floor.

## Measured on prod 2026-08-13 (`price_ohlcv_1h`, `202102`–`202608`)

Candidate rows, i.e. `(volume_quote_usd = 0 OR close_usd = 0) AND volume_quote > 0`:

| class | 2021-02..2024-01 (0114 never ran) | 2024-02.. (0114 ran) |
|---|---|---|
| **XLM pivot → fillable** | **31,982,165** | 1,100,131 |
| exotic → stays 0 | 24,056,634 | 50,020,562 |
| USDC peg → fillable | 196 | 403 |
| USDT → fillable | 8 | 31 |

Only `_1h` was broken down by class. The other four tables' totals from the same
dry run — these mix fillable and exotic, so they are upper bounds, not the work:
`_4h` 53,506,597 · `_1d` 17,907,401 · `_1w` 4,391,892 · `_1M` 1,463,985.

⚠️ **Re-measure per table before running.** Extrapolating `_1h`'s ratio across
the other four is exactly the kind of inference this project has been bitten by.

⚠️ **The 1,100,131 fillable rows inside 0114's own span want explaining before
the run, not after.** 0114 reported those months drained. Candidates: 202608
post-dates its end month; the 0088 pre-roll wrote coarse rows after it ran; or
its `one_shot` drain left more behind than the summary implied. Whichever it is,
it changes what "done" looks like here.

## Why it was invisible

Nothing lies about this — `close_usd = 0` is the correct encoding for "not
priced". It is the [[0145]] ambiguity: **a zero reads as a real price at ~130
unguarded `argMax(close_usd, …)` sites**, and reads as absent to BE (who filter
`close_usd > 0`). So the symptom is silence on our side and a `--` on theirs,
for three years of history nobody had queried deeply enough to notice.

## Implementation

The tool already exists and needs no change — this is [[0114]]'s
`coarse-repair` over an earlier span, with **no** `--reset-*` flags (nothing here
holds a wrong value; every target is a zero, so the run is purely additive).

- Runbook: `docs/runbooks/repair-coarse-usd-values.md`, Steps 1-7 unmodified.
- Span `--start-month 202102 --end-month 202401`, five forever-tables, one at a
  time. Cross-check the upper bound against 0114's own span rather than
  overlapping it.
- ⚠️ **Raise `--pivot-window-s`** for `_1w`/`_1M` — the 1-day default drops a
  reference sitting in the previous bucket. The tool refuses below the bucket
  width only in reset mode; here it would just silently under-fill.
- Snapshots: `prices_writer` cannot `FREEZE` and cannot be granted it. CH admin
  freezes out of band, tool runs `--skip-snapshot` (runbook Step 3b).
- Sizing anchor: 0114 measured **~7 min/month** on `_1h` at ~2.8M candidates and
  ~1M enriched. 36 months × 5 tables at that rate is **10-15 h**, so plan it as a
  multi-session campaign in low-traffic windows, not an afternoon.

## Ordering against [[0182]]

They overlap and the order is a real decision, recorded in 0182:

- The enrichment tiers take **no per-quote-leg filter**, so any month 0182 visits
  gets this work done too. There is currently no way to run one without the other.
- Running **0201 first** leaves each table at its `no_reference` floor, after
  which 0182's reset run is naturally bounded and its `rows_reset ≈ rows_enriched`
  safety check becomes meaningful again — it is swamped in a combined run.
- ⚠️ Whatever the order, do **not** re-run reset mode over a table twice: it is
  not a fixed point across invocations.

## Acceptance Criteria

- [ ] Per-table, per-class measurement recorded before the run (not extrapolated
      from `_1h`)
- [ ] The 1,100,131 fillable rows inside 0114's completed span explained
- [ ] Five forever-tables (`1h/4h/1d/1w/1M`) drained to their exotic-only floor
      for 2021-02 → 2024-01; `_1m`/`_15m` excluded (retention-bounded)
- [ ] Spot-check: implied reference for XLM-quoted rows matches the real XLM/USD
      price for that month (0114 Step 5b — check the *reference*, never a value
      ceiling)
- [ ] Snapshots verified before any write, and released afterwards
- [ ] BE told which historical window became priceable — their 30D/1Y pool charts
      are the consumer that reads history
