---
id: "0287"
title: "Candle `open`/`close` are the first and last price-forming fills of the bucket and `high`/`low` their extremes — one definition on every tier, built only from the bucket's own trades"
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
  - date: "2026-09-16"
    status: accepted
    who: akot
    note: >
      Third amendment, decided by Adam: a candle is built only from the
      price-forming trades of its own bucket, 1:1 with what traded. That
      reverses three of 0278's decisions. D5/D6 (open/close as a windowed
      VWAP) made every 1m candle a doji and every coarse close an average of
      up to an hour of trades — a price nobody paid. D4 (carrying open/close
      forward into a bucket with no price-forming fill) put a price from
      outside the bucket on the wire. D8's shape flips: `close` is the last
      price-forming fill and the VWAP becomes the auxiliary `pf_vwap` field —
      the "settle field" rejected on 2026-09-15 as "two truths". open/close
      are the first and last price-forming fills in fill order, high/low
      their extremes, and a bucket without one publishes no price. The
      settle pass, price_ohlcv_settle, close_median and the window columns
      are gone; the rollups take open/close from the child tier again. D1
      (transaction index) is load-bearing again and moves to phase 1. Kept:
      D2/D3 (price source, the 0.1 % bound), D7 (one definition per tier),
      the rate-based coarse close_usd, the enrichment predicate, D10. The
      file name keeps its 2026-09-15 slug so links from 0278 and 0286 hold.
---

# Candle prices come from price-forming fills

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

The defect is identifiable per fill: the rounding error of a ratio-priced
fill is bounded by `1/amount_sold + 1/amount_bought`, and an order-book fill
has an exact price — the resting offer's — that the amounts merely round.
The fix is therefore a filter at the source, not a statistic over the
result: a candle stays what a candle is — the trades of its own bucket — and
only the fills that carry no price information are kept out of its prices.

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
   ended, not what anyone paid. Until the offer price is read (0286 phase 2)
   order-book fills are ratio-priced too and take the same bound. Every fill
   still counts in `volume_base`, `volume_quote`, `vwap` and `trade_count`.
2. **`open` = the first price-forming fill of the bucket, `close` = the last,
   in fill order `(ledger, transaction_index, operation_index, claim_index)`.**
   Both are prices that traded, inside the bucket, never averaged and never
   taken from another bucket. On the 1m tier they are the minute's first and
   last price-forming fills; on every coarser tier they are the first
   price-forming child's `open` and the last price-forming child's `close`
   (`argMinIf(open, timestamp, pf_trade_count > 0)` /
   `argMaxIf(close, …)`), which is the same fill the bucket's own 1m rows
   would name. The transaction index is part of the definition: without it
   "last" is a different fill on 54/60 measured days.
3. **`high` / `low` = the extremes of price-forming fills**, on every tier;
   rollups take `maxIf(high, pf_trade_count > 0)` / `minIf(low, …)` so a
   minute without a price-forming fill contributes nothing.
4. **One definition on every tier, computed by the rollup chain.** 1m, 15m,
   1h, 4h, 1d, 1w and 1M all use 1–3; each tier is built from the tier below
   it, as today, with the `pf_trade_count > 0` conditions on every price
   aggregate. Nothing reads across bucket boundaries, nothing is settled
   later, nothing depends on 1m retention.
