---
id: "0182"
title: "44,657 stored candles across 495 assets carry a close_usd ~7.4x too high — the USDT peg fix stops new ones but does not correct history"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0172", "0165", "0145", "0111", "0114"]
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
