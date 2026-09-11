# Outlier prints in OHLCV candles — why one dust fill can set the price of the day

Measured on 2026-09-11 against production (`ch-prod-01`, ClickHouse 26.3.10.60),
public Horizon, Bitstamp and Binance. Found during task 0276's spot check (the
0267/0268 production rollout); the decision on what, if anything, to change is
task 0278.

Nothing here was caused by the 0267/0268 rollout. That rollout only multiplied
already-stored values by the measured USDC/USD rate.

---

## TL;DR

- A candle's `open`/`high`/`low`/`close` are single trades, and **every fill
  weighs the same**. On SDEX a fill of a few stroops carries a price that is a
  ratio of two small integers, so it can sit 10–1000 % away from the market.
- **Cause A — stroop quantisation.** All seven XLM daily closes that miss the
  market by more than 5 % are _exact_ small-integer ratios (1/3, 1/4, 1/1,
  1/17, 4/57, 1/10, 5/34). Horizon shows the fill behind 5/34: a classic
  liquidity-pool leg of **34 stroops XLM for 5 stroops USDC**, repeated every
  few seconds by a path-payment bot.
- **Cause B — intra-ledger order.** Our "last trade" key is
  `(ledger, operation_index, claim_index)` with **no transaction index**, so the
  close is not the chronologically last trade in 54 of 60 sampled days. On
  2026-04-02 this alone produced a −10 % close.
- **Blast radius.** The XLM/USDC close is also the USD reference for every
  XLM-quoted asset (pivot tier). On 2023-03-11 one 2-cent fill priced **7 328**
  daily candles ~26 % low — BTC at **$15 243** instead of ~$20 575.
- **Scale (SDEX, 1d, ≥ 20 trades):** 7.8 % of closes are exact small ratios;
  high is > 50 % above the candle VWAP in 21 % of candles, low > 33 % below it
  in 19 %. Soroban AMM sources (Aquarius, Soroswap, Phoenix) show almost none.
- **Candidate fix, measured:** exclude fills whose rounding error exceeds
  0.1 % from price formation (keep them in volume); fix the order key; define
  the published close as the VWAP of the closing minute; price the pivot from
  the bucket VWAP. On 1 181 days of XLM this takes the worst close from
  **280 % → 4.5 %** off Bitstamp without changing the typical error (0.15 %),
  and high/low days > 10 % off from **534 / 598 → 2 / 0**.

---

## 1. How a candle is built today

| Stage           | Where                                                     | What it does                                                                                                                                          |
| --------------- | --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| Fill extraction | `packages/prices-ingest-core/src/filter.rs:103`           | every `ClaimAtom` (order book **and** classic liquidity pool, incl. path-payment hops) becomes a trade; only zero amounts are dropped                 |
| Fill price      | `packages/prices-ingest-core/src/price.rs:7`              | `amount_bought / amount_sold`, both integer stroops                                                                                                   |
| Order           | `packages/prices-ingest-core/src/tick.rs:21`              | `lex_key = (ledger_sequence, operation_index, claim_index)` — `operation_index` restarts at 0 in every transaction                                    |
| 1m candle       | `packages/prices-ingest-core/src/bucket.rs:57-77`         | `close` = max `lex_key`, `high`/`low` = max/min over all fills, `vwap` = Σquote / Σbase                                                               |
| Rollups 15m…1M  | `packages/prices-clickhouse/schema/rollups.sql:102-105`   | `close = argMax(close, timestamp)` of the child candles, `high = max(high)`, `low = min(low)` — a dust close at 23:59 becomes the 1h, 4h and 1d close |
| USD pivot       | `packages/enrichment-worker/src/ch_enrich.rs` `pivot_sql` | XLM/USD reference = Σ(close × volume) / Σvolume across **sources** of the XLM/USDC bucket — with one source, that is the close                        |
| API `/ohlcv`    | read path                                                 | for `native` the leg with the largest volume wins — the SDEX leg, even on days where the AMM legs closed at the market (2026-02-09, 2026-04-02)       |

## 2. Cause A — stroop quantisation

Every classic Stellar amount is an integer number of stroops (10⁻⁷). A fill's
price is the ratio of two such integers, so its relative rounding error is
bounded by roughly

```
error ≲ 1 / amount_sold_stroops + 1 / amount_bought_stroops
```

A fill of 34 stroops against 5 can be off by up to ~20 %. The resting price
was 0.1631; the fill printed 5/34 = 0.1471.

