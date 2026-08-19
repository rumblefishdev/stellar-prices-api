---
id: "0209"
title: "USDT-quoted candles sit at close_usd = 0 for 17 h+ after the hourly sweep should have priced them"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0182", "0172", "0165", "0145"]
tags: ["priority-medium", "effort-small", "clickhouse", "enrichment", "data-correctness", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-19
    status: backlog
    who: okarcz
    note: >
      Spawned from 0182. Its post-repair damage check found 82 USDT-quoted rows
      still at close_usd = 0 across four tiers. All of them postdate the repair
      and none are damage from it, but the oldest is 17 h+ old, which is longer
      than an hourly sweep should leave a candle unpriced. Never investigated —
      0182 closed on the repair being verified, not on this.
---

# USDT-quoted candles stay unpriced far longer than the sweep interval

## Summary

Measured 2026-08-19 while verifying [[0182]]'s repair. 82 USDT-quoted candles
(`quote_asset_id = 111`) sit at `close_usd = 0` with a real `close` and real
volume, across four granularities:

| tier | rows | window |
|---|---|---|
| `_1h` | 38 | `2026-08-18 14:00` → `2026-08-19 07:00`, 9 assets |
| `_4h` | 22 | same window |
| `_1d` | 14 | yesterday + today |
| `_1w` | 8 | the current week bucket, `2026-08-17 00:00` |

These are **not** [[0182]]'s stranded rows — that defect was at the 2021-02-07
epoch boundary and is repaired, `at_boundary` 0 on every tier. These postdate the
run entirely. They are also **not dust**: `close` ranges from `0.045` to `524.78`
with ordinary `volume_quote`.

The problem is only the age. An in-flight bucket at zero is expected; a bucket
from **17 hours ago** is not, if an hourly sweep is running.

## Two candidate causes, neither measured

1. **The sweep is not keeping up, or is not running.** Would show as a lag
   affecting all quote legs, not just USDT.
2. **The pivot cannot find a USDT/USDC reference inside its window.** Live
   enrichment prices USDT-quoted candles by pivoting off the measured USDT/USDC
   market ([[0172]]). The default `--pivot-window-s` is **1 day**; if that market
   is thin enough that a bucket has no reference at-or-before within the window,
   the `ASOF LEFT JOIN` + `AND r.usd IS NOT NULL` drops the row and it stays at
   zero until a later trade rescues it. Would show as USDT-specific.

⚠️ **Distinguishing them is the first task, and it is one query** — compare the
unpriced-row age distribution for `quote_asset_id = 111` against XLM-quoted and
USDC-quoted legs over the same window. If only USDT lags, it is the reference
window; if everything lags, it is the sweep.

## Why it matters despite being small

BE's pool-list TVL takes the last `close_usd > 0` **within 48 hours** and renders
`--` when there is none ([[0182]], BE answers 2026-08-13). 82 rows is nothing, but
a 17-hour hole consumes a third of that margin, and the failure mode is silent on
our side — a zero is indistinguishable from "not yet written" at every one of the
~130 unguarded `argMax(close_usd, …)` sites ([[0145]]).

If cause 2 is the answer, the hole grows with USDT market thinness rather than
staying bounded, which is the shape that eventually crosses 48 h.

## Implementation

- Measure first: age distribution of `close_usd = 0 AND close > 5e-14` rows by
  quote leg, over the last 7 days, on `_1h`.
- If USDT-specific → the reference window is the lever. Decide whether the live
  pass should widen `pivot_window_s` for this leg, or whether a stale reference
  is *better* than no value here (⚠️ that is the [[0165]] peg-fallback argument
  and it was settled against a fixed peg, not against a stale measurement — do
  not reopen it as "just use $1").
- If leg-agnostic → the sweep's schedule or batch budget is the lever, and this
  becomes an [[0111]]-adjacent throughput question.

## Acceptance Criteria

- [ ] The cause is **measured**, not inferred — USDT-specific or leg-agnostic,
      with the comparison query recorded.
- [ ] Whichever it is, the fix is verified by the age distribution moving, not by
      the row count on one day.
- [ ] If the pivot window is widened, a note records why a stale measured
      reference is acceptable where a $1 peg was not.
