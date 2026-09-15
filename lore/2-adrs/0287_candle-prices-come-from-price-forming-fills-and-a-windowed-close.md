---
id: "0287"
title: "Candle `open`/`close` are window VWAPs of price-forming fills and `high`/`low` their extremes — a single definition on every tier, replacing single-print OHLC"
status: accepted
deciders: [akot]
related_tasks: ["0278", "0286", "0276", "0266", "0228", "0146", "0116"]
related_adrs: ["0011"]
tags: [api, read-surface, contract, ohlcv, ingest, enrichment, data-correctness]
links:
  - "../1-tasks/archive/0278_RESEARCH_decide-whether-to-act-on-dust-prints-in-ohlcv/README.md"
  - "../1-tasks/archive/0278_RESEARCH_decide-whether-to-act-on-dust-prints-in-ohlcv/notes/S-close-estimator-and-pivot-reference.md"
  - "../1-tasks/archive/0278_RESEARCH_decide-whether-to-act-on-dust-prints-in-ohlcv/notes/R-measurements-2026-09-15.md"
  - "../../docs/ohlcv-outlier-prints-analysis.md"
  - "../../packages/prices-ingest-core/src/bucket.rs"
  - "../../packages/prices-clickhouse/schema/rollups.sql"
history:
  - date: "2026-09-15"
    status: accepted
    who: akot
    note: >
      Decided by Adam in [[0278]] (decisions D3–D8), on the measurements in
      that task's R- note: on 1 130 days of yXLM/XLM the windowed close takes
      the worst error from 5.56 % to 1.00 % with the median error unchanged;
      k = 1…3 are equivalent and k ≥ 5 worse; offer-priced dust fills sit
      inside the market's noise floor. Implemented by [[0286]].
---

# Candle prices come from price-forming fills and a windowed close

## Context

A candle's `open`, `high`, `low` and `close` are today single fills at equal
weight, and a fill's price is the ratio of two integer stroop amounts. A fill
of a few stroops prints an exact small fraction (1/17, 5/34) up to hundreds of
percent off the market; when it lands last in the bucket it is the close, and
through the pivot tier it becomes the USD reference for every XLM-quoted
asset in that bucket — 7 328 daily candles ~26 % low on 2023-03-11 (analysis
doc §4). `high`/`low` are worse: any dust fill anywhere in the bucket sets
them (704 / 560 of 2 048 XLM days > 50 % / 30 % off Bitstamp). The order key
also lacks the transaction index, so "last" is the wrong fill on 54/60 days.

Every published reference price elsewhere is a volume-weighted statistic
over a window with small sources cut off first (CME settlement, CME CF BRR,
CoinGecko); none is a single print. On Stellar we additionally know every
fill's size, which makes the rounding error identifiable per fill.

## Decision

1. **A fill forms price only if its price does not come from its own rounded
   amounts** — the resting offer's `N/D` for order-book fills, the pool's
   reserves for AMM fills — **or, when it does, its rounding bound
   `1/amount_sold + 1/amount_bought ≤ 0.001` holds.** Every fill still counts
   in `volume_base`, `volume_quote`, `vwap` and `trade_count`.
2. **`close` = the volume-weighted mean price of price-forming fills over the
   bucket's last minutes, taken back until they hold ≥ 3 such fills, capped
   at 60 minutes and at the bucket; fewer if that is all there is.** `open`
   is the mirror image from the bucket start. The 1m candle's close is its
   own price-forming VWAP.
3. **`high` / `low` = the extremes of price-forming fills**, on every tier.
4. **One definition on every tier.** 1m, 15m, 1h, 4h, 1d, 1w and 1M all use
   1–3; a coarser tier's `open`/`close` are computed from the 1m data of its
   bucket's tails, not from the child tier's `open`/`close`.
5. **A minute with no price-forming fill** keeps its volume, carries the
   price fields forward from the last price-forming minute, and says so with
   `price_trade_count = 0`.
6. **The mean is the volume-weighted mean, not the amount ratio.** Close is
   `Σ pᵢ·vᵢ / Σ vᵢ` over price-forming fills with pᵢ the fill's price from
   (1); `volume_quote / volume_base` is the amount-derived price and must not
   be used for it. The volume-weighted median over the same window is
   computed alongside as a quality flag.
7. **The existing fields are redefined; no `settle` field is added.**

## Consequences

- **On the wire:** `open` and `close` are no longer the first and last print;
  `high` and `low` no longer include dust prints. On liquid markets the
  typical difference is inside the noise floor (measured median error
  unchanged at ~0.005 %); on the seven documented dust days the close moves
  to the market. `low ≤ open, close ≤ high` holds by construction.
  Consumers who want the raw last print do not get it from `/ohlcv`.
- **Thin markets:** on pairs with under 20 fills a day (84 % of SDEX
  asset-days) the window usually holds one or two fills within the cap, so
  `close` is the last price-forming fill — dust-free but still one print.
  The 60-minute cap is a policy choice; it cannot be measured, because no
  reference exists for such pairs.
- **Schema:** `price_ohlcv_1m` gains `pf_trade_count`, `pf_volume`,
  `pf_price_volume`; rollups read them and use
  `argMaxIf(…, price_trade_count > 0)`. The six rollup MVs are re-created
  ([[0146]]'s procedure).
- **Pivot:** once `close` is dust-free it is again the right end-of-bucket
  reference for `close_usd`; `volume_quote_usd` needs the bucket VWAP
  instead (two references — analysis doc §10.3, outside this ADR).
- **History:** candles written before the change keep the old semantics until
  [[0286]] phase 3 re-ingests the chain in place; the pivot re-enrichment
  campaign of [[0228]] runs after that.
- **Not covered:** the cross-source rule on the read path (largest-volume leg
  today; a median across sources was deferred by 0278).

## Alternatives considered

- **Keep `close` as the last print and add a `settle` field** (CME's shape).
  Rejected by Adam: one field, one truth; every consumer gets the repair.
- **Robust statistics alone (median of prints), no size filter.** Rejected:
  the rounding error is identifiable from fill size, so a filter is strictly
  better, and a median of a one-print window is that print.
- **Bucket VWAP as the close.** Rejected: it is the average price of the
  period, not the price at its end — 97–158 days > 5 % on 1d (analysis doc
  §6).
- **A fixed 30-second window.** Rejected: a ledger closes every ~5 s, so 30 s
  is 5–6 ledgers, often empty; and it cannot be back-tested from stored 1m
  data.
- **Larger k or a Kalman filter.** k ≥ 5 admits stale prints (measured max
  error 3.4–4.2 % vs 1.0 % at k = 3); the Kalman filter is the optimal
  linear version of the same idea but not explainable as an API definition.
- **Horizon's approach** (offer price for order-book fills, a 10 % slippage
  filter on pool fills). Its close is right on the measured dust days, but its
  `high`/`low` still carry 5/34 and 1/10: a 10 % bound passes exactly the
  fills whose error is largest relative to the noise floor.