**Evidence 1 — the seven outliers are exact ratios.** SDEX leg, XLM/USDC, 1d:

| Day        | Stored close     | As a fraction | Day VWAP |
| ---------- | ---------------- | ------------- | -------- |
| 2021-09-05 | 0.33333333333333 | 1/3           | 0.38556  |
| 2021-12-28 | 0.25             | 1/4           | 0.28514  |
| 2022-01-11 | 1                | 1/1           | 0.25390  |
| 2023-03-11 | 0.05882352941176 | 1/17          | 0.08375  |
| 2023-03-15 | 0.07017543859649 | 4/57          | 0.08574  |
| 2026-02-09 | 0.1              | 1/10          | 0.15980  |
| 2026-04-02 | 0.14705882352941 | 5/34          | 0.16451  |

Matched to 15 decimals. The Aquarius/Soroswap/Phoenix closes on the same days
are ordinary prices.

**Evidence 2 — Horizon, the fills themselves.**

- 2026-04-02 23:59:46 and 23:59:52: `liquidity_pool`, `0.0000034 XLM / 0.0000005 USDC`,
  price `5/34`, while order-book fills in the same seconds printed 0.16310.
- 2026-02-09, last fill of the day: `liquidity_pool`, `0.0000010 XLM / 0.0000001 USDC`,
  price `1/10`. The last fill with rounding error ≤ 0.1 % printed 0.15956
  (Bitstamp close 0.1595).

