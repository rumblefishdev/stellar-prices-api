---
id: "0083"
title: "enrichment + cleanup workers fail on prices_writer RBAC (ACCESS_DENIED) — grant scope + enrichment temp-table redesign"
type: BUG
status: active
related_adr: ["0007"]
related_tasks: ["0070", "0082", "0026", "0056", "0085"]
tags: [layer-ops, milestone-M1, priority-high, effort-small, aws, clickhouse, rbac, cross-team, hetzner, post-deploy]
milestone: 1
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../../packages/cleanup-worker/src"
history:
  - date: 2026-07-06
    status: active
    who: okarcz
    note: >
      BE applied cleanup grants (SELECT ON system.parts + ALTER DELETE ON prices.*)
      and restarted ch-prod-01 (14:58:12 UTC) in the same window. Read-only audit
      confirms grants present in SHOW GRANTS FOR prices_writer, and the restart was
      non-destructive: all 30 prices objects intact, no data loss, all 7 refreshable
      MVs Scheduled w/ zero exceptions, no detached parts, no orphaned *_xlmusd_ref_*
      tables, ingestion continuous through the restart (61.5k rows since, ~29s
      freshness, zero gaps in last 4h). Flipped the BE-RBAC acceptance criterion to
      done. Only cleanup live-invoke remains.
  - date: 2026-07-06
    status: active
    who: okarcz
    note: >
      Activated to do the BE-independent half: rework enrichment's peg-pivot tier
      from a `CREATE TABLE` ref-table to an inline ASOF-JOIN subquery (needs zero
      grants), unblocking enrichment without waiting on BE. cleanup stays parked
      on the BE RBAC grant (system.parts + DROP PARTITION).
  - date: 2026-07-06
    status: backlog
    who: okarcz
    note: >
      Found by the 0082 post-deploy verification right after the 0070 go-live.
      Two periodic workers fail against prod CH with Code 497 ACCESS_DENIED on the
      `prices_writer` mTLS identity (confirmed in `system.query_log`). Core
      ingestion (ledger-processor + oracle + asset-discovery) is unaffected — plain
      INSERT/SELECT are granted; these two need privileges BE's 0314 RBAC didn't
      include. Non-blocking for M1 go-live (enrichment = phase-2 USD backfill;
      cleanup = retention, nothing to drop yet), but must land before those
      workers are useful.
---

# enrichment + cleanup workers fail on prices_writer RBAC

## Summary

Post-0070 go-live, the `enrichment` and `cleanup` periodic workers both crash with
`Clickhouse(BadResponse(""))`; `system.query_log` shows **Code 497 ACCESS_DENIED**
on the `prices_writer` CH user. `oracle`, `asset-discovery`, and the
ledger-processor (plain `INSERT`/`SELECT`) all work.

## Root cause (from `system.query_log`, 2026-07-06)

- **cleanup** — `SELECT DISTINCT partition FROM system.parts WHERE database='prices'
  AND table='price_ohlcv_1m' …` → missing `SELECT ON system.parts` (and, once past
  that, the privilege for `ALTER TABLE … DROP PARTITION` on `prices.*`).
- **enrichment** — `CREATE TABLE prices.price_ohlcv_1m_xlmusd_ref_9_<nanos>
  ENGINE=MergeTree AS SELECT …` → missing `CREATE TABLE ON prices.*`. The
  peg-pivot tier (`ch_enrich.rs::pivot_ref_sql`, line 786) materializes the
  volume-weighted XLM/USDC reference series as a **real table in the shared
  `prices` DB**, ASOF-joins candles against it, then `DROP TABLE`.

## Resolution

### cleanup (grants — inherent to a retention worker; BE-owned RBAC)
Ask BE to extend `prices_writer`:
```sql
GRANT SELECT ON system.parts TO prices_writer;
-- for `ALTER TABLE prices.<t> DROP PARTITION <p>` (CH 26.3.10.60 — BE to confirm
-- the exact partition-drop privilege token; likely `ALTER DELETE`):
GRANT ALTER DELETE ON prices.* TO prices_writer;
```

