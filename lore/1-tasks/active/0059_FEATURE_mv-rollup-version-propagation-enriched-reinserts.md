---
id: "0059"
title: "MV rollup-chain version propagation under enriched `_1m` re-inserts"
type: FEATURE
status: active
related_adr: ["0007"]
related_tasks: ["0026", "0051"]
tags: [layer-database, priority-high, effort-medium, clickhouse, materialized-views, rollups]
links:
  - "../../../docs/database-schema/database-schema-overview.md"
  - "../blocked/0026_FEATURE_volume-quote-usd-enrichment-impl/notes/G-local-prototype-spec.md"
  - "./0051_FEATURE_clickhouse-prices-schema-and-mv-chain-migration.md"
history:
  - date: 2026-06-09
    status: backlog
    who: claude
    note: >
      Spawned from 0026 future work. The 0026 enrichment Lambda
      corrects volume_quote_usd by re-INSERTing _1m rows with
      version+1. That re-insert re-fires the MV rollup chain, raising
      open questions about how _15m..._1M re-aggregate and what version
      the MVs project onto their ReplacingMergeTree targets. 0026
      scoped itself to _1m only and flagged this as a 0051 dependency.
  - date: 2026-06-09
    status: active
    who: okarcz
    note: >
      Promoted from backlog to active. Fixed the stale 0026 link
      (active -> blocked). Note: real progress is gated on 0051
      landing the MV rollup-chain DDL — the SELECT/GROUP BY/projected
      version this task verifies does not exist yet.
---

# MV rollup-chain version propagation under enriched `_1m` re-inserts

## Summary

The 0026 enrichment Lambda re-INSERTs corrected `price_ohlcv_1m` rows
with `version = original + 1`. Each such INSERT re-fires the MV rollup
chain (`_1m → _15m → … → _1M`, which `sum()`s `volume_quote_usd`). This
task verifies — and fixes if needed — that the rolled-up granularities
end up with the *enriched* values rather than the stale `0`-contribution
rows, and that the MV-projected `version` makes the corrected rollup row
win its `ReplacingMergeTree` merge.

## Context

Task 0026 deliberately enriches `_1m` only and left rollup correctness to
0051 (which owns the MV chain DDL). The open risks:

- A summing MV fires on the *inserted block* (the single re-inserted
  `_1m` row), not a re-read of the whole bucket — so it may emit a
  partial-sum `_15m` row that needs to combine with, not replace, the
  existing one. On a `ReplacingMergeTree` target this can double-count
  or under-count depending on the MV's `version` projection.
- What `version` the MV assigns to its target rows determines whether
  the corrected rollup row wins over the original `0`-contribution row.

## Implementation

- Pin down the exact MV DDL (`SELECT` shape, `GROUP BY`, projected
  `version`) for each step of the chain in task 0051.
- Decide the correct engine/semantics for rollup targets so an enriched
  `_1m` re-insert propagates correctly (candidates: project
  `max(version)`/`maxState`, or restructure as `AggregatingMergeTree`).
- Add an integration test: write a `_1m` row at `volume_quote_usd = 0`,
  let the chain roll up, run 0026 enrichment, and assert every
  granularity reflects the enriched value after `FINAL`.

## Acceptance Criteria

- [ ] MV chain projects a `version` that lets an enriched `_1m`
      re-insert win at every rolled-up granularity
- [ ] No double-count / under-count of `volume_quote_usd` in `_15m … _1M`
      after an enrichment pass
- [ ] Integration test covering write → roll up → enrich → assert across
      all granularities (`FINAL`)
- [ ] 0026 G-note dependency note resolved / cross-linked
