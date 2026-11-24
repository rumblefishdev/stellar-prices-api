---
title: "Step 1 measured on production: XLM vs Binance, affected buckets, and where the remaining cheap lows come from"
type: research
status: mature
spawned_from: notes/R-usdt-legs-widen-usd-candles.md
spawns: []
tags: [ohlcv, usdt, high-low, measurement, aquarius]
links:
  - "R-usdt-legs-widen-usd-candles.md"
history:
  - date: "2026-10-14"
    status: mature
    who: akot
    note: >
      Read-only measurement on prod (dev_read) of PR #402's SQL against the
      code it replaces, 1d buckets 2021-02-01..2026-10-13. AC 6 and AC 7.
---

# Step 1 measured on production

## Method

- The query is the one PR #402 generates (taken from `system.query_log` of
  the `ohlcv_it` run), widened to every asset and grouped by asset × day.
- The old candle is the same query with `band` false everywhere: the fallback
  branch is byte-identical to the pre-change SQL, so this is today's API.
- Both run on `prices.price_ohlcv_1d` as `dev_read`, month by month. The
  3.73 GiB per-query limit rejects the whole range at once.
- Binance: `XLMUSDT` 1d klines from `data-api.binance.vision`.
- Only `1d` was measured, not `1h` or `1m`. The scripts stayed in the
  session scratchpad.

## AC 6: XLM vs Binance

XLM 1d 2022-06-18: $0.1767 / $0.0815 → **$0.1137 / $0.1005** (Binance
$0.1135 / $0.1001).

p95 of |ours / Binance − 1|:

| window | days | high: before → after | low: before → after |
|---|---:|---|---|
| the analysis' window | 557 | 8.45 % → **1.14 %** | 12.93 % → **2.16 %** |
| 2022-07 → 2026-08 | 1524 | 27.7 % → 1.33 % | 52.3 % → 8.18 % |
| all | 2081 | 23.2 % → 1.24 % | 43.5 % → 6.44 % |

Medians after: 0.16 % (high), 0.19 % (low).

**Step 1 alone reproduces the analysis' "steps 1 + 2" figures.** The analysis
measured 557 days, the ones where `1m` had prices: 2021-02 → 2022-06 and
2026-09 → 2026-10-12. On those days step 1 gives 1.14 % / 2.16 % against the
analysis' 1.14 % / 2.19 %. For XLM, step 2 adds almost nothing, because its
price-setting market is USDC, whose USD rate stays near 1 all day. The analysis'
gains for XLM-quoted tokens (ETH, XRP) are a separate measurement and still
stand.

Its baseline was lower (8.45 % / 12.93 % against 23.2 % / 43.5 %) only because
the window left out 2022-07 → 2026-08, where most bad days are.

## AC 7: what changes (11,114,887 priced 1d buckets)

| | buckets | assets |
|---|---:|---:|
| a leg leaves the band | 37,582 (0.34 %) | 419 |
| … of which USDT | 37,582 | 419 |
| … of which another quote | 0 | 0 |
| high or low changes | 16,136 | 340 |
| open or close changes | 4,519 | — |
| `method` changes | 528 | — |
| band extreme newly `null` (review IN-03) | 0 | — |
| fallback: no valid USDC/XLM leg | 1,345 | 160 |

- In practice the allowlist drops only USDT. A row on any other quote fails
  `convertible`, so it never entered the price before either.
- Where the high moves, the median move is −3.0 %. At p95 the old high was
  about 3× the new one. The low moves by a median +3.2 % and a p95 +57 %.
- Fallback buckets are few and falling: 405 in 2021, 17 in 2026. This
  supports Decision 1 (no flag).
- Changed buckets per year:

  | 2021 | 2022 | 2023 | 2024 | 2025 | 2026 |
  |---:|---:|---:|---:|---:|---:|
  | 2,486 | 4,534 | 1,780 | 2,484 | 2,698 | 2,154 |

## What step 1 leaves: cheap lows on the USDC markets themselves

After step 1, 121 XLM days have a low more than 5 % off Binance. Market that
sets the low:

| source | days | low vs Binance (median / worst) | that market's close vs Binance | years |
|---|---:|---|---|---|
| Aquarius | 62 | −56 % / −98 % | −0.03 % | 2025–26 |
| SDEX | 29 | +5.6 % / −34 % | −0.04 % | mostly 2021 |
| Phoenix | 12 | −8 % / −29 % | −0.7 % | 2024 |
| Soroswap | 9 | −18 % / −46 % | −12.7 % | 2024 |
| Sushiswap | 9 | −37 % / −99 % | −26.5 % | 2025–26 |

- **SDEX**: mostly real cross-venue gaps on the 2021 crash days, which the
  analysis also found.
- **Phoenix, Soroswap, Sushiswap**: thin pools trading away from the market.
- **Aquarius**: on a market of thousands of fills a day with a correct close,
  single fills print far below the price. The cheap low drifts down steadily:

  | 2025-10 | 2026-03 | 2026-04 | 2026-06 | 2026-08 | 2026-10 |
  |---:|---:|---:|---:|---:|---:|
  | $0.20 | $0.14 | $0.07 | $0.056 | $0.045 | $0.032 |

  At `1m`, 2026-09-25 08:38 holds 2 fills and 1,001 XLM: 1,000 XLM at
  0.2188 and exactly 1 XLM for 0.0383677 USDC. The arithmetic closes, so this
  is a real swap and not a decoding error. It looks like one stale pool that
  nobody arbitrages.

  Aquarius lists four XLM/USDC pools:
  - a concentrated pool and a constant-product pool at 0.1 %, both deep;
  - a stable-curve pool, now empty (`CAQXW3…`);
  - a constant-product pool at 0.3 % holding ~$40 (`CARCLMAP…`).

  Which of them printed the fill is not established. Candles carry no pool
  id, and stellar.expert's event index for these pools ends at 2025-12-31.
- The analysis' "rule 3" (a ±50 % band around the day's median price) removed
  exactly this kind of print: on its window, the worst high went from 395 % to
  12.7 % and the worst low from 85 % to 18.2 %. It was not part of 0332.