**Evidence 3 — size vs deviation.** Single-fill 1m candles of XLM/USDC (the
only place a candle tells us a fill's size), deviation from the hour's VWAP:

| Smaller leg  | Fills   | Median deviation | p90    | > 5 % off | Exact small ratio |
| ------------ | ------- | ---------------- | ------ | --------- | ----------------- |
| < 10 stroops | 538     | **6.6 %**        | 46 %   | 59 %      | 100 %             |
| 10–99        | 837     | 0.69 %           | 2.8 %  | 2.5 %     | 54 %              |
| 100–999      | 2 405   | 0.19 %           | 0.6 %  | 0 %       | 9.7 %             |
| 1k–10k       | 8 492   | 0.13 %           | 0.43 % | 0 %       | 0.8 %             |
| 10k–1M       | 55 080  | 0.14 %           | 0.5 %  | 0.1 %     | 0.2 %             |
| > 1M         | 120 080 | 0.19 %           | 0.72 % | 0.1 %     | 0.4 %             |

The curve follows the bound: from ~1 000 stroops on, a fill is as informative
as a dollar-sized one.

A round **price** is not by itself evidence of dust — a human limit order at
0.25 is a real trade. The discriminator is the fill **size**, which only the
ingest path still knows.

## 3. Cause B — the order key has no transaction index

`lex_key = (ledger_sequence, operation_index, claim_index)`. `operation_index`
is the index **inside a transaction**, so inside one ledger the "last" fill is
the one with the highest operation/claim index of _any_ transaction, not the
one applied last. Multi-hop path payments have high claim indices, which is
exactly where the dust pool legs sit, so the bias points at them.

- 60 random days of the last year (Horizon retains ~1 year): our stored close
  is a different fill from Horizon's last fill on **54 / 60** days. Usually
  harmless (all within 0.25 % of Bitstamp), because the wrong fill is usually
  an ordinary one.
- 2026-04-02: Horizon's last fill is an order-book trade at 0.16310; we stored
  5/34. That day is **only** this bug.
- `open` uses the same key and is wrong the same way.

## 4. Blast radius

**Across all SDEX data** (1d candles, ≥ 20 trades, `FINAL`):

| Source   | Candles   | close > 5 % off VWAP | …and an exact small ratio | close an exact small ratio | high > 50 % above VWAP | low > 33 % below VWAP |
| -------- | --------- | -------------------- | ------------------------- | -------------------------- | ---------------------- | --------------------- |
| sdex     | 6 118 543 | 2 216 213            | 237 641                   | 476 490 (7.8 %)            | 1 288 704 (21 %)       | 1 153 842 (19 %)      |
| aquarius | 22 290    | 1 404                | 1                         | 9                          | 388                    | 457                   |
| soroswap | 4 106     | 181                  | 0                         | 4                          | 27                     | 34                    |
| phoenix  | 2 226     | 174                  | 0                         | 0                          | 6                      | 4                     |

(close vs the _day_ VWAP is a loose test — a volatile token legitimately
closes away from its average; the ratio column is the sharp one.)

**XLM against Bitstamp XLM/USD, 2 048 days:** close off by > 5 % on 7 days
(> 10 % on 5); high above 1.5× Bitstamp's high on 704 days (up to 13.8×),
low below 0.7× Bitstamp's low on 560 days.

**Propagation through the pivot tier** — XLM rate applied to XLM-quoted 1d
candles:

| Day        | Rate used                 | Bitstamp | Candles              |
| ---------- | ------------------------- | -------- | -------------------- |
| 2023-03-11 | 0.0588 (1/17)             | 0.0794   | 7 328, all ~26 % low |
| 2026-02-09 | median 0.1156             | 0.1595   | 3 399                |
| 2022-01-11 | median 0.258, max **1.0** | 0.263    | part of 2 939        |

Example, 2023-03-11: BTC/XLM closed at 259 130 XLM; stored `close_usd`
$15 243; at the market XLM rate it is ~$20 575.

## 5. How reference prices are done elsewhere

- **CME daily settlement** (equity index futures): VWAP of the trades in the
  last 30 seconds (14:59:30–15:00:00 CT); if none, the bid/ask midpoint. The
  last trade is deliberately not the settlement.
- **CME CF Bitcoin Reference Rate:** one-hour window split into 12 × 5-minute
  partitions; each partition's volume-weighted median trade price; the rate is
  the equally weighted average of the partitions.
- **CoinGecko:** per-minute ticker snapshots; drops DEX tickers under $1 000
  liquidity and stale tickers; drops tickers outside median ± 4 × MMAD (MAD ×
  1.4826); final price = VWAP of the rest. Fewer than 3 tickers: only a 100×
  jump is flagged, so a single-venue token is exposed the way we are. Their
  daily history is a snapshot of that aggregate, their OHLC is built from
  hourly snapshots — not from individual trades.

Common rule: a published price is a **volume-weighted statistic over a window**,
never one print, with too-small sources cut off first.

## 6. Candidate fixes, with measurements

### Layer 1 — at the source (ingest)

- **1a. Order key** `(ledger, tx_index, op_index, claim_index)`. Fixes `open`
  and `close` on every candle.
- **1b. Price-forming fills.** A fill sets `open/high/low/close` only if
  `1/amount_sold + 1/amount_bought ≤ 0.001` (rounding error ≤ 0.1 %, ≈ both
  legs ≥ 2 000 stroops). Every fill still counts in `volume_*`, `vwap` and
  `trade_count`. Derived from Stellar arithmetic, not tuned; for Soroban tokens
  the same rule in each token's integer base units.
- **1c. A minute with no price-forming fill** keeps its volume but must not
  set price in the rollups (e.g. a `price_trade_count` column and
  `argMaxIf/maxIf/minIf … price_trade_count > 0` in `rollups.sql`). Open
  design question.

### Layer 2 — what "close" means (CME-style)

Close of 1h and coarser = VWAP of the last minute that traded. Measured on
1 181 days of XLM/USDC with 1m data, against Bitstamp's close:

| Close definition                                        | Median error | p99        | Worst day | Days > 5 % | Days > 10 % |
| ------------------------------------------------------- | ------------ | ---------- | --------- | ---------- | ----------- |
| today: last fill, broken order                          | 0.15 %       | 2.47 %     | **280 %** | 5          | 4           |
| **VWAP of the last minute that traded**                 | **0.15 %**   | **1.11 %** | **4.5 %** | **0**      | **0**       |
| VWAP of the last 5 min                                  | 0.15 %       | 1.21 %     | 5.8 %     | 1          | 0           |
| VWAP of the last 15 min                                 | 0.16 %       | 1.48 %     | 5.8 %     | 1          | 0           |
| VWAP of the last 60 min                                 | 0.25 %       | 2.31 %     | 6.5 %     | 3          | 0           |
| BRR-style (12 × 5 min, weighted median, averaged)       | 0.25 %       | 2.12 %     | 7.1 %     | 3          | 0           |
| last fill unless > 2 % from 15-min VWAP, then that VWAP | 0.15 %       | 1.10 %     | 5.8 %     | 1          | 0           |

Longer windows get worse because the market moves within them; the shortest
volume-weighted window wins. Day-level medians/VWAPs are not closes at all
(median error ~1 %, 97–158 days > 5 %) — they answer "typical price of the
day", not "price at the end of it".

### Layer 3 — high and low

From price-forming fills only (layer 1b). Proxy measured from 1m data —
extreme minute VWAP over minutes worth ≥ 0.0001 USDC:

|                                                  | Days > 10 % off Bitstamp | Median error |
| ------------------------------------------------ | ------------------------ | ------------ |
| high: today (max of all fills)                   | 534                      | 5.5 %        |
| high: extreme minute VWAP, minutes ≥ 0.0001 USDC | **2**                    | **0.16 %**   |
| low: today                                       | 598                      | 10.3 %       |
| low: extreme minute VWAP, minutes ≥ 0.0001 USDC  | **0**                    | **0.16 %**   |

Raw `max(high)` restricted to liquid minutes is still bad (488 days > 10 %):
the dust sits _inside_ normal minutes, so a complete high/low fix has to be at
the fill level.

### Layer 4 — stop the spread

- Pivot reference = bucket VWAP (`Σvolume_quote / Σvolume_base`) instead of
  the volume-weighted close.
- Read path: when sources disagree, take the cross-source median, not the
  largest-volume leg.
- Safety net: flag a candle when `|close / vwap − 1|` exceeds k × its recent
  robust volatility; expose the count as a metric.

## 7. History

1. **Re-ingest from ledgers** with the fixed ingest (the `sdex-backfill`
   machinery exists; `backfill_sdex_ledgers` tracks 63 M ledgers). Full
   fidelity, large cost.
2. **Candle-level repair.** Where 1m survives (XLM/USDC: 1 188 of 2 048 days),
   recompute close/high/low from minute VWAPs. Where the cleanup worker dropped
   1m (2025-02 → 2026-02), only 1h+ exists: keep the close unless it is > 3 %
   from the last hour's VWAP, then use that VWAP (measured: worst day 6.1 %,
   none > 10 %).

