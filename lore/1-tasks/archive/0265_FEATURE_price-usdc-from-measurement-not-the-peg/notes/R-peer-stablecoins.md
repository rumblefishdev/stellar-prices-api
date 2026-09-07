---
title: "Phase 2 — does another stablecoin help? Proxy, backfill or cross-check, measured on Chainlink"
type: research
status: developing
spawned_from: R-source-candidates.md
spawns: []
tags: [pricing, stablecoin, evidence, correlation]
links:
  - "../data/peer_stablecoins.md"
  - "../data/peer_corr.csv"
  - "../data/peer_episodes.csv"
  - "../analysis/peer_stablecoins.py"
history:
  - date: 2026-09-04
    status: seed
    who: akot
    note: >
      All ten coins read from Chainlink mainnet feeds (analysis/fetch_chainlink.py)
      so the comparison has one source and one method. Feeds with hourly
      heartbeats were sampled every 2nd-4th round (`stride` column in the
      rounds CSV); daily closes are unaffected, intraday lows may be shallower
      than the true print. DAI's first two aggregators no longer answer, so DAI
      starts 2020-08-21 here. BUSD has no mainnet feed any more and is absent.
---

# Phase 2 — the short answer

**No other stablecoin is a proxy or a backfill for USDC/USD. A peer basket is
useful for exactly one thing: anomaly detection.** The reasons are measured,
not argued, in `data/peer_stablecoins.md`.

## What the peers did on the one day that matters [data, Chainlink daily close]

| coin | 2023-03-11 close vs $1 | corr of daily deviation with USDC, 2023-03-08..18 | beta to USDC | reading |
|---|---|---|---|---|
| USDC | **−319 bps** (intraday −1 200) | 1 | 1 | the event |
| DAI | −285 bps | **0.994** | **0.89** | moves *with* USDC almost one-for-one: USDC collateral |
| FRAX | −423 bps | 0.930 | **1.43** | over-reacts: part USDC-collateralised, thinner |
| USDP | −95 bps | 0.63 | 0.24 | partial contagion |
| TUSD | −29 bps | 0.63 | 0.08 | barely moved |
| LUSD | **+45 bps** | −0.16 | −0.11 | ETH-collateralised: went the other way |
| USDT | **+90 bps** | **−0.81** | **−0.24** | the flight-to destination: premium while USDC sank |
| PYUSD, FDUSD, USDe | — | — | — | did not exist yet (feeds start 2023-11, 2023-12, 2024-04) |

Over the full common sample (2 025 days) the level correlation with USDC is
DAI **0.79**, FRAX 0.42, USDP 0.27, TUSD 0.02, LUSD 0.00, USDT **−0.33**.

## Why each candidate fails as a backfill [data + est.]

- **DAI is not an independent source.** Correlation 0.79 overall and 0.994
  in the stress window, beta 0.89: DAI *is* USDC for the purpose of a depeg,
  because its collateral is largely USDC (the PSM). Using it to fill USDC
  history would reproduce the same event slightly damped and add DAI's own
  six idiosyncratic episodes from 2020 (up to +424 bps) that USDC never had.
- **USDT is anti-correlated in stress.** It would record the SVB weekend as
  a *premium*. As a backfill it does not merely miss the depeg, it inverts it.
- **USDP** caught a quarter of the move, **TUSD** a tenth; both have their own
  episodes USDC did not share (TUSD: 12 of 12 idiosyncratic, including 47 days
  in early 2024 down to −404 bps; USDP: 7 of 9).
- **LUSD and FRAX** are the noisiest coins in the set (573 and 327 days
  beyond 50 bps) and their episodes are almost all their own (44 of 45, 16
  of 17).
- **PYUSD, FDUSD, USDe, BUSD**: no history that reaches 2023 (or, for BUSD,
  no feed at all).

Shared vs idiosyncratic, all coins, |close − 1| > 50 bps, ±3 days:

| coin | episodes | shared with USDC | idiosyncratic |
|---|---|---|---|
| USDC | 1 | 1 | 0 |
| USDT | 1 | 1 (the +90 bps premium) | 0 |
| DAI | 7 | 1 | 6 |
| FRAX | 17 | 1 | 16 |
| USDP | 9 | 2 | 7 |
| TUSD | 12 | 0 | 12 |
| LUSD | 45 | 1 | 44 |
| PYUSD | 1 | 0 | 1 |
| FDUSD | 2 | 0 | 2 |
| USDe | 0 | 0 | 0 |

USDC has had **one** episode in five and a half years of Chainlink history.
Every peer has more, and almost all of theirs are private. A backfill from
any of them imports noise USDC never had and, on the one shared day, gets the
size wrong (DAI −285, FRAX −423, USDP −95 against −319).

## What the basket *is* good for [est.]

A cross-check rule for phase 4: if USDC's measured deviation exceeds a
threshold while DAI's does not move with it (beta ≈ 0.9 expected), the USDC
reading is suspect; if DAI moves and USDC does not, the USDC *feed* is
suspect. USDT is the useful contrarian: a USDT premium coincident with a
USDC discount is the signature of a genuine USDC event, not of a bad print.
None of that needs a peer's price to be published as USDC's.

## Consequence for phase 3

Primary and fallback stay what phase 1 chose (Chainlink, Bitstamp, Kraken):
all three *are* USDC/USD. The peer basket enters only as a validation input.
The [[0172]] point stands separately: canonical Stellar USDT
(`GCQTGZQQ…TG6V`) is an IOU that traded at ~0.13, and its `/ohlcv` series is
measured; it is not this USDT and not a candidate for anything here.
