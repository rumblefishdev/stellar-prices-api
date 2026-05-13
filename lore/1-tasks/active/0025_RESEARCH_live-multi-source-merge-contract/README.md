---
id: "0025"
title: "Live multi-source merge contract: source=sdex → 'aggregated' transition rules"
type: RESEARCH
status: active
related_adr: ["0003"]
related_tasks: ["0022", "0012", "0023", "0024"]
tags: [priority-medium, effort-small, ohlcv, live-ingestion, multi-source]
links:
  - "../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md"
  - "../archive/0023_RESEARCH_ohlcv-row-identity-base-vs-pair/notes/S-recommendation.md"
  - "../archive/0024_FEATURE_volume-quote-usd-enrichment/notes/G-enrichment-pass-design.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../../docs/database-schema/database-schema-overview.md"
history:
  - date: 2026-05-13
    status: backlog
    who: claude
    note: >
      Spawned from 0022 future-work item 3. Decode spec §5.6 sketched
      the multi-source merge pattern but explicitly scoped the live
      writer-side contract out.
  - date: 2026-05-13
    status: active
    who: okarcz
    note: >
      Promoted to active. Scope is design-only — produces a G-note
      specifying the writer-side multi-source merge contract.
      Implementation lives in the Prices Ledger Processor when task
      0012 lands. ADR 0003's `quote_asset_id` PK makes the merge
      collision well-defined (per-native-pair, per-minute).
---

# Live multi-source merge contract

## Summary

The schema doc commits to a `source = 'aggregated'` row when a
1-minute candle for `(asset, minute, '1m')` is touched by multiple
distinct sources (SDEX, Soroswap, Aquarius, …). Task 0022's
decode-and-bucket spec covered the **backfill** contract (write
`source = 'sdex'` for SDEX-only rows; trust downstream to merge);
this task specs the **live writer-side** contract.

## Context

Per `docs/database-schema/database-schema-overview.md` §"Source
attribution":

> When the same `(timestamp, asset, granularity)` is written by
> multiple distinct sources … the writer uses `source =
> 'aggregated'` and merges across sources. Single-source candles
> keep their original source label.

Open questions:

1. **Who initiates the merge?** Three options:
   - Each live writer detects `source != self.source` on UPSERT
     conflict and rewrites to `'aggregated'`.
   - A separate Current Price Updater–adjacent process scans
     `price_ohlcv` and consolidates multi-touched rows.
   - The Rollup Lambda merges when it re-aggregates from 1m.

2. **What numeric merge happens?** Sum volumes? Volume-weighted
   re-aggregation of OHLC from the underlying ticks (requires
   ticks to be persisted, which 0022's backfill does not do)?
   Argmin/argmax for open/close across sources by their internal
   timestamps?

3. **Is the source attribution losslessly recoverable?** Once a
   row is `'aggregated'`, can downstream consumers see which
   constituent sources contributed? (The `sources` JSONB on
   `current_prices` carries per-source breakdown but
   `price_ohlcv` doesn't.)

## Implementation

- Survey: identify the consumer paths that read `source` from
  `price_ohlcv`.
- Decide who initiates the merge.
- Spec the numeric merge formula (likely: each source contributes
  its own complete candle; the aggregated row stores
  volume-weighted everything across sources, with open/close
  determined by source-internal timestamp).
- Decide whether to persist constituent source list per row
  (extra `sources_seen JSONB` column?) or rely on `current_prices`
  for that information.

## Acceptance Criteria

- [ ] Specification note in this task's `notes/` directory.
- [ ] Schema implication (new column? new table?) documented and
      either implemented or spawned as a separate schema task.
- [ ] Live writer implementation guidance for the Prices Ledger
      Processor (task 0012's eventual sibling).
