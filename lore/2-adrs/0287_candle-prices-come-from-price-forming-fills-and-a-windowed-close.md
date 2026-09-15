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
  - date: "2026-09-15"
    status: accepted
    who: akot
    note: >
      Amended after a specification review against the code (0286 branch):
      the window is anchored at the last price-forming minute, not the
      bucket end; a 1m candle has open = close by construction; rollups use
      maxIf/minIf and carry no high/low; coarse close_usd is close × the
      latest priced child's rate; a missing 1m tail falls back to the child's
      close with close_window_fills = 0; the median is a stored column
      (close_median); the pool price is the post-fill reserves; one column
      name (pf_trade_count); 0142 and 0137 are live; 0228's reset campaign
      becomes unnecessary after the re-ingest. No decision reversed.
  - date: "2026-09-15"
    status: accepted
    who: akot
    note: >
      Second amendment, decided by Adam after a review against the rollup
      and enrichment code: (1) coarse open/close are computed once by a
      settle pass into price_ohlcv_settle and joined by the MVs — option (b)
      of three — because MVs reading 1m tails would scan up to 400 days per
      refresh and degrade settled closes after 1m retention; (2) the
      enrichment candidate predicate excludes close_usd = 0 rows whose close
      is 0, which the stateless ingest writes for dust-only minutes; (3) a
      pool fill's price is its execution ratio with the 0.1 % bound, not the
      post-fill reserves — the reserve-based price was unmeasured and prices
      large swaps at a price nobody paid.
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

