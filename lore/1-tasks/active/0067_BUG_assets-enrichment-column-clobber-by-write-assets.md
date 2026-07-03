---
id: "0067"
title: "assets enrichment columns clobbered by write_assets full-row re-emit (home_domain, future token_supply)"
type: BUG
status: active
related_adr: ["0004", "0007"]
related_tasks: ["0038", "0039"]
tags: [layer-ingestion, clickhouse, data-integrity, writers, effort-small, priority-medium]
links:
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
history:
  - date: 2026-06-25
    status: backlog
    who: oski
    note: >
      Spawned from 0039 future work. Surfaced while resolving 0039 open
      Q#1 (current-price MV + asset_supply single-writer design).
  - date: 2026-07-03
    status: active
    who: okarcz
    note: >
      Promoted backlog → active to implement the fix. Pre-deploy review confirmed
      the bug is latent (nothing writes a non-empty home_domain today, so nothing
      is clobbered) — NOT a 0070 go-live gate — but chose to fix now via Option 1
      (single-writer enrichment table), the same pattern asset_supply uses. No data
      migration needed (assets.home_domain is always '').
---

# assets enrichment columns clobbered by `write_assets` full-row re-emit

## Summary

`prices.assets` is `ReplacingMergeTree(updated_at)` (whole-row replace on
merge, no column-level merge). The live ledger processor's
`OhlcvWriter::write_assets` (`packages/prices-ingest-core/src/writer.rs:141`)
re-emits a **full** `AssetRow` for every asset in its in-memory registry with
hardcoded `home_domain: ""`, `is_active: 1`, and an implicit `updated_at =
now()`. Any column an enrichment writer (Asset Discovery) sets on that row —
today `home_domain`, and the proposed `token_supply` — is **clobbered back to
the default** the next time `write_assets` runs (which is far more frequent
than the hourly discovery pass). Net: enrichment columns silently never stick.

## Context

- Discovered while designing 0039's Asset Discovery worker. 0039 dodges it for
  market-cap by routing `token_supply` into a dedicated single-writer
  `prices.asset_supply` table instead of an `assets` column — but
  `home_domain` is already on `assets` and already exposed to this hazard.
- `home_domain` exists in the schema (`schema/init.sql`) specifically for
  discovery to populate; nothing populates it today, so the bug is latent, not
  yet observed.
- Root cause is the two-writer-on-one-ReplacingMergeTree-row anti-pattern (see
  ADR 0007 §3 read-time-merge semantics): `assets` has two writers (ledger
  processor + discovery), and full-row replace means last-write-wins on the
  whole row.

## Options (decide at impl)

1. **Split identity vs. enrichment.** Ledger processor writes only identity
   columns; enrichment (`home_domain`, …) lives in a separate single-writer
   table (e.g. `prices.asset_metadata`) the read views JOIN — same pattern
   0039 uses for `asset_supply`.
2. **Make `write_assets` insert-if-absent**, not re-emit the whole registry
   every run (only write genuinely new asset_ids), so it stops overwriting
   existing rows. Reduces clobber to the create moment but is racy if discovery
   and the processor both create the same asset.
3. **Coalesce on merge** by reading current enrichment before re-emit, or move
   `assets` to a merge engine that preserves columns. Heavier; least preferred.

Option 1 is the cleanest and matches the single-writer invariant 0039 relies on.

## Acceptance Criteria

- [x] An enrichment value (`home_domain`) written for an asset survives a
      subsequent `write_assets` run — regression test
      `asset-discovery/tests/enrichment_survives_it.rs` (green vs local CH):
      write identity → `write_asset_metadata(home_domain)` → re-run `write_assets`
      → `home_domain` still present.
- [x] Chosen writer-ownership model documented (Option 1) — identity lives in
      `prices.assets` (ledger processor + discovery, identity columns only),
      enrichment lives in the new single-writer `prices.asset_metadata`. No two
      writers target the same RMT row.
- [x] Read surface returns the enrichment value — the assets list + detail
      queries LEFT JOIN `asset_metadata`; `endpoints_it.rs` asserts the detail
      endpoint returns `home_domain` seeded only in `asset_metadata`.

## Implementation

Option 1 (split identity vs. enrichment), mirroring the `asset_supply` pattern.

- `packages/prices-clickhouse/schema/init.sql` — new `prices.asset_metadata`
  (`asset_id, home_domain, updated_at`) RMT; `home_domain` stays a `DEFAULT ''`
  column on `assets` for back-compat but is no longer read.
- `packages/prices-ingest-core/src/writer.rs` — dropped `home_domain` from the
  `write_assets` row (identity only); added the single-writer
  `write_asset_metadata(&[AssetMetadata])` + `AssetMetadata` (re-exported).
- `packages/prices-api/src/assets/queries_ch.rs` — list + detail queries now
  `LEFT JOIN asset_metadata AS m FINAL` and source `m.home_domain`.
- Tests: new `enrichment_survives_it.rs` regression; `endpoints_it.rs` fixture
  moved `home_domain` into `asset_metadata` + added a detail assertion.

## Design Decisions

### From Plan

1. **Option 1 — single-writer enrichment table.** Cleanest, matches the
   `asset_supply` invariant 0039 relies on; rejected Option 2 (insert-if-absent,
   racy) and Option 3 (merge-engine change, heavy).

### Emerged

2. **Kept `assets.home_domain` as a vestigial `DEFAULT ''` column** instead of
   `ALTER TABLE DROP COLUMN`. Dropping it is a destructive prod migration for no
   benefit — reads no longer reference it and `write_assets` no longer writes it,
   so it collapses to a harmless always-empty column. A later cleanup can drop it.
3. **No production `home_domain` populator wired.** Populating `home_domain`
   (SEP-1 TOML fetch) is the separate enrichment feature; this task only makes the
   column *survivable*. `write_asset_metadata` is exercised by the regression test;
   its first real caller ships with the enrichment work.

## Out of scope

- `token_supply` / `market_cap_usd` — already handled by 0039 via the
  dedicated `prices.asset_supply` table.
- Fetching/populating `home_domain` values — separate enrichment feature; this
  task only removes the clobber hazard so those values will stick.
- Dropping the now-vestigial `assets.home_domain` column — deferred cleanup.
