---
id: "0074"
title: "`GET /assets` keyset pagination — 250-row no-dup/no-skip integration test"
type: FEATURE
status: completed
related_adr: ["0008"]
related_tasks: ["0040"]
tags: [layer-backend, effort-small, priority-medium, api, rust, clickhouse, test, pagination, milestone-M1]
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
  - date: 2026-07-02
    status: active
    who: claude
    note: >
      Promoted from backlog to implement the last non-deploy-gated M1
      item. Small, code-only: add the 250-row cursor-walk IT to
      `list_it.rs`.
  - date: 2026-07-02
    status: completed
    who: claude
    note: >
      Done + merged (PR #75, merge `573b02c` → develop). Added
      `keyset_pagination_250_rows_no_dup_no_skip` (+`setup_n`/`enc_cursor`
      helpers) to `list_it.rs` (+124 lines, 1 test). All 3 ACs met; verified
      green vs prod-pinned CH 26.3.10.60 (full `list_it` suite 5/5), clippy +
      rustfmt-edition2024 clean. No production code changed; no existing tests
      modified. Also added the `milestone-M1` tag (task previously carried only
      the `milestone: 1` field, which is what excluded it from the tag-based M1
      filter). This was the last non-deploy-gated M1 item.
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

- [x] Integration test seeds 250 rows (with sort-column ties) and
      walks all pages via `?cursor` at `limit=50`.
      — `keyset_pagination_250_rows_no_dup_no_skip` in `list_it.rs`;
      `setup_n` seeds 250 assets + `current_prices` rows.
- [x] Asserts no duplicate and no skipped ids; collected set equals
      the seeded set; page count is 5. — asserts `pages == 5`,
      `seen.len() == 250`, distinct-set len == 250, and set equality
      vs the seeded `A0001..A0250`.
- [x] Passes against local docker ClickHouse; green under the
      `--ignored` live-CH test lane. — verified green vs prod-pinned
      CH 26.3.10.60 (full `list_it` suite 5/5); clippy + fmt clean.

## Implementation Notes

- **Identity for the completeness assertion is `asset_code`, not
  `asset_id`.** The `GET /assets` list item DTO (`assets/dto.rs::AssetListItem`)
  does not expose the internal `asset_id`, so the fixture gives each of the
  250 rows a unique `asset_code` (`A0001`…`A0250`) and the walk collects those.
- **Tie-break is genuinely exercised.** Volumes are bucketed
  `(asset_id % 13) * 100` → 13 large (~19-row) tie-groups that are
  non-monotonic in `asset_id`, so equal-volume rows both *reorder* relative to
  id and *straddle* the 50-row page boundary. A broken `(sort_col, asset_id)`
  keyset would surface as a dropped/repeated `asset_code`.
- **Cursor is URL-encoded on the walk.** The opaque cursor is STANDARD Base64
  (`common/cursor.rs`), whose alphabet includes `+ / =`; a raw `+` in a query
  string decodes to a space and 400s the request. `enc_cursor` percent-encodes
  those three so the walk is robust to whatever bytes the fixture produces.
  (The pre-existing 4-row test passes the cursor raw and only worked by luck of
  its small cursor values.)
