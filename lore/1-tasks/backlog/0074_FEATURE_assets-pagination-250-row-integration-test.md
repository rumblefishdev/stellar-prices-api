---
id: "0074"
title: "`GET /assets` keyset pagination — 250-row no-dup/no-skip integration test"
type: FEATURE
status: backlog
related_adr: ["0008"]
related_tasks: ["0040"]
tags: [layer-backend, effort-small, priority-medium, api, rust, clickhouse, test, pagination]
milestone: 1
links:
  - "../../../packages/prices-api/tests/list_it.rs"
history:
  - date: 2026-07-01
    status: backlog
    who: claude
    note: >
      Spawned from 0040 deferred acceptance criterion. 0040 shipped keyset
      cursor pagination for `GET /v1/assets` and tests its correctness at a
      small scale (4-row fixture, `limit=2`, 2-page walk in `list_it.rs`), but
      the AC calls for a 250-row fixture walked via `?cursor` with an explicit
      no-duplicate / no-skip assertion across the full traversal. That
      larger-scale test was not written; carve it out here.
---

# `GET /assets` pagination — 250-row integration test

## Summary

Add an integration test that walks the full `GET /v1/assets`
keyset-pagination result set over a **250-row fixture** using
`?limit=50` + `?cursor`, and asserts every asset appears exactly
once — no duplicates, no skips — across all cursor requests. This
closes the one functional acceptance criterion 0040 deferred.

## Context

0040 (PR #68) implemented the cursor (`common/cursor.rs`, opaque
Base64 `{v,id}`) and the default `volume_24h DESC` keyset query.
Existing coverage (`list_it.rs::default_sort_volume_desc_paginates`)
proves the mechanism on a 4-row fixture but not at scale, and does
not assert set-completeness. The 250-row walk is the AC that
matters for confidence against tie-breaking on the sort column.

## Implementation

- Seed a 250-row `assets` + `current_prices` fixture in a scratch
  CH database (reuse the `setup`/`teardown` helpers in
  `list_it.rs`); include duplicate/adjacent `volume_24h` values so
  the cursor's `{sort_col, id}` tie-break is actually exercised.
- Loop `GET /v1/assets?limit=50` following `cursor` until
  `has_more == false`, collecting every returned `id`.
- Assert: total collected == 250, the id set has no duplicates
  (`len(set) == len(list)`), and it equals the seeded id set (no
  skips). Assert page count == 5.
- Mark `#[ignore]` like the other live-CH tests (runs under
  `cargo test -- --ignored` against local docker CH, prod-pinned
  26.3.10.60).

## Acceptance Criteria

- [ ] Integration test seeds 250 rows (with sort-column ties) and
      walks all pages via `?cursor` at `limit=50`.
- [ ] Asserts no duplicate and no skipped ids; collected set equals
      the seeded set; page count is 5.
- [ ] Passes against local docker ClickHouse; green under the
      `--ignored` live-CH test lane.
