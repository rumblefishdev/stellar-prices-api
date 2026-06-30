---
id: "0072"
title: "Materialize current_prices v1-deferred columns (sources breakdown, price_xlm, change_24h_pct) in the MV"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0040", "0039"]
tags: ["phase-future", "effort-medium", "priority-medium"]
links: []
history:
  - date: 2026-06-30
    status: backlog
    who: claude
    note: >
      Spawned from 0040 future work. The prices-api /price endpoint ships in
      0040 with sources={} (and price_xlm / change_24h_pct as zero stubs)
      because mv_current_prices (current.sql, task 0039) writes only the v1
      subset. This task materializes the deferred columns producer-side so the
      API upgrades to pass-through without adding a hot-path query.
---

# Materialize current_prices v1-deferred columns in the MV

## Summary

`prices.mv_current_prices` (the sole writer of `current_prices`, task 0039)
currently populates only the v1 subset
(`price_usd, volume_24h_usd, vwap_24h, market_cap_usd, updated_at`). Four
columns are left at their table DEFAULTs: `price_xlm` (0), `change_24h_pct` (0),
`change_7d_pct` (0), and `sources` ('') — see `current.sql:25-27`. Materialize
the columns the public API needs so `GET /assets/{id}/price` can return them by
pass-through instead of deriving them per request.

## Context

Parent task **0040** (Prices API Gateway + read handlers) ships `/price` with
these fields stubbed (decision 2026-06-30: "D now → A later" — stub in the API,
fill in the MV later) so the load-test endpoint stays a cheap point lookup. The
raw per-source data already exists in `price_ohlcv_1m` (it carries a `source`
column and per-source rows). The natural home for the per-source JSON breakdown
and the XLM-quote / reference-close derivations is the once-per-minute MV, NOT a
per-request API query (which would add a 24h scan + GROUP BY to the hottest
endpoint and undermine the p95<200ms SLO).

## Implementation

- **`sources` (JSON String)**: per-source breakdown over the trailing 24h —
  for each `source`, the latest close (`argMax(close_usd, timestamp)`) and the
  summed volume. Assemble into a JSON object string matching the §4.2 shape
  (`{ "sdex": { "price": ..., "volume_24h": ... }, ... }`). Build inside CH
  (e.g. `toJSONString(map(...))` from a per-source `groupArray` of tuples) so
  the MV stays the single writer.
- **`price_xlm`**: the XLM-quote orientation of the current price (current.sql
  comment §"follow-ups").
- **`change_24h_pct`** (and optionally **`change_7d_pct`**): the 24h/7d
  reference-close self-join.
- Decide: extend `mv_current_prices`'s SELECT + target column list in-place, or
  add a companion MV — weigh refresh cost vs. keeping a single ReplacingMergeTree
  row per asset (the supply worker already uses a separate table to avoid row
  contention; mirror that reasoning if the JSON build makes the 1-min refresh
  too heavy).
- Update `prices.current_price_usd` view (and any read surface) to forward the
  newly-populated columns.
- **0040 upgrade**: once columns are live, switch the `/price` handler from the
  `sources: {}` stub to pass-through.

## Acceptance Criteria

- [ ] `mv_current_prices` (or companion) writes `sources` as a valid JSON string
      matching the §4.2 `/price` `sources` shape.
- [ ] `price_xlm` and `change_24h_pct` are populated (non-DEFAULT) for assets
      with sufficient data; `change_7d_pct` decided (populate or document defer).
- [ ] Integration test vs prod-pinned CH 26.3.10.60 asserts a seeded multi-source
      asset yields the expected per-source breakdown + scalar fields.
- [ ] `current_price_usd` view (or read surface) exposes the new columns.
- [ ] 0040 `/price` handler switched to pass-through; `sources` stub removed.