### enrichment (decision — do NOT just widen grants)
Granting broad `CREATE TABLE ON prices.*` to the writer on a **BE-co-tenanted**
database is poor posture, and — once the grant exists — a crash mid-run would leak
orphaned `*_xlmusd_ref_*` tables in the shared DB. (No orphan exists now: the
current failures are `ExceptionBeforeStart`, so the `CREATE` never ran.) Prefer,
in order:

1. **Subquery / CTE (no DDL).** Replace the materialized ref table with an inline
   `ASOF LEFT JOIN (SELECT … the XLM/USDC series …) AS r`. The ref series is small
   (one row/minute), so this may perform fine and needs **zero** new grants.
   Evaluate perf across the batch loop first.
2. **Dedicated scratch DB.** If materialization is needed for perf, point the ref
   table at a `prices_scratch`-style DB the writer fully owns (BE creates it +
   grants `CREATE/DROP/SELECT/INSERT ON prices_scratch.*`), keeping DDL out of the
   shared `prices` data DB. Small code change: separate `scratch_database` config.

## Implementation — enrichment (2026-07-06, option 1 chosen)

Reworked the peg-pivot tier to compute the XLM/USDC reference **inline as an
ASOF-join subquery** — no `CREATE TABLE` / `DROP TABLE` at all, so `prices_writer`
needs **zero new grants** for enrichment (fully BE-independent). Removed
`pivot_ref_sql`, `materialize_pivot_ref`, `drop_table`, `unique_suffix`;
`pivot_sql` now embeds the ref `SELECT` (single-pair sort-key-prefix scan, cheap
to recompute per batch) and `enrich_peg_pivot_step` self-gates on `refs.xlm/usdc`
with a 4-bind order (watermark, window, watermark, LIMIT). Validated against local
CH **26.3.10.60** (= prod): 25 unit + 5 `ch_enrich_it` (incl. the pivot-tier
`close_usd` correctness case) + 2 `enrich_e2e` all green; clippy clean. **PR #86**.
Perf note: review #10's single-materialization is traded for a per-batch
single-pair re-aggregation — negligible now (no backfill), revisit with a
session-pinned `CREATE TEMPORARY TABLE` only if it regresses post-backfill.

**Takes effect on redeploy:** rebuild the `enrichment-worker` bootstrap +
`make deploy-production-eventbridge` (enrichment lives in the EventBridge stack).

