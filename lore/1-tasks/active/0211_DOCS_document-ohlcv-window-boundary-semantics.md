---
id: "0211"
title: "Document OHLCV window boundary semantics — both start and end are inclusive, measured but stated nowhere"
type: DOCS
status: active
assignee: okarcz
related_adr: []
related_tasks: ["0120", "0119"]
tags: [layer-docs, priority-low, effort-small, milestone-M2, api, docs]
milestone: 2
links: []
history:
  - date: "2026-11-25"
    status: active
    who: okarcz
    note: >
      Implemented on docs/0211_… (840147fd): inclusive bucket-start rule,
      mid-bucket cases and the open current bucket stated in the /ohlcv
      OpenAPI text and overview §4.2. PR to follow.
  - date: "2026-11-25"
    status: active
    who: okarcz
    note: "Activated for implementation (quick-fix queue; Stan OK'd the handover 2026-11-24)."
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

- [x] §4 documents inclusive `start`/`end`
      ✅ 2026-11-25: overview §4.2 — the `start`/`end` rows say **inclusive**, and a
      new "Window boundaries" paragraph gives the six-candle example from 0120, the
      mid-bucket rule and the `timeframe`-anchors-to-`end` rule (0119). Revision
      history row added.
- [x] Generated spec param descriptions state it
      ✅ Already said "inclusive" since 0195 (`e8ed5c0e`). Now also say what is
      compared — the bucket's start time — and the `/ohlcv` description carries the
      same rule with the example. `web/portal/public/openapi.json` refreshed;
      `openapi:verify-bundled` and `openapi:lint` pass.
- [x] Partial-current-bucket behavior documented
      ✅ In the `/ohlcv` description and §4.2: the newest candle can be the open
      current bucket and changes until it ends; §4.2 adds how a client drops it.

## Implementation Notes

Branch `docs/0211_document-ohlcv-window-boundary-semantics`, commit `840147fd`.

- The rule comes from the SQL, not only from the 0120 measurement: both OHLCV
  queries filter `timestamp >= toDateTime(?)` / `timestamp <= toDateTime(?)` on
  the bucket's start (`queries_ch.rs`, `ohlcv` and the USD-mode query). So a
  `start` inside a bucket leaves that bucket out and an `end` inside one keeps it.
  The mid-bucket case was read from code, not re-measured on production.
- Files: `packages/prices-api/src/assets/handlers.rs` (description + `start`/`end`
  params), `web/portal/public/openapi.json`, `docs/prices-api-general-overview.md`.

## Design Decisions

### Emerged

1. **Also corrected §4.2's auto-granularity table.** It still said `all → 1d`;
   since PR #217 `all` and `start`/`end` windows take the finest size within 5000
   candles (`1d` until 2029-06-08, then `1w`; see 0253). Same table, same reader.
2. **No code change.** The behaviour is right and measured; only its statement
   was missing.
