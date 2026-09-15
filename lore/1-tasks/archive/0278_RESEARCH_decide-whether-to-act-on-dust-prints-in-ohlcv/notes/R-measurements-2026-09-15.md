---
title: "Measurements for the candle-fix decisions: dust by price source, window size k, VWAP vs weighted median, thin-pair fill density"
type: research
status: mature
tags: [ohlcv, dust-prints, measurement, horizon, clickhouse]
links: []
history:
  - date: "2026-09-15"
    status: mature
    who: akot
    spawned_from: ["notes/S-close-estimator-and-pivot-reference.md"]
    note: >
      Run read-only against production ClickHouse and the public Horizon, to
      settle D3, D5 and D6 of the candle-fix decision list. The queries and
      scripts are described in "How it was measured" below and are not in
      the repository; raw outputs were not kept either.
---

# Measurements for the candle-fix decisions

Three data sources, all read on 2026-09-15:

| Source | What | Size |
| --- | --- | --- |
| `price_ohlcv_1m`, yXLM/XLM, SDEX, 2021-04 → 2026-09 | daily close estimators vs the reference 1 yXLM = 1 XLM | 1 130 days |
| `price_ohlcv_1m`, every SDEX pair, 2026-08-01…07 | fills in the last traded minute of a day; minutes back to reach 3 fills | 99 559 asset-days |
| Horizon `/trades`: XLM/USDC (0.8 d), yXLM/XLM (5.1 d), AQUA/XLM (13.0 d) | fill size and type; offer price vs amount ratio | 237 789 fills |

Asset ids on prod: XLM 4, USDC 3, AQUA 5, yXLM 10. Coverage check
(coverage query): yXLM/XLM and AQUA/XLM have SDEX 1m since 2021 (13.0 M and
57.8 M fills), so neither is a thin pair; they were chosen because yXLM has a
built-in reference and AQUA is the largest non-XLM market.

## 1. Is yXLM/XLM a usable reference?

| | p01 | p10 | p50 | p90 | p99 | min | max |
| --- | --- | --- | --- | --- | --- | --- | --- |
| daily VWAP | 0.99727 | 0.99925 | 0.99992 | 1.00017 | 1.00194 | 0.93092 | 1.04226 |
| stored 1d close | 0.99700 | 0.99924 | 0.99996 | 1.00000 | 1.00112 | 0.94444 | 1.01010 |

Yes: it sits at 1.0 within ~0.1 % on 80 % of days. Deviations beyond ~0.3 %
are treated as measurement error, with the caveat that a real, small discount
is possible.

## 2. Close estimators on yXLM/XLM — D5, D6

Window = the last traded minute of the day, extended back over earlier
minutes until the cumulative `trade_count` ≥ k. k counts **all** fills
(historical 1m rows do not know which fills were price-forming). Error =
|estimate − 1|.

| Estimator | Median | p99 | Max | Days > 1 % | Days > 5 % |
| --- | --- | --- | --- | --- | --- |
| today: last fill (`close`) | 0.004 % | 0.500 % | **5.556 %** | 4 | 1 |
| VWAP, k = 1 | 0.005 % | 0.405 % | 1.010 % | 1 | 0 |
| VWAP, k = 2 | 0.006 % | 0.401 % | 1.000 % | 1 | 0 |
| VWAP, k = 3 | 0.006 % | 0.386 % | 1.000 % | 1 | 0 |
| VWAP, k = 5 | 0.006 % | 0.382 % | **3.367 %** | 3 | 0 |
| VWAP, k = 10 | 0.007 % | 0.419 % | **4.220 %** | 2 | 0 |
| weighted median, k = 3 | 0.005 % | 0.401 % | 1.000 % | 1 | 0 |
| weighted median, k = 5 | 0.005 % | 0.372 % | 1.153 % | 2 | 0 |
| weighted median, k = 10 | 0.005 % | 0.341 % | 1.000 % | 1 | 0 |

- The one remaining day > 1 % (2021-04-04) is the pair's first trading day:
  4 fills, all at 1.01. A real price, not an error.
- The worst stored-close days (2023-10-31 at 0.9444, 2022-07-08 at 0.9896,
  2023-08-16 at 0.99) all have every window estimator at 0.9999–1.0000: a
  single last print on a 4 000–7 000-fill day.
- On this pair the last minute holds 1 fill on 372 of 1 130 days (33 %), 2
  on 172, ≥ 3 on 586. Minutes back to reach 3 fills: median 1, p90 3, max 3.
  To reach 10: median 4, p90 8.

Reading: k = 1…3 are equivalent; k ≥ 5 hurts VWAP (older prints enter the
window on thin days); the weighted median is stable up to k = 10. At k = 3
the two estimators are indistinguishable.

