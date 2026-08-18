---
id: "0201"
title: "32M pre-Soroban coarse candles carry close_usd = 0 — the 0088 backfill wrote them, the 0114 repair started three years too late"
type: BUG
status: backlog
assignee: okarcz
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
  - date: 2026-08-18
    status: backlog
    who: okarcz
    note: >
      THE WORK IS DONE — as pass 1 of 0182's route (c) campaign, on all five
      forever-tables. 53,965,024 rows recovered: _1h 33,073,568, _4h 13,868,017,
      _1d 4,930,151, _1w 1,521,267, _1M 572,021. Each table's total reconciles
      with that morning's dry-run figure to within its own reset population,
      which is four independent confirmations. ⚠️ THE TITLE UNDERSTATES IT — "32M"
      was _1h alone; the campaign was two-thirds larger. Status left at backlog
      deliberately: the data outcome is complete but this file still describes
      the work as pending, and closing it is the assignee's call. What a closure
      needs to state: the recoverable window is 2022-04 -> 2024-01, not
      2021-02 -> 2024-01, and everything below 2022-04 is exotic-quoted floor.
  - date: 2026-08-18
    status: backlog
    who: okarcz
    note: >
      RE-MEASURED during 0182's pass 1 — the classification below is wrong about
      the SPAN. It attributes ~32M fillable XLM-quoted rows to 2021-02..2024-01;
      almost none are below 2022-04. Every month from 202110 to 202203 ran with
      `enriched 0` and the "no USD reference (exotic quotes)" warning, then
      202204 enriched 761,735 of 1,531,768 in 79 batches (~67 s). The pre-2022-04
      candidates are exotic-quoted (0-13 XLM-quoted per month), so they are the
      permanent no_reference floor, not missing data. Two plausible explanations
      were falsified on the way and are recorded so they are not re-opened: the
      XLM/USDC reference DOES reach back to 2021-02-01, and its early candles do
      NOT have zero volume. The count may still be near 32M — take it from pass
      1's summary, not from the classification.
  - date: 2026-08-13
    status: backlog
    who: okarcz
    note: >
      Assigned to okarcz, who is solving it personally. Stays in backlog until
      they promote it themselves — it is not to be started, drafted or folded
      into a 0182 run by anyone else.
---

# 32M pre-Soroban coarse candles were never enriched

## 🙋 Owner: okarcz — hands off

Assigned 2026-08-13. **The operator is solving this one personally.** Stays in
`backlog` until they promote it themselves.

Do not start it, do not draft the run, do not open a branch or PR for it, and do
not fold it into a [[0182]] run — not even the parts that look mechanical. Answer
questions about it and surface anything that changes its shape (a new
measurement, a conflicting task), but the work itself is not to be picked up
unless the operator asks in that message.

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

## ✅ RECOVERED 2026-08-18 — 53,965,024 rows, as pass 1 of 0182's campaign

| table | enriched | left at the no_reference floor |
|---|---|---|
| `price_ohlcv_1h` | 33,073,568 | 74,282,984 |
| `price_ohlcv_4h` | 13,868,017 | 39,595,149 |
| `price_ohlcv_1d` | 4,930,151 | 12,981,344 |
| `price_ohlcv_1w` | 1,521,267 | 2,873,278 |
| `price_ohlcv_1M` | 572,021 | 890,603 |
| **total** | **53,965,024** | |

Every table's `enriched + floor` falls short of that morning's dry-run total by
almost exactly its own USDT reset population (`_1h` 354,803 vs 357,002 expected ·
`_4h` 154,685 vs 157,683 · `_1d` 39,882 vs 41,505 · `_1w` 8,233 vs 8,253 · `_1M`
2,767 vs 2,789). Four independent confirmations that the two passes saw the
populations they were supposed to, from a direction neither pass measures
directly.

⚠️ **The title understates this task.** "32M" was `price_ohlcv_1h` alone — the
only table the 2026-08-13 classification broke down by class. The real campaign
was two-thirds larger.

Run cost ~4 h across both passes and all five tables, against a 10-15 h estimate
that assumed the rows were spread over 36 months. They were concentrated in about
20 — see below.

## ⚠️ Re-measured 2026-08-18 during 0182's pass 1 — the SPAN above is wrong

The classification is right that ~32M rows are XLM-quoted and fillable. It is
wrong about **when** they are, and not by a little: it attributes them to
`2021-02..2024-01`, and almost none of them are below **2022-04**.

Candidates (`close_usd = 0 AND volume_quote > 0`) on `price_ohlcv_1h`, broken
down by quote leg:

| month | candidates | XLM-quoted | USDC-quoted | USDT-quoted |
|---|---|---|---|---|
| 202110 | 23,453 | 13 | 7 | 96 |
| 202111 | 374,704 | 2 | 1 | 37 |
| 202112 | 644,682 | 0 | 0 | 62 |
| 202201 | 670,389 | 4 | 1 | 43 |
| 202202 | 519,720 | 3 | 1 | 84 |
| 202203 | 637,918 | 4 | 129 | 176 |
| 202204 | 775,756 | 20 | 154 | 79 |
| 202205 | 951,141 | 6 | 209 | 25 |
| 202206 | 931,338 | 4 | 315 | 56 |

⚠️ **202204 onward is post-drain** — pass 1 had already filled those months when
this ran, which is why 202204 reads 20 XLM-quoted rather than the ~761k that
went into it. The pre-2022-04 months are untouched, and they are simply not
XLM-quoted.

The run agrees. Every month from 202110 to 202203: `enriched 0`, with *"peg-pivot
tier made no progress — remaining candles have no USD reference (exotic
quotes)"*. Then **202204: 761,735 enriched of 1,531,768, 79 batches, ~67 s.** The
boundary is sharp, and it is a property of the data rather than of the tool.

**Scope consequence:** the recoverable window is roughly **2022-04 → 2024-01**,
not 2021-02 → 2024-01. Everything below 2022-04 is exotic-quoted and belongs to
the permanent `no_reference` floor. The total may still be near 32M — this
measurement settles the *span*, not the count. **Take the count from pass 1's own
summary, never from the classification.**

⚠️ Verified for **202110-202203**. Months 202102-202109 had already scrolled past
in the run's output when this was measured, but their candidate counts are of the
same small order (202110 is 23,453).

### Two hypotheses this falsified — do not re-open them

Both were plausible, both are wrong, and each cost an hour:

1. **"The XLM/USDC reference series does not reach back that far."** It does —
   first candle `2021-02-01 21:00`, 149,629 candles to date.
2. **"The early reference candles carry `volume_base = 0`, so the pivot's
   `sum(close × volume_base) / nullIf(sum(volume_base), 0)` returns NULL."** They
   do not. Every month 202102-202207 holds ~720 candles — complete hourly
   coverage — **all** with volume, and a non-NULL `usd_ref`: XLM 0.434 → 0.110,
   USDT 1.010 → 0.350 across the depeg.

### What this settles for [[0182]]

The same class of assumption underpins 0182's reset epoch `1612656000`
(2021-02-07). The USDT/USDC reference proved **dense and non-NULL from 202102** —
first candle `2021-02-07 19:00`, matching the epoch exactly, 276 candles that
month and ~700 thereafter. So pass 2 cannot zero rows into a reference hole.
That risk is closed.

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
