---
id: "0075"
title: "Update DB schema overview for newer prices.* tables (unresolved_pools, discovery_state, backfill_progress columns)"
type: DOCS
status: active
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
  - date: 2026-08-04
    status: active
    who: akot
    note: >
      Promoted from backlog. Blocking deps are all archived (0053, 0054, 0073),
      so backfill_progress has its final shape and the doc can be brought up to
      date against it.
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

- [x] `unresolved_pools` and `discovery_state` each have a §3.x DDL + role
      section and a §5 sort-key row. — §3.7 and §3.8.
- [x] §3.5 `backfill_progress` DDL includes `earliest_data_available` +
      `newest_data_available` with a one-line semantics note. — DDL plus a
      "Covered time-window" subsection contrasting the pair with the
      ledger-directional `current_ledger`.
- [x] No remaining `prices.*` table in `init.sql` is absent from the overview.
      — the four beyond the task's original list are covered too: §3.9
      `asset_metadata`, §3.10 `asset_supply`, §3.11 `backfill_sdex_ledgers`,
      §3.12 `ingest_cursor`.

## Implementation Notes

`docs/database-schema/database-schema-overview.md`, +328/−20.

- **New sections §3.7–§3.12** — six tables, each DDL + role. Sourced from
  `packages/prices-clickhouse/schema/init.sql` (the source of truth), not from
  memory of the design docs.
- **§3.5 refresh** — the two columns plus the covered-time-window semantics:
  written by the backfill as it lands candles, read O(1), never a live
  `MIN`/`MAX` (timestamp is not the leading sort key → full scan).
- **§5 sort keys, §13 at-a-glance, §2 engine summary** — extended for all six
  new tables. §13 also gained the `pool_registry` row it was missing.
- **Appendix A** — added all seven missing entities so the "every `prices.*`
  table, every column" claim in its preamble is true again; noted the two
  non-timestamp version columns and the composite `unresolved_pools` key.
- **§3.0** — added the two new `backfill_progress` columns and an explicit
  scope note that the diagram is core-path-only (Appendix A is the exhaustive
  one). Left the side tables out of it deliberately: adding seven entities
  would cost the price path its legibility.

## Design Decisions

### Emerged

1. **Documented all seven missing tables, not just the two named.** AC #3 ("no
   remaining `prices.*` table absent") could not be met by covering only
   `unresolved_pools` + `discovery_state` — `asset_metadata`, `asset_supply`,
   `backfill_sdex_ledgers`, and `ingest_cursor` were also undocumented. The doc
   covered 6 of 13 tables; it now covers all 13.

2. **§3.0 kept core-path-only; Appendix A made exhaustive.** The two ER diagrams
   had drifted into overlapping roles. Split them explicitly by scope rather
   than duplicating seven entities into both, and stated the split in-line so
   the next editor knows which diagram to extend.

3. **Section numbering appends rather than inserts.** `unresolved_pools` is
   §3.7 (right after its §3.6 `pool_registry` companion) and the rest follow in
   init.sql order. Inserting to group them thematically would have renumbered
   §3.6 and broken inbound cross-references from other docs and task notes.

## Issues Encountered

- **Diagram rendering not machine-verified.** Mermaid is not a workspace
  dependency, so the two edited `erDiagram` blocks were checked structurally
  (fence balance, type-token stand-ins, attribute-line shape against the
  existing entities in the same file) rather than rendered. Worth an eye on the
  GitHub preview during review.