## 3. Fill density across SDEX — D5's time cap

One week, every SDEX pair, per (asset, quote, day):

| Fills per day | Asset-days | Last minute has 1 fill | Whole day has < 3 fills | Minutes back to 3 fills (median / p90) |
| --- | --- | --- | --- | --- |
| < 20 | 83 864 (84 %) | 95.7 % | **50.2 %** | 231 / 808 |
| 20–99 | 11 838 | 95.4 % | 0 % | 57 / 262 |
| 100–999 | 3 523 | 93.6 % | 0 % | 9 / 75 |
| ≥ 1 000 | 334 | 76.9 % | 0 % | 2 / 7 |

Reading: "the last traded minute" is one fill on ~95 % of asset-days
outside the top bucket, and even in the top bucket on 77 %. A window in
fills must therefore be capped in time, or it reaches back hours on 84 % of
asset-days. The cap cannot be measured: no reference exists for thin pairs.

## 4. Dust fills by price source — D3

Horizon reports the **resting offer's** `price.n/d` for order-book fills and
the amount ratio for pool fills. A fill is "below threshold" when
`1/base_stroops + 1/counter_stroops > 0.001`. Reference for deviation: the
median amount-ratio price of price-forming fills within ±10 min.

| Pair | Fills | OB below thr. | Pool below thr. | OB dust at **offer** price | OB dust at **ratio** price | Pool dust at ratio |
| --- | --- | --- | --- | --- | --- | --- |
| XLM/USDC | 78 199 | 418 (0.6 %) | 14 (0.6 %) | med 0.10 %, p90 0.26 %, max 0.6 %, > 1 %: 0 | med 0.14 %, max 4.5 %, > 1 %: 3.1 % | med **415 %** |
| yXLM/XLM | 80 000 | 1 664 (2.1 %) | 1 of 25 | med 0.05 %, p90 0.19 %, max 0.6 %, > 1 %: 0 | med 0.10 %, max 16.6 % | 0.24 % (n = 1) |
| AQUA/XLM | 79 590 | 6 012 (10.7 %) | 3 662 (15.6 %) | med 0.01 %, p90 0.46 %, max 1.4 %, > 1 %: 0.2 % | med 0.03 %, max 20.2 % | med 0.03 %, max **5 388 %** |

Noise floor (price-forming fills vs the same reference): p99 0.50 % /
0.23 % / 0.60 %. Base volume carried by below-threshold fills: 0.0000 % on
every pair. Minutes with fills but no price-forming fill: 0 / 11 (0.2 %) /
644 (5.6 %).

Reading: an order-book dust fill priced from the offer is inside the noise
floor (worst 1.4 % on ~8 000 such fills). The "favourable off-market dust
offer" case the all-fills threshold was meant to catch did not materialise
in 220 k fills. The same fills priced from the amount ratio deviate up to
20 %, pool dust up to 5 388 %, so the threshold is necessary exactly where
the price comes from the amounts.

## How it was measured

- **Close estimators (§2):** one ClickHouse query over `price_ohlcv_1m FINAL`
  for the pair, grouped by day: the day's minutes as an array sorted by
  timestamp descending, the cumulative `trade_count` over it, the window for
  each k as the prefix up to the first index where the cumulative count
  reaches k; VWAP = Σ `volume_quote` / Σ `volume_base` over the prefix,
  weighted median = `quantileExactWeighted(0.5)` of the minute VWAPs weighted
  by `volume_base` in stroops. Errors summarised offline.
- **Fill density (§3):** the same array shape over every SDEX pair for one
  week, keeping per (asset, quote, day) the last minute's `trade_count`, the
  day's total, and the timestamp gap to the minute where the cumulative count
  reaches 3.
- **Dust by price source (§4):** Horizon `GET /trades` for the pair,
  `order=desc`, `limit=200`, followed through `_links.next` back to the cut-off
  date (capped at 400 pages). Per record: `base_amount`, `counter_amount`,
  `trade_type`, `price.n/d`, `ledger_close_time`. Stroops = amount × 10⁷;
  threshold `1/base + 1/counter > 0.001`; the local market reference is the
  median amount-ratio price of price-forming fills within ±10 minutes.

## Limits

- Three liquid pairs over days (Horizon) and one liquid pair over years
  (yXLM). Nothing here measures a thin pair's close error, because no
  reference exists for one.
- k is counted in all fills, not price-forming ones; on liquid pairs the
  difference is small, on thin pairs unknown.
- Horizon's `price` for pool fills is the amount ratio, so the "pool dust at
  reserves price" variant of D2 was not measured; the reserves themselves are
  not in `/trades`.
