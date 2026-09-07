---
title: "Phase 0 — the $1.00 is ours, not a provider's: where the peg is asserted and which assets it touches"
type: synthesis
status: developing
spawned_from: ../README.md
spawns: []
tags: [pricing, stablecoin, data-correctness, root-cause]
links:
  - "../../../../../packages/prices-api/src/assets/handlers.rs"
  - "../../../../../packages/prices-api/src/assets/queries_ch.rs"
  - "../../../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-09-04
    status: seed
    who: akot
    note: >
      Root cause located in code (facts from the repo). Numeric proof of the
      synthetic profile and the asset sweep are in analysis/audit_our_series.py
      and land here once run against the deployed API.
---

# Phase 0 — root cause and blast radius

## 1. Where the $1.00 comes from (facts from the code, verified 2026-09-04)

**No upstream provider returns 1.0. Our own read path asserts it.**

The `/ohlcv` handler decides per request whether the asset is *the* peg asset:

- `packages/prices-api/src/assets/handlers.rs:643-648` — `is_peg_asset` is true
  only when the requested identity equals canonical USDC
  (`GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN`, by id or by
  canonical string). Every other asset, pegged or not, goes through the
  ordinary candle query.
- For USDC it calls `queries_ch::ohlcv_peg_series`
  (`queries_ch.rs:809`). That function builds one bucket per XLM/USDC candle
  timestamp and joins `usd_rate` restricted to `method = 'oracle'`
  (`queries_ch.rs:952-957`). Where the join finds nothing usable it inserts a
  **literal** `toDecimal128(1, 14)` as O=H=L=C=VWAP, `'0'` volume, `0`
  trade_count, `method = 'peg'`, `derived = 1` (`queries_ch.rs:930-947`).
  The sentinel is `no_rate = (ifNull(r.meth,'') = '' OR r.rts < floor)`
  (`queries_ch.rs:926`).
- `usd_rate` with `method = 'oracle'` starts **2026-03-11 14:00** (measured by
  [[0247]], not by us). So every daily bucket from 2021-02-01 to 2026-03-10
  takes the literal branch. This is the whole "2021 through 2025 is 100% peg"
  observation in the task summary.

The same rule is mirrored in the ClickHouse view `price_usd_series`
(`views.sql:362`, `:570`): `if(max(peg_rate) > 0, 'oracle', 'peg')` with the
value defaulting to 1. The view is not what `/ohlcv` reads, but it is the
second place the assertion lives, and the [[0247]] backfill path targets it.

**Why there is no candle to measure from.** USDC is the top-preference quote
asset; canonicalisation always writes pairs as X/USDC, never USDC/X, so
`price_ohlcv_1d WHERE asset_id = USDC` is empty ([[0165]], [[0247]]: 0 rows).
The only USDC-denominated numbers we hold are *other* assets' prices in USDC.

**Why those cannot be inverted.** Before 2026-03-11, `close_usd` on every
USDC-quoted candle was produced by `enrichment-worker::ch_enrich::pivot_sql`,
which defines XLM's USD price as XLM's USDC price, i.e. `USDC = $1` is the
input. [[0247]] measured the implied rate `close_usd / close` over all
654,291 pre-oracle USDC-quoted candles: exactly one distinct value, 1.0. So
"derive USDC/USD from XLM/USDC and XLM/USD" returns the assumption, relabelled.
The circularity in the task's Context section is real and is the reason a fix
needs an *external* anchor.

## 2. Who else is affected (from the code; the numeric sweep in §3 confirms it)

Two different defects hide under "stablecoin reads 1.00", and they have
different blast radii:

| defect | mechanism | assets hit | visible as |
|---|---|---|---|
| **A. fully synthetic series** | the `is_peg_asset` branch above | **canonical USDC only** — the gate is an identity comparison, not a class | O=H=L=C, trade_count 0, `method: peg`, `derived: true` on every pre-2026-03-11 candle |
| **B. USD denomination assumed** | `close_usd = close × 1` for every USDC-quoted candle before the oracle window | **every asset whose canonical quote is USDC** (the majority of the universe) | candles look measured (real O/H/L/C, real trades) but `close_usd` inherits USDC=$1; on 2023-03-11 every such asset's USD price is overstated by the depeg |

