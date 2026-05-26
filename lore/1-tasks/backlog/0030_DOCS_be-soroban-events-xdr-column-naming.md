---
id: '0030'
title: 'Surface BE soroban_events.topics_xdr / .data_xdr column-naming issue'
type: DOCS
status: backlog
related_adr: []
related_tasks: ['0018']
tags:
  [
    layer-database,
    priority-low,
    effort-small,
    cross-repo,
    be-feedback,
    clickhouse,
    soroban-events,
  ]
links:
  - '../active/0018_RESEARCH_decode-per-amm-swap-event-shapes/notes/R-be-storage-format.md'
  - '../../../../soroban-block-explorer/crates/db-clickhouse/src/persist/stage.rs'
  - '../../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md'
history:
  - date: 2026-05-15
    status: backlog
    who: claude
    note: 'Spawned from 0018 Appendix B item 1.'
---

# Surface BE soroban_events.topics_xdr / .data_xdr column-naming issue

## Summary

The BE ClickHouse `soroban_events` table has columns
`topics_xdr` and `data_xdr` whose contents are **not** XDR bytes
(neither base64 nor binary) but BE's own tagged JSON of the
decoded ScVal (`{type: "<tag>", value: <val>}` produced by
`crates/xdr-parser/src/scval.rs::scval_to_typed_json`). Task 0018
documented this; it tripped the prices-api consumer's initial
decode assumption and is likely to trip future readers.

## Context

This is a cross-repo signal, not a prices-api code change. The
fix lives in `soroban-block-explorer`. Options for BE:

1. Rename columns to e.g. `topics_json` / `data_json` (schema
   change; non-trivial migration even for a pilot store).
2. Keep the column names but add CH `COMMENT ON COLUMN` describing
   the actual content shape (minimal change, big readability win).
3. Document the format in
   `crates/db-clickhouse/README.md` Type-translation table (free).

Option 2 + 3 are likely the right combination. Option 1 is
deferrable until the pilot turns into a non-pilot.

## Implementation

- Open an inbox message (or Linear ticket) against
  `soroban-block-explorer` linking to task 0018's
  `R-be-storage-format.md`.
- Suggest the COMMENT ON COLUMN + README update as the
  minimum-cost fix.

## Acceptance Criteria

- [ ] BE has been notified via the usual cross-repo channel.
- [ ] Notification includes a concrete reproduction (the link to
      `R-be-storage-format.md` suffices) and the suggested fix.
