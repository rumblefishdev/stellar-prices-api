---
id: "0210"
title: "Soroban assets carry an empty asset_code in listing and detail — top-volume rows are unidentifiable to consumers"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0120"]
tags: [layer-backend, priority-medium, effort-small, milestone-M2, api, metadata, defect]
milestone: 2
links: []
history:
  - date: 2026-08-19
    status: backlog
    who: stkrolikiewicz
    note: "Spawned from the 0120 conformance run."
---

# Soroban assets have empty asset_code

## Summary

Every soroban-native row in `GET /v1/assets` (including `CBIJ…`, ranked #6 by
24h volume at ~$29.5k) has `asset_code: ""` in both the listing and
`GET /v1/assets/{id}` (`code: ""`). Consumers see a top-10 asset with no
name — only a 56-char contract address.

## Context

Found by [[0120]]. Soroban token metadata (symbol/name) lives on the token
contract; the ingest pipeline evidently never resolves it into
`prices.assets`.

## Implementation

- Resolve token `symbol()` (and `name()`) at asset-mint time in the soroban
  ingest path, or via a periodic metadata backfill.
- Backfill existing soroban rows.
- 0120 keeps `code: ""` equality for `CBIJ…` in its fixed list — update
  `tools/scripts/conformance-assets.json` when this lands.

## Acceptance Criteria

- [ ] Soroban rows in the listing carry a non-empty code where the contract
      exposes one
- [ ] `CBIJ…` shows its real symbol in list + detail
