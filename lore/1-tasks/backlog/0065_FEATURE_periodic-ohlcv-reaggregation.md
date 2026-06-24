---
id: "0065"
title: "Periodic OHLCV re-aggregation for cross-chunk intra-minute candles"
type: FEATURE
status: backlog
related_adr: ["0004", "0007"]
related_tasks: ["0038", "0039"]
tags: [layer-indexing, priority-medium, effort-medium, clickhouse, ohlcv]
links:
  - "../archive/0048_RESEARCH_soroban-events-pricing-decoder-spec/notes/G-soroban-events-pricing-decoder.md"
history:
  - date: 2026-06-24
    status: backlog
    who: oski
    note: "Spawned from 0038 future work (cross-invocation intra-minute merge gap)."
---

# Periodic OHLCV re-aggregation for cross-chunk intra-minute candles

## Summary

Close the intra-minute aggregation gap shared by **both** writers: the live
Lambda (per contiguous run) and the backfill (per partition) accumulate
candles in memory and flush per chunk. When a single minute spans two
chunks/invocations, two rows land with the same PK but different `version`,
and `ReplacingMergeTree(version)` keeps only the latest — dropping the other
chunk's trades for that minute.

## Context

`price_ohlcv_1m` is `ReplacingMergeTree(version)` keyed by
`(asset_id, quote_asset_id, source, timestamp)`. RMT **replaces**, it does
not sum — so per-chunk partial candles for a boundary minute don't merge.
Negligible-but-real (one minute per chunk boundary). Same root cause for live
and backfill since both now use `prices-ingest-core`'s `CandleAccumulator`.

## Implementation (options to evaluate)

- A periodic worker (task 0039 family) that re-reads raw trades/`_1m FINAL`
  and rewrites boundary minutes with a higher `version`; OR
- An `AggregatingMergeTree` / SummingMergeTree variant for the write path so
  partial candles combine on merge; OR
- Emit candles keyed to include a chunk discriminator and re-roll at read.

## Acceptance Criteria

- [ ] A minute split across two runs/chunks aggregates to one correct candle.
- [ ] Fix applies to both live (0038) and backfill writers (shared core).
- [ ] Regression test with a deliberately split-minute fixture.
