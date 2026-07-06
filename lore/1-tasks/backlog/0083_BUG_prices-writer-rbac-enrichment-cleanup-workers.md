---
id: "0083"
title: "enrichment + cleanup workers fail on prices_writer RBAC (ACCESS_DENIED) — grant scope + enrichment temp-table redesign"
type: BUG
status: backlog
related_adr: ["0007"]
related_tasks: ["0070", "0082", "0026", "0056"]
tags: [layer-ops, milestone-M1, priority-high, effort-small, aws, clickhouse, rbac, cross-team, hetzner, post-deploy]
milestone: 1
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../../packages/cleanup-worker/src"
history:
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

## Acceptance Criteria

- [ ] cleanup runs green: enumerates + drops old partitions (verify on a live invoke).
- [ ] enrichment runs green and populates `close_usd` on `price_ohlcv_1m` — via
      subquery/CTE (preferred) or a scratch DB, not broad `CREATE TABLE` on `prices.*`.
- [ ] No orphaned `prices.price_ohlcv_1m_xlmusd_ref_*` tables in prod (none
      expected now — CREATE was denied pre-execution; re-check after the fix lands).
- [ ] BE RBAC change (cleanup grants) applied + documented alongside their 0314.