1. **A fill's price is its execution price.** For an order-book fill that
   is the resting offer's `N/D` (the price the maker set; rounding touches the
   amounts, not the price), so an order-book fill always forms price. For a
   pool fill (classic LP or Soroban AMM) there is no nominal price, and the
   execution price is the amount ratio; **a pool fill forms price only if its
   rounding bound `1/amount_sold + 1/amount_bought ≤ 0.001` holds**,
   evaluated on the raw integer amounts in each asset's own base units
   (stroops on classic, the token's decimals on Soroban). Pool dust is
   therefore excluded, not re-priced: measured, it carries 0.0000 % of volume.
   The pool's reserves are not used — a post-fill spot is where the pool
   ended, not what anyone paid, and would put untraded prices into VWAP and
   `high`/`low`. Until the offer price is read (0286 phase 2) order-book
   fills are ratio-priced too and take the same bound. Every fill still
   counts in `volume_base`, `volume_quote`, `vwap` and `trade_count`.
2. **`close` = the volume-weighted mean price of price-forming fills over a
   window of whole minutes anchored at the last minute of the bucket that
   holds a price-forming fill, extended backwards until the window holds
   ≥ 3 such fills, but never more than 60 minutes before that anchor minute
   and never before the bucket start; fewer fills if that is all the window
   can reach.** `open` is the mirror image, anchored at the first
   price-forming minute and extended forwards. On the 1m tier the window is
   the minute itself, so **a 1m candle has `open = close` = its own
   price-forming VWAP** (a doji by construction); the first/last print is
   not published anywhere.
3. **`high` / `low` = the extremes of price-forming fills**, on every tier;
   rollups take `maxIf(high, pf_trade_count > 0)` / `minIf(low, …)` so a
   minute without a price-forming fill contributes nothing.
4. **One definition on every tier, computed once by a settle pass.** 1m,
   15m, 1h, 4h, 1d, 1w and 1M all use 1–3. The 1m tier computes its own
   `open = close` in the ingest. For every coarser tier, a **settle pass**
   (enrichment-worker, beside the coarse sweep) computes `open`, `close`,
   `open_window_fills`, `close_window_fills` and `close_median` from the 1m
   rows of the bucket **once, after the bucket has closed**, and writes them
   to `prices.price_ohlcv_settle` (key: tier, asset, quote, source, bucket;
   `ReplacingMergeTree(settled_at)`). It re-settles a bucket only if the 1m
   rows its windows read have changed since (a late write or reconcile —
   tracked by the sum of their `version`s) and **never writes when those 1m
   rows are gone**, so a settled close cannot be degraded by 1m retention.
   The rollup MVs do not read 1m: they take `open`/`close` and the window
   columns from `price_ohlcv_settle` by key, and `high`/`low`/volumes from
   the child tier as today. Their re-aggregation windows (up to 400 days on
   1M) therefore stay cheap and deterministic. **A bucket without a settle
   row** (still open, not yet settled, or history whose 1m never existed)
   falls back to the child tier's `argMaxIf(close, timestamp,
   pf_trade_count > 0)` / `argMinIf(open, …)` with `close_window_fills = 0`,
   so a consumer can tell a windowed close from a carried one; the next MV
   refresh after the settle pass replaces the fallback.
5. **`close_usd` on a coarse tier is `close × rate`, where the rate is the
   latest priced child's `close_usd / close`** (`argMaxIf(close_usd / close,
   timestamp, close_usd > 0)`). The child's rate at the bucket end is the
   end-of-bucket reference the pivot would apply; carrying the *product*
   from the child, as the rollups do today, would price the new windowed
   close with the child's close. A bucket with no priced child writes 0 and
   the enrichment worker's coarse pass prices it, as it does today.
6. **A minute with no price-forming fill** keeps its volume and trade
   count, is written with `pf_trade_count = 0` and price fields 0 (the
   ingest keeps no state across minutes or backfill chunks), is skipped by
   every rollup aggregate through the `pf_trade_count > 0` condition, and is
   shown by the read surface with `open`/`close` carried forward from the
   last price-forming candle and `pf_trade_count = 0` exposed. `high`/`low`
   are never carried: a carried extreme would leak into the next bucket's
   `max`/`min`. **Enrichment treats such a row as priceable for volume
   only:** its candidate predicate becomes
   `(volume_quote_usd = 0 OR (close_usd = 0 AND close > 0)) AND volume_quote > 0`,
   so `close_usd = 0` with `close = 0` means "no price to convert", not
   "not yet enriched" — otherwise every such row would be re-selected and
   rewritten on every pass.
7. **The mean is the volume-weighted mean, not the amount ratio.** Close is
   `Σ pᵢ·vᵢ / Σ vᵢ` over price-forming fills with pᵢ the fill's price from
   (1); `volume_quote / volume_base` is the amount-derived price and must not
   be used for it. The volume-weighted median over the same window is stored
   alongside as `close_median` on every tier (1m: median of price-forming
   fill prices weighted by volume, computed in the ingest; coarse: median of
   the window's minute price-forming VWAPs weighted by `pf_volume`); the
   read surface flags a candle when `|close / close_median − 1|` exceeds 1 %.
8. **The existing fields are redefined; no `settle` field is added on the
   wire.** (`price_ohlcv_settle` is an internal table feeding the rollups,
   not an API field.)

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
  `pf_price_volume`; every tier gains `close_median`, `open_window_fills`
  and `close_window_fills`; a new table `price_ohlcv_settle` holds the
  settled windows of the coarse tiers. Rollups join it by key and use
  `argMaxIf`/`maxIf`/`minIf` on `pf_trade_count > 0`. The six rollup MVs are
  re-created; [[0142]] (drift detection) and [[0137]] (freshness alarm) are
  already live, so the only other change sharing that DROP window is
  [[0146]]. Every writer of `price_ohlcv_1m` (live ingest, the backfill
  sink) is positional and changes in the same commit.
- **Latency:** a coarse bucket shows the fallback close from its end until
  the next settle pass plus the next MV refresh (minutes on 15m–4h, up to a
  day on 1d–1M, whose MVs refresh daily). The fallback is the child's
  price-forming close, never a dust print.
- **Enrichment:** the candidate predicate change (§6) is part of this
  decision, not of the pivot's two-reference change.
- **Pivot:** once `close` is dust-free it is again the right end-of-bucket
  reference for `close_usd`; `volume_quote_usd` needs the bucket VWAP
  instead (two references — analysis doc §10.3, outside this ADR).
- **History:** candles written before the change keep the old semantics until
  [[0286]] phase 3 re-ingests the chain in place. Re-ingested rows carry
  `close_usd = 0`, so the enrichment worker re-prices the whole history
  afterwards — which makes [[0228]]'s reset-mode campaign (Appendix C)
  unnecessary: its refill happens as part of the re-enrichment, and only its
  after-check still applies. That re-enrichment is a cost of phase 3.
- **Measured basis and its limit:** k = 3 was measured on yXLM/XLM, a pegged
  pair with no drift inside the window; on XLM/USDC k = 3 is one minute
  anyway (~24 fills a minute). The window's behaviour on mid-liquidity,
  trending pairs is the one thing the measurements do not cover; the
  first-week measurement in 0286 is where it is checked.
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
  fills whose error is largest relative to the noise floor. §1 keeps its
  shape with a 0.1 % bound.
- **Pool price from the reserves after the fill.** Rejected: it is the
  pool's marginal price after the trade, not an execution price; a large
  swap would enter VWAP and `high`/`low` at a price nobody traded at. Above
  the bound the amount ratio is already exact, and below it the fill carries
  no volume worth keeping.
- **Rollup MVs reading the 1m tails directly.** Rejected: the anchor can sit
  anywhere in a thin pair's bucket, so each refresh of the 1w (60-day) and
  1M (400-day) windows would scan months of 1m; and once 1m ages out of
  retention the same refresh would recompute a settled close from the
  fallback and, via APPEND + `sum(version)`, could overwrite it.
