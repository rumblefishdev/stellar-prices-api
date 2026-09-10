---
title: "Phase 1 — candidate sources for a measured USDC/USD history, compared on fetched data"
type: research
status: developing
spawned_from: S-phase0-root-cause.md
spawns: []
tags: [pricing, stablecoin, oracle, data-sources, evidence]
links:
  - "../data/sources.csv"
  - "../data/compare_sources.md"
  - "../analysis/fetch_external.py"
  - "../analysis/fetch_chainlink.py"
  - "../analysis/compare_sources.py"
history:
  - date: 2026-09-04
    status: seed
    who: akot
    note: >
      Every venue number below was produced by analysis/*.py against a live
      fetch on 2026-09-04; every documentation claim carries the URL that was
      read. "n/v" = not verified, and stays n/v rather than being guessed.
---

# Phase 1 — which sources can actually supply a measured USDC/USD series

Three kinds of evidence, kept apart: **[data]** from the fetched series,
**[docs]** from a page read on 2026-09-04, **[est.]** my judgement.

## Smoke test: does the source show March 2023, and does it disagree with its neighbours?

Every key-less source with history passed, and they disagree, which is the
result the brief expected. [data]

| source | quote | hourly min close (UTC) | 2023-03-11 daily close | daily low |
|---|---|---|---|---|
| Bitstamp | USD | **0.8627** @ 03-11 07:00 | 0.9685 | 0.8053 |
| Kraken (from 580 028 trades) | USD | 0.9010 @ 03-11 12:00 | 0.9670 | 0.8740 |
| Chainlink on-chain (3 240 rounds read, 2021-02-17 → today) | USD | **0.8800** @ 03-11 07:51:23, 243 rounds that day | 0.9681 | 0.8800 |
| OKX | USDT | 0.8787 @ 03-11 07:00 | 0.9607 | 0.8698 |
| Bybit | USDT | 0.8840 @ 03-11 07:00 | 0.9611 | 0.8500 |
| Binance | USDT | 0.9128 @ 03-11 15:00 (first hour after relisting) | 0.9592 (partial day) | 0.8820 |
| Coin Metrics PriceUSD | USD | n/a, daily | 0.9706 | — |

The trough is 07:00–08:00 UTC on 2023-03-11 on every venue that was open.
The daily *close* is 0.96–0.97 everywhere, i.e. a daily grain alone
understates the event by two thirds. Institutional references agree on the
trough: the Fed's FEDS Note (2025-12-17) says "at its trough, USDC traded at
86 cents"; Circle's 2023-03-13 release closes the de-peg. [docs, URLs in
`sources.csv` and the agent transcript]

## Coverage and gaps [data]

| source | first day | missing days | the gap |
|---|---|---|---|
| Coin Metrics | 2018-09-28 | 0 | — |
| Binance | 2018-12-15 | 165 | delisted 2022-09-26 → relisted **2023-03-11 14:00 UTC** (both announcements fetched) — the hole sits exactly on the depeg |
| OKX | 2020-01-01 | 0 | one idiosyncratic print 2020-03-12 (close 0.9395, 610 bps off Binance) |
| Bitstamp | 2021-01-01 | 0 | — (API silent before 2021; listing date n/v) |
| Chainlink | 2021-02-17 | 1 (of 2 026 days) | 3 phase aggregators, round ids non-contiguous; 3 240 rounds total |
| Bybit | 2021-10-22 | 0 | — |
| Kraken OHLC | 2024-09-14 | 0 | endpoint caps at 720 candles; older history only via trades |

Our own series starts 2021-01-25 ([[0247]]). So the **only** key-less USD
sources that reach our start are Coin Metrics (daily, non-commercial licence)
and, from 2021-02-17, Chainlink. Bitstamp reaches 2021-01-01. Nothing
key-less covers 2021-01-25 → 2021-02-16 except Coin Metrics and Bitstamp.

## Dispersion between sources, daily closes, bps [data]

From `data/dispersion_daily.csv` (calm = every common day outside 2023-03-08..18):

| pair | common days | median calm | p95 calm | max calm | median stress | max stress |
|---|---|---|---|---|---|---|
| Chainlink – Bitstamp (USD–USD) | 2025 | **0.8** | 6.8 | 36 | **4.0** | 33 |
| Chainlink – Kraken (USD–USD) | 721 | **0.6** | 1.7 | 2.8 | — | — |
| Chainlink – Coin Metrics (USD–USD) | 2024 | 0.9 | 5.0 | 83 | 2.1 | 25 |
| Bitstamp – Kraken (USD–USD) | 721 | 0.8 | 2.3 | 6.3 | — | — |
| Bitstamp – Coin Metrics (USD–USD) | 2072 | 0.9 | 6.5 | 105 | 2.4 | 33 |
| Binance – Bybit (USDT–USDT) | 1614 | **1.0** | 3.0 | 48 | 1.5 | 19 |
| Binance – OKX (USDT–USDT) | 2274 | 1.0 | 5.0 | 610 | 1.0 | 15 |
| Chainlink – Binance (USD–USDT) | 1860 | 3.2 | 12.7 | 40 | **30** | 89 |
| Chainlink – OKX (USD–USDT) | 2025 | 3.0 | 12.7 | 38 | 29 | 74 |
| Bitstamp – Binance (USD–USDT) | 1908 | **3.7** | 14.7 | 95 | **23** | 93 |
| Bitstamp – OKX (USD–USDT) | 2073 | 3.4 | 14.8 | 97 | 24 | 78 |
| Bitstamp – Bybit (USD–USDT) | 1779 | 3.4 | 15.3 | 65 | 22 | 74 |

Reading: the USD sources, Chainlink included, form one cluster (sub-1 bps in calm, ~4 bps under stress between Chainlink and Bitstamp), the USDT venues form
another (about 1 bps), and the two clusters sit 3–4 bps apart in calm and
**20–25 bps apart under stress**. That offset *is* the USDT/USD rate. A
USDT-quoted candle is therefore not a USDC/USD observation; it can be a
cross-check, or an input after dividing by a measured USDT/USD, never a
primary. This decides phase 3's fallback ordering. [est. on data]

## The oracle question the brief asked

**Chainlink** could and did record the depeg: 243 rounds on 2023-03-11 with
consecutive answers ~0.3 % apart, lowest 0.8800 at 07:51:23 UTC (roundId
36893488147419104215, phase 2). Current parameters are deviation 0.25 % and
heartbeat 82 800 s [docs, feeds-mainnet.json]; the 2023 parameter value is
n/v, but a 12 % move exceeds any plausible threshold, and the observed
density is consistent with a threshold well under 1 %. [data + docs]

**Pyth** Benchmarks require an API key since 2026-08-26 (401 observed) and
the docs do not state history depth. **RedStone** serves 30 days. **API3**
has no on-chain history by design. **Chronicle** docs returned 429 three
times, nothing verified. [docs]

## Vendors [docs]

- **Coin Metrics Community**: `PriceUSD` daily from 2018-09-28, free, but
  **CC BY-NC 4.0**. A commercial price API cannot redistribute it; keep it as
  the independent cross-check and for the pre-2021 tail in analysis only.
- **CoinGecko**: key-less limited to the past 365 days (error 10012 observed);
  Analyst $129/mo for 2023; licence forbids redistribution/syndication.
- **CoinMarketCap**: Startup $79/mo; no redistribution as a standalone service.
- **CryptoCompare/CoinDesk**: key now required (401); limits n/v.
- **Kaiko / Amberdata / CoinAPI**: enterprise or paid; not evaluated on data.
  Amberdata has no USDC reference rate at all.
- **Curve 3pool**: key-less price history exists, orientation looks inverted
  (2023-03-11 high 1.2142) and is unverified. **Uniswap v3** subgraph needs a
  Graph API key.

## What this narrows the decision to [est.]

1. **Primary anchor: Chainlink USDC/USD rounds.** Real USD, verifiable
   on-chain by anyone (which matters for an SCF reviewer), no licence
   question, 0.25 % deviation so quiet days cost one round and stress days
   cost hundreds, history from 2021-02-17. Cost: a few thousand `eth_call`s
   once, then one call per new round; no key needed today.
2. **Fallback and cross-check: Bitstamp USDC/USD** (from 2021-01-01, real USD
   book, free, hourly), with Kraken trades as the second USD venue where the
   two need arbitration.
3. **Reject as primary:** every USDT-quoted venue (cluster offset above),
   Binance specifically (gap on the event), Coin Metrics (licence), CoinGecko
   and CoinMarketCap (redistribution clauses), Pyth/RedStone/API3 (no
   key-less history).
4. **Open for phase 3:** the 2021-01-25 → 2021-02-16 tail (Bitstamp only, or
   labelled peg); whether an hourly grain is worth it given that the daily
   close hides two thirds of the trough.

Full row-per-source table with cost, licence and effort: `data/sources.csv`.
