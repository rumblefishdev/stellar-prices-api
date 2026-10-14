---
title: "Why USDT-quoted markets widen USD candles, and how others pick price-setting trades"
type: research
status: mature
spawns:
  - notes/S-price-setting-markets-are-usdc-and-xlm.md
tags: [ohlcv, usdt, methodology, high-low]
links: []
history:
  - date: "2026-10-14"
    status: mature
    who: akot
    note: "Adam's analysis, moved out of the task README"
---

# Why USDT-quoted markets widen USD candles

A market here is pair × source: XLM/USDC on SDEX, XLM/USDT on SDEX, XLM/USDC on
Aquarius, and so on.

## How USD mode computes high/low today

Each market is converted with one rate for the whole bucket, the day's close:
`rate = close_usd / close` of that market. Then `high_usd = high × rate`,
`low_usd = low × rate`, and `high = max`, `low = min` across markets.

## Case: XLM, 1d, 2022-06-18

| market | share of volume | high | low |
|---|---:|---:|---:|
| XLM/USDC, SDEX | 96 % (28.1 M XLM) | $0.1137 | $0.1005 |
| XLM/USDT, SDEX | 4 % (1.2 M XLM) | 0.3242 USDT → **$0.1767** | 0.1496 USDT → **$0.0815** |
| API today (max/min of both) | — | **$0.1767** | **$0.0815** |
| Binance | — | $0.1135 | $0.1001 |

The USDT market is converted at the end-of-day USDT rate ($0.5449). That day
USDT traded between $0.43 and $0.96, so an evening rate does not fit morning
trades. The API returns the USDT market's extremes.

## Why max/min is right and its inputs are wrong

High and low are, by definition, the extremes of real trades; volume plays no
part. A volume-weighted "average high" (weights 0.96 / 0.04) gave tidy numbers
but was rejected: the result is no longer a price anyone traded at.

A trade on XLM/USDC has a price in USD. A trade on XLM/USDT has a price in USDT;
its USD value is our estimate, price × a USDT rate we compute ourselves. A
maximum of values measured with error is biased up and a minimum down. The more
estimated markets go in, the wider the candle, whatever the truth.

## Why Stellar USDT has no usable USD rate

- "USDT" on Stellar (`GCQTGZ…`) is not Tether but an Apay bridge receipt (issuer
  domain `dead.apay.io`). It lost its peg in 2022-06; ~$0.15 today.
- Its only USDC market has turned over a median of 0–27 USDC a day since 2023.
- Within one minute it can disagree with the rate implied through XLM. On
  2022-06-18 USDT/USDC printed 0.43 at 00:49 and 0.957 at 01:12 on ~74 USDC an
  hour, while the XLM legs implied 0.36–0.65.
- A per-minute rate would not help: the token has no single, reliable USD price.
- Same reason [[0172]]/[[0212]] fenced the USD reference to USDC (Oskar,
  2026-10-14).

## How others do it

- **US equities (CTA tape):** every trade has a sale-condition code that decides
  whether it may set high/low. Some trades count for volume only.
- **CoinMarketCap:** drops outlier prices and, manually, markets whose price does
  not reflect a free market (e.g. an exchange blocking deposits/withdrawals).
- **CoinGecko:** drops tickers with outlier prices (MAD).

We do not filter by price; we filter by whether the quote asset's USD value is
known.

## Measured effect (steps 1 and 2 together)

- 419 assets traded at the same time in USDT and in USDC or XLM since 2021.
- XLM vs Binance, p95 deviation: high 8.45 % → 1.14 %, low 12.93 % → 2.19 %.
- XLM-quoted tokens: median error 2–4× lower (ETH 1.34 % → 0.57 %, XRP
  0.74 % → 0.18 %).

The analysis does not separate step 1 (drop USDT from the band) from step 2
(per-minute rate for USDC/XLM markets).