Defect B is [[0247]]'s "known adjacent gap" and [[0168]]'s re-enrichment
item; it is out of scope for the fix but in scope for the memo, because the
same external anchor closes both.

Other pegged assets are **not** in defect A. USDT's canonical Stellar issuance
is a third-party IOU that really traded at ~0.13 after June 2022 ([[0172]]),
and it has base-leg candles, so `/ohlcv` measures it. [[0172]]'s prod sweep
found *"only three assets are ever a reference leg, and only USDC is pinned
at 1.0"*. The numeric sweep in `analysis/audit_our_series.py` re-checks this
from the API side (share of flat candles, zero-trade candles and closes
exactly 1.0, per asset) and its result belongs in §3 below.

## 3. Numeric proof [data, `analysis/audit_our_series.py` against the deployed API, 2026-09-04]

The detector metrics, our USDC series against the `native` control
(`data/audit_our_series.md`, `data/our_USDC_1d.csv`, `data/our_native_1d.csv`):

| metric | USDC | native |
|---|---|---|
| daily points | 2 042 (2021-02-01 → 2026-09-04) | 2 414 (2018-05-15 →) |
| share O = H = L = C | **1.000** | 0.000 |
| share close exactly 1.0 | 0.913 | 0.000 |
| share volume_base = 0 | 1.000 | 0.000 |
| share trade_count = 0 | **1.000** | 0.000 |
| share derived | 1.000 | 0.846 |
| longest run of identical closes | **1 864 days** | 1 |
| distinct closes | 177 | 2 042 |
| realized vol of daily log returns | **0.80 bps** | 654 bps |
| method | peg 1 864 · oracle 178 | peg 1 865 · oracle 177 · empty 372 |

2023-03-10 → 03-13 as we publish it: four candles `1.0 / 1.0 / 1.0 / 1.0`,
trade_count 0, `method: peg`, `derived: true`. Chainlink on the same day:
243 rounds, low 0.8800 (see `R-source-candidates.md`).

Realized vol on the 2 041 common days with Bitstamp: **ours 0.80 bps, Bitstamp
9.11 bps**; largest daily move ours 8 bps, Bitstamp 297 bps. Bitstamp shows 2
days with |close − 1| > 50 bps in our span; we read exactly 1.0 on both.

`figures/fig4_quality_flags.png` is the timeline: every flag is set on every
day until 2026-03-11, when `oracle` takes over and close ≠ 1.0 begins.

### Sweep: 300 assets by 24 h volume plus every peg-coded asset, 232 profiled
(`data/synthetic_profile.csv`; 68 had fewer than 30 daily points)

- **`share_trades0 == 1.0`: USDC and nothing else.** The fully synthetic
  series (defect A) is one identity, confirming [[0172]]'s prod sweep from the
  API side.
- **`share_close_1 > 0.5`: USDC, USDCAllow, FELIX.** Both others have real
  trades (USDCAllow: 29–37 trades/day, ~$57 M/day in the last week) and trade
  1:1 against USDC, so a close of exactly 1.0 is *measured* there. A phase-4
  detector on "close == 1.0" alone would false-positive on them; the
  discriminating feature is `trade_count == 0`, not the price.
- **`share_flat > 0.5`: 25 assets**, 20 of them with < 180 points — thin
  assets with one trade per day, where O = H = L = C is honest. Flatness is
  not a synthetic signal on its own either.
- **Canonical USDT** (`GCQTGZQQ…TG6V`): 2 034 points, share_flat 0.002,
  trades on every day, realized vol 1 270 bps — measured, with
  `method: peg` on 1 858 days. That label is defect B, not A.
- **135 of 232 assets carry `method: peg` on some days**, `native` on 1 865
  of them. `peg` on XLM — an asset with 36 208 trades on 2023-03-11 — means
  "USD price = USDC price × an assumed 1.00", which no consumer will read
  that way. This is the strongest evidence for acceptance criterion 5: the
  current `method` vocabulary conflates "no measurement of USDC" with "USD
  denomination assumed", on assets that are not stablecoins.
