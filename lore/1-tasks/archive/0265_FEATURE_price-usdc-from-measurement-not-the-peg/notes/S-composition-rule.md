---
title: "Phase 3 — how the measured USDC/USD series is put together: primary, fallback, disputes, gaps, and what the endpoint says about each point"
type: synthesis
status: developing
spawned_from: R-source-candidates.md
spawns: []
tags: [pricing, stablecoin, decision, api-contract]
links:
  - "../analysis/compose_usdc.py"
  - "../data/compose_usdc.md"
  - "../data/composed_usdc_usd_1d.csv"
  - "../data/composed_usdc_usd_1h_2023-03.csv"
history:
  - date: 2026-09-04
    status: seed
    who: akot
    note: >
      Rule chosen on the phase-1 dispersion figures and proven by running it
      (analysis/compose_usdc.py) over 2021-01-25 → today. Every number here is
      from that run.
---

# Phase 3 — the composition rule

## The rule

| step | rule | why (measured) |
|---|---|---|
| **primary** | Chainlink USDC/USD rounds, Ethereum mainnet proxy `0x8fFf…18f6` | real USD, verifiable by anyone with an RPC, no licence, 0.25 % deviation + 23 h heartbeat, from 2021-02-17; agrees with Bitstamp to 0.8 bps median calm / 4 bps stress and with Kraken to 0.6 bps |
| **fallback** | Bitstamp USDC/USD, used only where the primary has **no observation in the bucket** | the only key-less real-USD book reaching 2021-01-01; same cluster as the primary |
| **cross-check** | Kraken (OHLC last 720 d, trades for 2023-03) and Bitstamp | independent USD venues; USDT-quoted venues are excluded from arbitration because their cluster sits 3–4 bps off in calm and 20–30 bps off in stress (that is USDT/USD, not USDC/USD) |
| **disagreement** | if a cross-check differs from the primary by **> 25 bps** on the bucket close, keep the primary value and flag `measured-disputed` | 25 bps is 3.5× the calm p95 between USD sources (≤ 7 bps) and below the stress median between them (4 bps median, 33 bps max). No averaging: a median of venues is a number nobody printed, and the reviewer's question "where did 0.9681 come from" must have a one-word answer |
| **gaps** | a bucket with no source publishes **no price** and `quality = missing` | forward-fill is the defect being removed. The only carry allowed is *inside the primary's own heartbeat* on the hourly grain: an hour with no round means "no 0.25 % move since the last round", and the row says `n_obs = 0` |

**Why not median-of-N or volume-weighting.** The dispersion data says the USD
sources are one cluster; a median would change the answer by < 1 bps on 95 %
of days and would blur the one thing that matters on the other 5 %: which
venue printed what. Volume-weighting would hand the series to Binance (which
was closed for the event) or to the USDT cluster. "Primary with a flagged
dispute" keeps provenance intact and is what an SCF reviewer can re-derive.

## What the rule produces (2021-01-25 → 2026-09-04, `data/compose_usdc.md`)

| quality | days | share |
|---|---|---|
| measured (Chainlink) | 2 021 | 98.6 % |
| fallback (Bitstamp) | 24 | 1.2 % — 2021-01-25 → 02-16 before the feed existed, plus sparse days to 2021-03-18 while phase 1 was young |
| measured-disputed | 4 | 0.2 % — 2022-11-09 (27 bps), 2022-11-23 (26), 2023-01-19 (36), 2023-03-13 (33) |
| missing | 0 | — |

Hourly, March 2023: 706 measured, 32 disputed (all inside 03-10 → 03-13, when
venues genuinely disagreed by up to 93 bps hour to hour), 6 fallback.
Trough on the composed hourly series: **0.8833 at 2023-03-11 07:00 UTC**.

Against our series on the 2 042 common days: median |diff| 0.8 bps, p99
11.8 bps, **max 318.8 bps on 2023-03-11**. Two days where we publish exactly
1.0 and the market was more than 25 bps away.

## Normalisation decisions

- **Time zone and boundaries:** everything is UTC; a daily bucket is
  `[00:00, 24:00)`. OKX's `1D` bar is Hong-Kong-day aligned and was fetched
  as `1Dutc` for that reason; anyone adding a venue must check its day.
- **Candle from rounds:** open = first round in the bucket, high/low =
  max/min, close = last; `n_obs` = number of rounds. A day with one round is
  honestly flat (68 % of Chainlink days), which is why flatness is *not* a
  synthetic signal in phase 4.
- **Candle from trades (Kraken):** standard OHLC over trade prints,
  `n_obs` = trades.
- **Venues that were closed** (Binance 2022-09-26 → 2023-03-11 14:00) are
  simply absent for those buckets; they are never interpolated.

## What the endpoint must say (acceptance criterion 5)

Today `/ohlcv` already carries `method`, `derived` and `trade_count`, and
phase 0 showed why they are not enough: `method: peg` is also stamped on
`native` (1 865 days) and on canonical USDT (1 858 days), meaning "USD
denomination assumed", so a consumer cannot read it as "USDC unmeasured".
The composed series carries, per point:

| field | values | consumer meaning |
|---|---|---|
| `source` | `chainlink` · `bitstamp` · `` | who printed the number |
| `quality` | `measured` · `measured-disputed` · `fallback` · `missing` | how much to trust it; `missing` has a null price |
| `n_obs` | integer | rounds or trades behind the bucket; 0 on an hourly carry inside the heartbeat |
| `xcheck_spread_bps` | float | max distance to a cross-check venue that day |

Recommendation for the API: add `source` and `quality` beside the existing
`method`, keep `method` for the enrichment vocabulary (`oracle`/`peg`/
`pivot`/`traded`) but stop emitting `peg` on non-stablecoins as if it were a
price method; the phase-5 note says how that interacts with [[0247]]'s
`'external'` value.
