---
id: "G-prod-apply-runbook"
title: "Operator runbook — apply pending prices.* schema drift to ch-prod-01"
type: G
task: "0076"
status: mature
spawned_from: []
spawns: []
related_notes: []
links:
  - "../../../../../packages/prices-clickhouse/schema/init.sql"
  - "../../../../../packages/prices-clickhouse/schema/current.sql"
---

# Operator runbook — apply pending prices.* schema drift to ch-prod-01

> **Prepare-not-deploy.** The live DDL against ch-prod-01 is operator-executed.
> Pure SQL via `docker exec` — **no container restart**, no service bounce.
> Run from the repo root so the `< schema/*.sql` redirects resolve.

## Step 0 — Notify BE FIRST (before any apply)

Give BE a heads-up that you're about to apply prices.* schema DDL to the shared
`ch-prod-01`, and that a new `REFRESH EVERY 1 MINUTE` MV (`prices.mv_current_prices`)
will start running continuously afterward. Pure DDL, no restart — but it's their
shared box, so they hear about it before, not after.

## Access (Route A — same as 0071 / 0051)

```bash
# ch-prod-01 = 168.119.73.161, container app-clickhouse-1, CH 26.3.10.60
# Loopback default-admin via docker exec (no mTLS needed on the box itself).
CH_SSH='ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161'   # ch-prod-01
CH='docker exec -i app-clickhouse-1 clickhouse-client'

# sanity
$CH_SSH "$CH -q 'SELECT version()'"       # expect 26.3.10.x
```

## Step 1 — Confirm the drift (READ-ONLY, do this first)

```bash
# Which prices.* tables exist today
$CH_SSH "$CH -q \"SELECT name FROM system.tables WHERE database='prices' ORDER BY name FORMAT TSVRaw\""
# EXPECT MISSING: asset_supply, current_prices, mv_current_prices, pool_registry, unresolved_pools

# The two new backfill_progress columns
$CH_SSH "$CH -q \"SELECT name FROM system.columns WHERE database='prices' AND table='backfill_progress' AND name IN ('earliest_data_available','newest_data_available') ORDER BY name FORMAT TSVRaw\""
# EXPECT EMPTY (columns not yet present)
```

Capture this output — it is the pre-apply evidence for the task's AC #1.

## Step 2 — Apply init.sql (idempotent: base tables + 0053 columns/tables)

`init.sql` is all `CREATE … IF NOT EXISTS` / `ALTER … ADD COLUMN IF NOT EXISTS`,
so already-present objects (e.g. `price_ohlcv_1m`, the 7 grains) are untouched.
`seed.sql` is intentionally NOT applied — no data seeding into prod.

```bash
$CH_SSH "$CH --multiquery" < packages/prices-clickhouse/schema/init.sql
```

## Step 3 — Apply current.sql (mv_current_prices refreshable MV)

Idempotent `CREATE MATERIALIZED VIEW IF NOT EXISTS`. Self-heals once enrichment
(0026) fills the USD columns — see README "MV decision".

```bash
$CH_SSH "$CH --multiquery" < packages/prices-clickhouse/schema/current.sql
```

## Step 4 — Verify (READ-ONLY)

```bash
# The four tables now present
$CH_SSH "$CH -q \"SELECT name FROM system.tables WHERE database='prices' AND name IN ('asset_supply','current_prices','pool_registry','unresolved_pools') ORDER BY name FORMAT TSVRaw\""
# EXPECT: asset_supply, current_prices, pool_registry, unresolved_pools

# The two new columns
$CH_SSH "$CH -q \"SELECT name FROM system.columns WHERE database='prices' AND table='backfill_progress' AND name LIKE '%_data_available' ORDER BY name FORMAT TSVRaw\""
# EXPECT: earliest_data_available, newest_data_available

# The MV is registered and refreshable
$CH_SSH "$CH -q \"SELECT name FROM system.tables WHERE database='prices' AND name='mv_current_prices' FORMAT TSVRaw\""
# EXPECT: mv_current_prices
$CH_SSH "$CH -q \"SELECT status, last_success_time, next_refresh_time FROM system.view_refreshes WHERE view='mv_current_prices' FORMAT Vertical\""
# EXPECT: a scheduled refresh (next_refresh_time populated)
```

## Step 5 — Confirm to BE the MV is live

Post-apply, confirm to BE that `prices.mv_current_prices` is now refreshing on
`ch-prod-01` (see the `system.view_refreshes` check in Step 4).

## Rollback

All additive. If needed:

```bash
$CH_SSH "$CH -q 'DROP VIEW IF EXISTS prices.mv_current_prices'"
$CH_SSH "$CH -q 'DROP TABLE IF EXISTS prices.current_prices'"
$CH_SSH "$CH -q 'DROP TABLE IF EXISTS prices.asset_supply'"
$CH_SSH "$CH -q 'DROP TABLE IF EXISTS prices.unresolved_pools'"
$CH_SSH "$CH -q 'DROP TABLE IF EXISTS prices.pool_registry'"
# The two backfill_progress columns are additive Nullable(DateTime); leave them,
# or DROP COLUMN IF EXISTS if a clean revert is required.
```

Leave `price_ohlcv_1m`, the grains, rollup MVs, and views alone — those are the
already-applied 0051/0071 baseline, not part of this drift.
