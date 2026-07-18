---
id: "0076"
title: "Apply pending prices.* schema drift to live ch-prod-01 (asset_supply, current_prices + MV, 0053 backfill/discovery tables)"
type: BUG
status: completed
related_adr: ["0007"]
related_tasks: ["0070", "0053", "0039", "0059", "0071", "0026"]
tags: [layer-database, priority-high, effort-small, clickhouse, materialized-views, operations, milestone-M1, schema-drift]
milestone: 1
links:
  - "../../../../packages/prices-clickhouse/schema/init.sql"
  - "../../../../packages/prices-clickhouse/schema/current.sql"
  - "notes/G-prod-apply-runbook.md"
history:
  - date: 2026-07-03
    status: completed
    who: oski
    note: >
      BE informed of the new 1-min refresh MV; all 6 acceptance criteria met.
      Schema drift closed on ch-prod-01. Archived. Unblocks 0070 (workers) + 0053
      (backfill run). mv_current_prices full population still gated on 0026 enrichment.
  - date: 2026-07-03
    status: applied
    who: oski
    note: >
      Applied to live ch-prod-01 (CH 26.3.10.60, Route A / deploy@ docker exec) and
      verified. init.sql + current.sql applied idempotently: created asset_supply,
      pool_registry, unresolved_pools + backfill_progress.{earliest,newest}_data_available
      columns + mv_current_prices refreshable MV; current_prices pre-existed and was
      byte-identical (no-op). MV confirmed refreshing 1/min via system.view_refreshes.
      price_ohlcv_1m = 0 rows so current_prices is empty (expected pre-ingestion).
      Remaining: BE heads-up on the new 1-min refresh MV. Ready to archive once confirmed.
  - date: 2026-07-03
    status: active
    who: oski
    note: >
      Spawned while prepping the 0070 production deploy. Audit found the live
      `ch-prod-01` schema is behind the repo: it was last applied 2026-06-24
      (0063 initial) + 2026-06-29 (0071 rollup-MV correction), but `init.sql`
      has since grown with tables the 0070 workers AND the 0053 backfill write.
      Missing on prod (by commit date): `prices.asset_supply` + `prices.current_prices`
      base tables (0039, 2026-06-25), the `mv_current_prices` refreshable MV
      (0039, `current.sql`), and 0053's `backfill_progress.{earliest,newest}_data_available`
      columns + `unresolved_pools` + `pool_registry` (2026-07-02). This task
      applies the idempotent top-up (`init.sql` + `current.sql`) in one operator
      pass so 0070's workers don't fail and the 0053 backfill has its tables.
      Prepare-only: the runbook is authored here; the live DDL against ch-prod-01
      is operator-executed (prepare-not-deploy).
---

# Apply pending prices.* schema drift to live ch-prod-01

## Summary

The `prices.*` schema on BE's shared Hetzner ClickHouse (`ch-prod-01`) is behind
the repo. Several tables/columns were added to `packages/prices-clickhouse/schema/`
**after** the last prod apply, two of which the 0070 worker Lambdas write to —
so deploying 0070 against the current prod DB would fail. This task applies the
idempotent schema top-up (`init.sql` + `current.sql`) in a single operator pass,
bringing prod current and unblocking both the 0070 live-ingestion deploy and the
0053 backfill run.

## Status: Active

**Current state:** Runbook authored (`notes/G-prod-apply-runbook.md`). Awaiting
operator execution of the live DDL against ch-prod-01 (Route A / `docker exec`).

## Context

Prod schema baseline:

- **2026-06-24** — 0063 initial `prices.*` apply (from `init.sql`, per 0051 chain).
- **2026-06-29** — 0071 re-applied the corrected rollup/preroll MVs (argMin/argMax fix).

Drift added to the repo **after** that baseline (not on prod):

| Object | Added by | Date | Needed for |
|--------|----------|------|-----------|
| `prices.asset_supply` | 0039 | 06-25 | **0070** supply-worker |
| `prices.current_prices` (base table) | 0039 | 06-25 | **0070** current_prices |
| `prices.mv_current_prices` (refreshable MV, `current.sql`) | 0039 | 06-25 | 0070 — see MV note |
| `backfill_progress.earliest_data_available` / `newest_data_available` | 0053 | 07-02 | **0053** backfill run |
| `prices.unresolved_pools` | 0053 | 07-02 | **0053** backfill run |
| `prices.pool_registry` | 0053 | 07-02 | **0053** backfill run |

