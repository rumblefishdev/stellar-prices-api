---
id: "0182"
title: "44,657 stored candles across 495 assets carry a close_usd ~7.4x too high — the USDT peg fix stops new ones but does not correct history"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0172", "0196", "0165", "0145", "0111", "0114"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "enrichment", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-12
    status: backlog
    who: okarcz
    note: >
      Spawned from 0172. That task fixed the WRITER (USDT moved from the peg
      tier to the pivot tier, so new candles are priced at the measured rate),
      but every close_usd already written through the old peg path is still on
      disk and still ~7.4x too high. Filed separately because correcting it is a
      re-enrichment run with its own risk profile, not a code change.
---

# `close_usd` is ~7.4× too high on every USDT-quoted candle ever written

## Measured on prod (2026-08-12)

```
price_ohlcv_1d WHERE quote_asset_id = 111 (canonical USDT):
  candles              44,657
  distinct base assets    495
  span                 2018-05-15 -> 2026-08-12   (i.e. still being written)
  priced (close_usd>0) 44,653
  implied USDT rate     0.999999   <- every one valued at par
```

The correct rate is USDT's measured market value — ~$0.13 in 2026-08, and
varying over time (see [[0172]] for the full monthly series). So each affected
`close_usd` is overstated by roughly `1 / 0.13 ≈ 7.4×`, with the exact factor
depending on the bucket's date.

## Why this is not fixed by 0172

0172 changed how candles are enriched **going forward**: `stable_ids()` no longer
contains USDT, and a second pivot pass prices USDT-quoted candles from the
measured USDT/USDC market. But enrichment only writes rows where
`close_usd = 0` — already-enriched rows are deliberately skipped (that filter is
what makes the pass idempotent and restartable). So the 44,657 wrong values are
inert: nothing will revisit them.

## 🛑 TWO ORDERING CONSTRAINTS — violating either makes this task destructive

Both surfaced in the 2026-08-13 review of 0172's PR #205. Neither is optional,
and neither is visible from inside this task's own plan.

### 1. `oracle_prices` must be purged FIRST ([[0196]])

The enrichment **oracle tier runs before the peg-pivot tier and wins where it
applies** (`ch_enrich.rs:19-22`). [[0196]] measured **46,378 mis-attributed
Reflector rows** on the USDT identity in `prices.oracle_prices`, covering
2026-03 → present and current to the hour.

So zeroing the rows and re-enriching *while those exist* re-applies
`close_usd = close × ~$1.00` to every 2026-03 → 2026-08 USDT-quoted candle and
labels it `method = 'oracle'` — a consumer reads that as **more** authoritative
than the peg placeholder it replaced. Same failure mode already recorded for
[[0168]].

### 2. Pre-2021 candles have NO pivot reference — do not zero them

The USDT/USDC market begins **2021-02-07** (2,011 daily candles). USDT-quoted
candles begin **2018-05-15**. The pivot's `ASOF LEFT JOIN` + `AND r.usd IS NOT
NULL` drops any candle with no reference at or before its bucket, so
**2018-05-15 → 2021-02-07 has nothing to pivot on**.

Those rows currently hold `close × $1` from the old peg path. Zero them and they
stay at `close_usd = 0` permanently — the ambiguous zero read unguarded by ~130
`argMax(close_usd, …)` sites ([[0145]]), which 0172's own rationale argues is
*worse* than a wrong-but-visible number.

⚠️ **And the old `$1` is CORRECT for that window.** 0172 measured USDT at par
from 2021-02 until the June 2022 break, and the depeg is what makes the peg
wrong *after* it — not before. So this is not "wrong data we cannot fix", it is
**right data this task would destroy**.

Options: bound the re-enrichment to ≥ 2021-02-07; or give the pivot a dated peg
epoch (par before the break, measured after). Decide before writing the driver.

⚠️ Also unresolved: `volume_quote_usd` is preserved write-once
(`if(p.volume_quote_usd > 0, …)`), so a row whose `close_usd` this task corrects
keeps a `volume_quote_usd` computed at $1 — the same row then carries two USD
columns that disagree by 7.4×. This task's scope is `close_usd` only; either
widen it or spawn a follow-up.

## What needs deciding

- **Scope.** All six granularities, or `_1m` + a coarse re-roll? The wrong values
  are in the rolled tables too, and the rollup MVs are `sum(version)`-based
  (see [[0095]], [[0136]]) — a naive re-insert into `_1m` does not propagate.
- **Mechanism.** The task 0114 coarse-repair driver already exists for exactly
  this shape of problem (bounded per-month re-enrichment with FREEZE snapshots)
  and should be evaluated before anything bespoke is written.
- **Zeroing first.** Enrichment skips `close_usd > 0`, so the rows must be reset
  to 0 (or written with a higher `version`) before the corrected pivot can fill
  them. On a `ReplacingMergeTree(version)` the version route is safer than a
  mutation; confirm against the [[0097]] pre-roll notes (RMT ties need
  DELETE-first).

## Blast radius — read this before prioritising

⚠️ **Volume weight is NOT the impact measure.** Measured over 2026-05-01+, the
assets that actually *depend* on the USDT leg for their USD price are tiny:
`SPCXLM` ($12 total volume, 100% via USDT), `SCOP`/`RCC` ($0, 100%), `BAT`
(90.9%, $652), `LINK` (86.1%, $2,487), `BTC`-anchor (59.6%, $3,090), `MXN`
(36.8%, $4,022), `GYEN` (17.2%, $9,520). Everything with real volume — XLM
($470M), AQUA ($14.7M), SHX ($12.5M), XRP ($12.3M), yXLM ($10.3M) — draws ~0%
through the USDT leg, so **published prices for the major assets are unaffected**
(XLM measured at 0.0078% of weight).

**But BE values holdings, not flow.** A pool can hold a large position in an
asset that barely trades. 0172's opening recorded **106 pools with a USDT leg,
102 priceable** — every one of those positions is valued ~7.4× high regardless of
how thin the trading is. Prioritise on that, not on the volume table above.

## Acceptance Criteria

- [ ] Decision recorded: correct history, or leave it and document the epoch
      boundary for consumers
- [ ] If correcting: all six granularities consistent afterwards, verified by
      re-running the `implied rate` probe (should move from ~1.0 to the measured
      per-bucket USDT rate)
- [ ] Guard against re-introduction: the [[0172]] regression tests already pin
      the writer; add a data-level check that no USDT-quoted candle carries
      `close_usd / close ≈ 1.0`
- [ ] BE notified of the corrected values and the window affected