5. **`close_usd` on a coarse tier is `close × rate`, where the rate is the
   latest priced child's `close_usd / close`** (`argMaxIf(close_usd / close,
   timestamp, close_usd > 0)`). The child's rate at the bucket end is the
   end-of-bucket reference the pivot would apply; carrying the *product*
   from the child, as the rollups do today, would price the bucket's close
   with the child's close. A bucket with no priced child writes 0 and the
   enrichment worker's coarse pass prices it, as it does today.
6. **A bucket with no price-forming fill has no price.** The ingest writes
   the minute with its volume and trade count, `pf_trade_count = 0` and
   price fields 0 (it keeps no state across minutes or chunks); every rollup
   aggregate skips it through the `pf_trade_count > 0` condition; and the
   read surface returns the bucket with `open`/`high`/`low`/`close`/`vwap`
   absent and `pf_trade_count = 0` exposed — the value-or-absent rule of ADR
   0011 §5, applied to one more reason for absence. Nothing is carried from
   an earlier bucket. **Enrichment treats such a row as priceable for volume
   only:** its candidate predicate becomes
   `(volume_quote_usd = 0 OR (close_usd = 0 AND close > 0)) AND volume_quote > 0`
   — and every writing statement (oracle, peg, external, pivot) carries the
   same exclusion, because they do not read the shared predicate — so
   `close_usd = 0` with `close = 0` means "no price to convert", not "not yet
   enriched". The pivot's reference over XLM/USDC additionally ignores rows
   with `pf_trade_count = 0`, so a dust-only reference minute neither
   yields a 0 reference nor blocks the forward-fill.
7. **A quality field, not a second close.** Every tier carries
   `pf_trade_count`, `pf_volume` (Σ base volume of price-forming fills) and
   `pf_price_volume` (Σ price × base volume over them), summed up the chain;
   `pf_vwap = pf_price_volume / pf_volume` is the volume-weighted mean price
   of the bucket's price-forming trades, and the read surface flags a candle
   when `|close / pf_vwap − 1|` exceeds 1 %. `vwap` (Σ quote / Σ base over
   all fills) keeps its meaning and is never used for a price: that ratio
   *is* the amount-derived price this ADR filters.
8. **The existing fields keep their names and their meaning — first trade,
   last trade, extremes — over price-forming fills only.** `pf_trade_count`
   and `pf_vwap` are added to the wire (additive). No settle field, no
   settle table.

## Consequences

- **On the wire:** `open`, `high`, `low` and `close` remain prices that
  traded; what changes is the population — dust fills no longer take part.
  On the seven documented dust days the close moves to the market (on
  2026-02-09 the last fill inside the bound printed 0.15956 against
  Bitstamp's 0.1595; on 2026-04-02 the correctly ordered last fill is the
  order-book trade at 0.16310). A bucket whose only fills were dust is
  returned without prices. `low ≤ open, close ≤ high` holds by construction.
  Every 1m candle keeps its direction — the 2026-09-15 definition had made
  all of them dojis.
- **Thin markets:** on pairs with under 20 fills a day (84 % of SDEX
  asset-days) the close is one real trade, as it is today, now dust-free.
  A genuinely off-market trade on such a pair sets the close because it
  happened; `pf_vwap` and the divergence flag say so.
- **Ordering is load-bearing:** the transaction index (0278 D1) is part of
  the definition of `open`/`close` and ships in phase 1 of [[0286]], on
  every path — classic (`tx_processing` order), Soroban live (the
  transaction's apply index) and the events-sourced Soroban backfill (BE's
  `application_order`, with today's order as a logged fallback).
- **Schema:** `price_ohlcv_1m` and every rolled copy gain `pf_trade_count`,
  `pf_volume`, `pf_price_volume`, appended after `version`, with DEFAULT
  expressions (`trade_count`, `volume_base`, `volume_quote`) so pre-existing
  rows keep their old meaning until phase 3 re-ingests them. Rollups use
  `argMinIf`/`argMaxIf`/`maxIf`/`minIf` on `pf_trade_count > 0` and a
  Float64-safe `vwap`/`close_usd` (Decimal division silently overflows past
  ~1.7e10 on the production build). The six rollup MVs are re-created; the
  only other change sharing that DROP window is [[0146]]'s
  `argMaxIf(close_usd)`, superseded by §5. Every writer of a candle table —
  the name-routed Rust writer, the pre-roll scripts, every enrichment
  re-insert — carries the new columns in the same change: a re-insert that
  omits them takes the DEFAULT and declares a dust-only row price-forming.
- **Latency:** none added. A coarse bucket is right at its next refresh.
- **Pivot:** once `close` is dust-free it is again the right end-of-bucket
  reference for `close_usd`; `volume_quote_usd` needs the bucket VWAP
  instead (two references — analysis doc §10.3, outside this ADR).
- **History:** candles written before the change keep the old semantics until
  [[0286]] phase 3 re-ingests the chain in place. Re-ingested rows carry
  `close_usd = 0`, so the enrichment worker re-prices the whole history
  afterwards — which makes [[0228]]'s reset-mode campaign (Appendix C)
  unnecessary: its refill happens as part of the re-enrichment, and only its
  after-check still applies.
- **Measured basis and its limit:** the filter is measured (0278's R- note
  §4: offer-priced dust sits inside the noise floor, worst 1.4 % on ~8 000
  fills; ratio-priced dust up to 5 388 %) and so is the effect of ordering
  on the two Horizon-checkable dust days. The close error of "last
  price-forming fill" over the 1 181 XLM/USDC days was **not** measured —
  only the last-minute VWAP variant was (worst 4.5 %). [[0286]] measures it
  on the seven dust days before phase 3 declares them fixed, and on the
  first week of new data.
- **Not covered:** the cross-source rule on the read path (largest-volume leg
  today; a median across sources was deferred by 0278).

## Alternatives considered

- **`open`/`close` as a volume-weighted mean over a window of the bucket's
  last minutes (this ADR's own §2 of 2026-09-15; 0278 D5/D6).** Rejected by
  Adam on 2026-09-16: a mean is not a trade, every 1m candle became a doji
  and every coarse close an average of up to an hour; and the definition
  needed a settle pass over the 1m rows (its review found a catch-up that
  never retried and a rollback that rolled nothing back). The measured
  gain over the filter alone was inside the noise floor. The mean survives
  as `pf_vwap`, a quality field.
- **Carry `open`/`close` forward into a bucket with no price-forming fill
  (0278 D4).** Rejected on 2026-09-16: a price from outside the bucket.
  ADR 0011 §5 already returns a bucket without a price; one more reason for
  absence needs no new mechanism.
- **Keep `close` as the last print and add a `settle` field** (CME's shape).
  Rejected on 2026-09-15 as two truths for one field; on 2026-09-16 the
  roles settled the other way round — the trade is the truth, the mean is
  the auxiliary field.
- **Robust statistics alone (median of prints), no size filter.** Rejected:
  the rounding error is identifiable from fill size, so a filter is strictly
  better, and a median of a one-print window is that print.
- **Bucket VWAP as the close.** Rejected: it is the average price of the
  period, not the price at its end — 97–158 days > 5 % on 1d (analysis doc
  §6).
- **A fixed 30-second window.** Rejected: a ledger closes every ~5 s, so 30 s
  is 5–6 ledgers, often empty; and it cannot be back-tested from stored 1m
  data.
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