All pending DDL is `CREATE … IF NOT EXISTS` / `ADD COLUMN IF NOT EXISTS`, so a
whole-file re-apply of `init.sql` is a safe no-op on already-present objects.
`seed.sql` is deliberately **not** applied (no data seeding into prod).

### `mv_current_prices` decision — apply now (forward-compatible)

`current.sql`'s header cautions "apply only once enrichment is live, else
`current_prices` serves all-zero USD rows." Decision (2026-07-03): **apply it now
anyway.** Rationale:

- It is a **refreshable** MV (`REFRESH EVERY 1 MINUTE`) that re-derives
  `current_prices` from scratch each minute, so it **self-heals** the instant
  enrichment (0026) starts filling `close_usd`/`volume_quote_usd` — no re-apply.
- The interim zero USD columns (`price_usd`, `volume_24h_usd`, `vwap_24h`,
  `market_cap_usd`) mislead no one: the read API (0040) is not built, so there is
  no consumer today.
- One prod-touch instead of two.

**Cost accepted:** a cheap 1-minute refresh query runs on shared `ch-prod-01`
from apply time — flag to BE.

## Implementation Plan

Operator-run, Route A (`ssh ch-prod-01 → docker exec app-clickhouse-1
clickhouse-client`), pure DDL, no container restart. Full commands:
`notes/G-prod-apply-runbook.md`.

1. **Confirm drift** — read-only `system.tables` / `system.columns` queries.
2. **Apply `init.sql`** (idempotent, base tables + 0053 columns/tables).
3. **Apply `current.sql`** (`mv_current_prices` refreshable MV).
4. **Verify** — the four tables + two columns present; MV registered.
5. **Notify BE** of the new 1-minute refresh MV on the shared box.

## Acceptance Criteria

- [x] Pre-apply drift confirmed against live ch-prod-01 (2026-07-03: `asset_supply`,
      `mv_current_prices`, `pool_registry`, `unresolved_pools` + both columns absent;
      `current_prices` already present; `price_ohlcv_1m` = 0 rows).
- [x] `init.sql` applied; `prices.asset_supply`, `prices.current_prices`,
      `prices.unresolved_pools`, `prices.pool_registry` all present.
- [x] `backfill_progress.earliest_data_available` + `newest_data_available` present.
- [x] `current.sql` applied; `prices.mv_current_prices` registered and refreshing
      (`system.view_refreshes` status=Scheduled, last_success_time advancing 1/min).
- [x] No existing-object clobbered — `current_prices` pre-existed and was verified
      byte-identical to `init.sql` (10 cols, same order/types, `ReplacingMergeTree(updated_at)`,
      `ORDER BY asset_id`), so `IF NOT EXISTS` skip was a true no-op.
- [x] BE notified of the 1-minute refresh MV on shared ch-prod-01 (2026-07-03).

## Issues Encountered

- **`current_prices` already on prod, contradicting commit-date reasoning.** The
  pre-apply audit predicted it missing (0039 base-table commit 2026-06-25 postdates
  the 2026-06-24 initial apply). It was in fact present. Root cause not chased — it
  was verified **byte-identical** to the repo's `init.sql` definition via
  `SHOW CREATE TABLE`, so the positional `TO`-table match for `mv_current_prices`
  is safe and the `IF NOT EXISTS` re-apply was a no-op. No action needed.
- **Legacy `prices.current_price_usd` object on prod** (not in the repo schema files).
  Out of scope for this task; left untouched. Flagged for a later cleanup/audit.

## Design Decisions

### Emerged

1. **Applied `mv_current_prices` now rather than deferring to enrichment (0026).**
   The MV is refreshable (`REFRESH EVERY 1 MINUTE`) so it self-heals when 0026 fills
   `close_usd`/`volume_quote_usd`; interim zero-USD rows mislead no consumer (read API
   0040 not built). One prod-touch instead of two. Cost: a 1-min refresh query now
   runs on shared ch-prod-01. See README "MV decision".

## Notes

- Blocks/unblocks: gates 0070 Step 0.5 (pre-deploy) and the 0053 backfill run.
- Precedent: 0071 (`notes/G-prod-reapply-runbook.md`) for the Route A mechanism.
- `mv_current_prices` full population depends on 0026 enrichment (tracked there).
