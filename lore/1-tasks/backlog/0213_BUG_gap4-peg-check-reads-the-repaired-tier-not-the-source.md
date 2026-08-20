---
id: "0213"
title: "The USD peg check reads _1h — the tier 0182 repaired — so it publishes 0 over 1.5M wrong _1m rows"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0204", "0212", "0209", "0182", "0172"]
tags: ["priority-medium", "effort-small", "observability", "clickhouse", "data-correctness", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/rollup-freshness-probe/src/usd_sanity.rs"
  - "../../../infra/src/lib/stacks/observability-stack.ts"
history:
  - date: 2026-08-20
    status: backlog
    who: okarcz
    note: >
      Spawned from 0204 gap 4 during the pre-deploy prod baseline. The
      peg-applied ladder reads price_ohlcv_1h, which 0182's repair wrote, while
      the peg values live in price_ohlcv_1m, which that repair never touched —
      1,564,045 of them. The check would publish a confident 0. The stranded
      direction is unaffected and is what found 0209.
---

# The peg check reads the repaired tier, not the source

## Summary

[[0204]] gap 4 alarms on two directions. The **stranded** one works: a zero in
`_1m` rolls up as a zero in `_1h`, so reading the coarse tier detects it — that
is how [[0209]] was found. The **peg-applied** one does not, because a *repaired*
value in `_1h` says nothing about the row it was rolled from.

Measured on prod 2026-08-20:

| table | USDT-quoted rows at `close_usd / close ≈ 1.0` |
|---|---|
| `price_ohlcv_1h` — what the check reads | **0** |
| `price_ohlcv_1m` — where enrichment writes | **1,564,045** |

[[0182]]'s repair wrote the five coarse tiers directly and never touched `_1m`
([[0212]]). So the alarm reads clean over 1.5 M wrong values and would have gone
on doing so indefinitely.

⚠️ **This is the task's own founding failure, reproduced inside the guard built
against it** — a check scoring healthy because it looked at the surface least
able to show the defect.

## Why the obvious fix is wrong

⛔ **Do not simply repoint `SANITY_TABLE` at `price_ohlcv_1m`.**

1. It would read 1,564,045 immediately — above every rung of
   `usdSanityEscalationCounts` (`[1, 100, 10000]`) — and sit permanently in
   ALARM. A permanently-firing alarm gets muted, which is the exact end-state
   [[0204]] exists to prevent.
2. `_1m` is **retention-managed at 7 days** while `_1h` is a forever-table. The
   check's 7-day `LOOKBACK_SECONDS` sits exactly on that boundary, so the window
   reasoning has to be redone rather than inherited.
3. The stranded direction is *correct* on `_1h` and would be made worse by moving
   — the 48 h grace is calibrated to BE's loss window on the hourly tier.

## Implementation

- Split the two directions: keep `stranded` on `_1h` unchanged; give
  `peg_applied` its own `_1m`-scoped query, window and ladder.
- ⚠️ Sequence it **after [[0212]]** has repaired the 1.5 M rows, or the new
  ladder ships permanently breached — which is the muting failure above.
- Re-verify the scan cost by what it **reads**, not what it returns; `_1m` at
  7 days on one quote leg is a different shape from `_1h`.
- The IT already writes a par-valued candle into a real ClickHouse
  (`usd_sanity_counts_both_induced_defects`); extend it to `_1m` so the tier
  distinction is induced rather than reasoned about.

## Acceptance Criteria

- [ ] The peg-applied metric is computed from **`price_ohlcv_1m`**, and an IT
      proves it counts a par-valued `_1m` row that no coarse tier carries.
- [ ] The stranded metric still reads `_1h` and its 48 h grace still means
      BE's loss window.
- [ ] The ladder reads **0** on prod at deploy time — i.e. [[0212]] landed first.
- [ ] A note records why `_1m`'s 7-day retention does not undermine the lookback.
