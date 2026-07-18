---
id: "0075"
title: "Update DB schema overview for newer prices.* tables (unresolved_pools, discovery_state, backfill_progress columns)"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0053", "0054", "0073"]
tags: ["phase-future", "effort-small", "priority-medium", "docs"]
links:
  - "../../../docs/database-schema/database-schema-overview.md"
history:
  - date: 2026-07-02
    status: backlog
    who: claude
    note: >
      Spawned from 0053. While documenting prices.pool_registry (§3.6) in the
      general DB schema overview, noticed several newer prices.* tables are
      missing or stale there. Do this once 0053 lands so the doc reflects the
      final schema.
---

# Update DB schema overview for newer prices.* tables

## Summary

Bring `docs/database-schema/database-schema-overview.md` up to date with the
`prices.*` tables added/changed since it was last refreshed. `pool_registry`
was documented under 0053 (§3.6); the remaining gaps are catalogued below.

## Context

The general DB doc documents §3.1–§3.5 (assets, price_ohlcv, current_prices,
oracle_prices, backfill_progress) but predates several tables/columns. Do this
**after 0053 completes** so the doc captures the final `backfill_progress`
shape.

## Implementation

- **`prices.unresolved_pools` (task 0053)** — add a §3.x section: DDL +
  role (per-`(contract_id, source)` record of swaps dropped for an unregistered
  pool; empty on a clean forward-discovery run; a still_unresolved=1 row is an
  extractor gap). Add a §5 sort-key row (`ORDER BY (contract_id, source)`).
- **`prices.discovery_state` (task 0054)** — add a §3.x section: DDL + role
  (asset-discovery high-water-mark, one row per worker). Add a §5 sort-key row.
- **`prices.backfill_progress` (§3.5) refresh** — its DDL predates the
  `earliest_data_available` and `newest_data_available` columns (task 0053 /
  0073); add them and note the covered-time-window semantics.
- Re-check the §3.0 entity-relationship diagram and the §2 storage-engine
  summary for the same tables while here.

## Acceptance Criteria

- [ ] `unresolved_pools` and `discovery_state` each have a §3.x DDL + role
      section and a §5 sort-key row.
- [ ] §3.5 `backfill_progress` DDL includes `earliest_data_available` +
      `newest_data_available` with a one-line semantics note.
- [ ] No remaining `prices.*` table in `init.sql` is absent from the overview.