## 8. Limits of this analysis

- Measured on XLM/USDC, the most liquid market. Illiquid assets need their
  own measurement: their last minute may hold one fill, and it may be the dust.
- Fill-level evidence (Horizon) exists only for the last ~year
  (`history_elder_ledger` 58 068 721); older fills are inferred from 1m candles.
- 0.1 % is a choice. The data says fills above ~1 000 stroops are
  indistinguishable from large ones.
- The Soroban AMM extractors (`aquarius`, `soroswap`, `phoenix`) were not
  audited for the same order-key issue.

## 9. How it was measured

- **Exact-ratio test** (ClickHouse): `arrayExists(k -> abs(toFloat64(close) * k - round(toFloat64(close) * k)) < 1e-9 * k AND round(toFloat64(close) * k) BETWEEN 1 AND 200, range(1, 201))`.
- **Size vs deviation:** `price_ohlcv_1m FINAL`, `trade_count = 1`, smaller leg
  = `least(volume_base, volume_quote) × 1e7`, deviation from the hour's
  Σquote/Σbase.
- **Horizon:** ledger at 00:00 UTC found by bisection on `/ledgers/{seq}`,
  then `/trades?…&order=desc&cursor=<ledger << 32>`.
- **Bitstamp:** `/api/v2/ohlc/xlmusd/?step=86400`; USDC → USD from
  task 0265's `composed_usdc_usd_1d.csv`.
- Close candidates computed per day from `price_ohlcv_1m FINAL`,
  `source = 'sdex'`, XLM/USDC.

## Sources

- [CME Group — Understanding Equity Index Daily & Final Settlement](https://www.cmegroup.com/education/courses/introduction-to-equity-index-products/understanding-equity-index-daily-and-final-settlement)
- [CFTC filing — CME S&P & E-Mini S&P 500 Futures Daily Settlement Procedure](https://www.cftc.gov/sites/default/files/filings/orgrules/18/01/rule012618cbotdcm002.pdf)
- [CME CF Bitcoin Reference Rate Methodology Guide](https://www.cmegroup.com/trading/files/bitcoin-reference-rate-methodology.pdf)
- [CF Benchmarks — CME CF Reference Rates Methodology](https://docs.cfbenchmarks.com/CME%20CF%20Reference%20Rates%20Methodology.pdf)
- [CoinGecko Price Aggregation Methodology v3.0 (2026-01-26)](https://assets.coingecko.com/methodology/CoinGecko-Price-Aggregation-Methodology.pdf)
- [CoinGecko API — Coin OHLC Chart by ID](https://docs.coingecko.com/reference/coins-id-ohlc)