**Code review (PR #86, high-effort):** no hard correctness bug — behaviour-
preserving (bind order, ASOF forward-fill, no drift, no caller breakage all
confirmed). Flagged the accepted trade-off: the inline ref is re-aggregated per
batch (`O(slice × batches)`), which could risk the 300s timeout once the 0053
backfill grows the XLM/USDC slice → **restore materialize-once before then,
spawned as 0085**. Tightened the code comment to state this honestly (PR #86).

## Verification — BE RBAC grant + CH restart audit (2026-07-06)

BE applied the cleanup grants **and restarted `ch-prod-01`** in the same window
(restart at `14:58:12 UTC`). Ran a read-only audit over `prices.*` to confirm the
grants landed and the restart was non-destructive. All checks via
`docker exec … clickhouse-client` (no writes, no restarts).

**Grants confirmed present** (`SHOW GRANTS FOR prices_writer`):
```sql
GRANT SELECT, INSERT, ALTER DELETE, OPTIMIZE ON prices.* TO prices_writer
GRANT SELECT ON system.parts                             TO prices_writer
```
Exactly what cleanup needs: `SELECT ON system.parts` (enumerate partitions) +
`ALTER DELETE ON prices.*` (BE confirmed this is the `DROP PARTITION` token in
26.3.10.60, matching the hand-off note). cleanup should no longer ACCESS_DENIED.

**Restart broke/deleted nothing:**
- All 30 `prices` objects present; every data table is `ReplacingMergeTree`
  (disk-backed) — nothing in-memory to lose across a restart.
- Data intact: `price_ohlcv_1m` = 316k rows, all rollups (15m/1h/4h/1d/1w) populated;
  partitions healthy (`202607` = 316k rows / 7 parts).
- All 7 refreshable MVs `Scheduled`, **zero exceptions**; per-minute MVs
  (`mv_current_prices`, `mv_ohlcv_1m_to_15m`) refreshing on schedule.
- No detached/broken parts; **no orphaned `*_xlmusd_ref_*` tables** (0083 invariant).
- **Live ingestion continuous through the restart**: 61.5k rows written since
  `14:58:12`, freshness ~29s (newest candle tracks real time). **Zero gaps in the
  last 4h** — a per-minute scan across `14:55–15:05` shows no dip to zero at the
  `14:58` restart (the one minute the boundary-scan flagged, `13:09`, was a
  query off-by-a-second artifact — it has 1006 rows).

Only remaining item: a **live cleanup-worker invoke** to confirm it enumerates +
drops green now that the grants are in place.

## Acceptance Criteria

- [ ] cleanup runs green: enumerates + drops old partitions (verify on a live invoke).
- [x] enrichment reworked to populate `close_usd` via an inline ASOF subquery (no
      `CREATE TABLE`); validated against CH 26.3.10.60. **Confirm on a live prod
      invoke after redeploy** (should no longer ACCESS_DENIED).
- [x] No orphaned `prices.price_ohlcv_1m_xlmusd_ref_*` tables — the table is now
      never created (inline subquery); IT asserts count 0.
- [x] BE RBAC change (cleanup grants) applied + documented alongside their 0314 —
      verified present in `SHOW GRANTS FOR prices_writer` (2026-07-06); post-grant
      CH restart audited as non-destructive (no data loss, no gaps, ingestion
      continuous).

## BE hand-off message (ready to send)

> **Subject: `prices_writer` CH grants — two periodic workers hit ACCESS_DENIED (post prices go-live)**
>
> We deployed prices live-ingestion to prod today (ledger-processor + workers).
> Core ingestion works — the ledger-processor, oracle, and asset-discovery all
> write fine over `prices_writer`. But two scheduled workers fail with **Code 497
> ACCESS_DENIED** (confirmed in `system.query_log`), because they need privileges
> beyond the `INSERT`/`SELECT` your 0314 RBAC granted:
>
> **1. `cleanup` (retention worker)** — enumerates + drops old monthly partitions:
> ```sql
> -- reads partitions to drop:
> GRANT SELECT ON system.parts TO prices_writer;
> -- executes `ALTER TABLE prices.<t> DROP PARTITION <p>` — please grant the
> -- partition-drop privilege scoped to prices.* (in 26.3.10.60 this is covered
> -- by ALTER DELETE; grant whichever token your model uses for DROP PARTITION):
> GRANT ALTER DELETE ON prices.* TO prices_writer;
> ```
> Exact failing query: `SELECT DISTINCT partition FROM system.parts WHERE
> database='prices' AND table='price_ohlcv_1m' AND active=1 …` → `Not enough
> privileges … SELECT ON system.parts`.
>
> **2. `enrichment`** — currently does `CREATE TABLE
> prices.price_ohlcv_1m_xlmusd_ref_<nanos> …` in the shared `prices` DB. **We are
> NOT asking for `CREATE TABLE ON prices.*`** — we agree that's poor posture on a
> co-tenanted DB. We'll rework it on our side (inline subquery, or a dedicated
> `prices_scratch` DB we own). Flagging only so you're aware; **if** you'd prefer
> we go the scratch-DB route, let us know and we'll request grants scoped to
> `prices_scratch.*` only.
>
> No orphaned tables exist (the CREATE was denied pre-execution). Thanks!
