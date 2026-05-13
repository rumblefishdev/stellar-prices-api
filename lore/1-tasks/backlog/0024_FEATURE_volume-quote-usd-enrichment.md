---
id: "0024"
title: "volume_quote_usd enrichment pass: join price_ohlcv with oracle_prices for non-USD-quoted pairs"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0022", "0012"]
tags: [priority-medium, effort-medium, ohlcv, enrichment, oracle, backfill, stream-2]
links:
  - "../active/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md"
  - "../../../docs/database-schema/database-schema-overview.md"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-05-13
    status: backlog
    who: claude
    note: >
      Spawned from 0022 future-work item 2. Decode spec §5.3 + §6
      item 2 deferred this to a follow-up pass; the backfill
      writes volume_quote_usd = 0 until enrichment runs.
---

# `volume_quote_usd` enrichment pass

## Summary

The SDEX backfill (task 0022 / 0012) writes `volume_base`
authoritatively but leaves `volume_quote_usd = 0` whenever the
quote-side reference price in USD is not directly available.
Implement an enrichment pass that fills in `volume_quote_usd` by
joining `price_ohlcv` against `oracle_prices` at minute
granularity.

## Context

For pairs where `quote ∈ {USDC, USDT}`, enrichment is approximately
the identity (USDC/USDT are pegged to ~$1). For pairs where
`quote = native XLM` (the bulk of SDEX volume — task 0022 profile
showed XLM-quoted pairs are common), enrichment requires the
`XLM/USD` oracle price at the minute bucket.

Schema is already in place:

- `price_ohlcv.volume_quote_usd` `NUMERIC(28,14) NOT NULL DEFAULT 0`
- `oracle_prices` table holds Reflector / Chainlink / RedStone
  / Band quotes, indexed by `(timestamp, asset_id, oracle_name)`.

The pass joins these, computes
`volume_quote_usd = price_ohlcv.volume_quote * oracle_price`,
and UPSERTs the result back to `price_ohlcv`.

## Implementation

- Decide enrichment trigger: hourly cron Lambda, end-of-backfill
  one-shot, or live writer extension. Live extension is simplest
  but couples the Prices Ledger Processor to oracle availability;
  cron Lambda decouples and lets backfill complete without
  oracle gating.
- Define the SQL update (or Rust task) joining `price_ohlcv` with
  `oracle_prices` for `volume_quote_usd = 0` rows.
- Handle minute-bucket alignment between `price_ohlcv` (1m
  candles) and `oracle_prices` (5m oracle fetch cadence).
  Recommended: forward-fill oracle price to 1m granularity within
  the same 5m window.
- Idempotent: re-running the pass on already-enriched rows must
  produce identical values.

## Acceptance Criteria

- [ ] Enrichment pass implemented (Lambda or backfill task
      extension — TBD in design).
- [ ] `current_prices.volume_24h_usd` reflects SDEX trades from
      XLM-quoted pairs (verify: pick an XLM-quoted pair like
      SCOP/XLM, confirm USD volume is non-zero in `current_prices`
      after enrichment runs).
- [ ] Idempotency test: enrichment over already-enriched data is
      a no-op.
- [ ] Documented behaviour for pairs with **no** oracle reference
      (e.g. exotic pair where neither side has oracle coverage):
      leave `volume_quote_usd = 0` and add a metric to track
      missing-oracle rate.
