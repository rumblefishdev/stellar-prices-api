---
id: "0211"
title: "Document OHLCV window boundary semantics — both start and end are inclusive, measured but stated nowhere"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0120", "0119"]
tags: [layer-docs, priority-low, effort-small, milestone-M2, api, docs]
milestone: 2
links: []
history:
  - date: 2026-08-19
    status: backlog
    who: stkrolikiewicz
    note: "Spawned from the 0120 conformance run."
---

# Document OHLCV window boundary semantics

## Summary

`GET /v1/assets/{id}/ohlcv?start=…&end=…` treats **both ends as inclusive**:
a request for `start=2026-08-14T00:00Z&end=2026-08-19T00:00Z` at 1d
granularity returns six buckets, 08-14 through 08-19 (measured on production
2026-08-19, task [[0120]]). Neither §4 of the overview doc nor the OpenAPI
param descriptions state this; 0120's suite initially assumed an exclusive
`end` and produced 14 false failures.

## Implementation

- State the inclusivity rule in §4 and in the `start`/`end` param
  descriptions in `packages/prices-api/src/openapi/mod.rs` (they flow into the
  generated spec).
- Mention the interaction with `timeframe` (window anchors to `end`, 0119)
  and with partial current buckets (today's 1d bucket is returned while the
  day is still open).

## Acceptance Criteria

- [ ] §4 documents inclusive `start`/`end`
- [ ] Generated spec param descriptions state it
- [ ] Partial-current-bucket behavior documented
