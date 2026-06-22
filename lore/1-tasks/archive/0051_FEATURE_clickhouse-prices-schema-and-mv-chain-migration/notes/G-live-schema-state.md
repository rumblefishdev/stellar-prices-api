---
title: "Live Hetzner prices schema state (production apply provenance)"
prefix: G
status: mature
spawned_from: "0051"
related_tasks: ["0051", "0063", "0060", "0061", "0059"]
date: 2026-06-22
---

# G — Live `prices.*` schema state on Hetzner production

Provenance capture for task 0051 Step 4 + task 0063 AC#1 — the record that
the `prices` database and its full object set are live on the **single**
Hetzner CH box (BE's `production` server `ch-prod-01`; there is no separate
dev/staging CH box, per `G-provisioning-plan.md` §0).

- **Box:** `168.119.73.161` (`ch-prod-01`)
- **Applied by:** box `default` admin over loopback via
  `docker exec app-clickhouse-1 clickhouse-client` (Option 1 — no mTLS, no DDL
  cert; matches BE's sidecar model).
- **CH server version:** 26.3.10
- **Apply method:** Route A — streamed `init.sql` → `seed.sql` → `views.sql` →
  `rollups.sql` from the local repo through `ssh … docker exec -i …
  clickhouse-client --multiquery`.
- **Date applied:** 2026-06-22

---

## 1. Database exists

`SHOW DATABASES` not captured separately — the §2 `system.tables` query below
returns 24 objects for `WHERE database='prices'`, which proves the `prices`
database exists on the box.

## 2. Full object set (`system.tables` — name + engine) — VERIFIED ✅

24 objects: 12 base `ReplacingMergeTree` tables + 6 rollup MVs + 6 read-surface
views. Includes the read surface from 0061 (`price_usd_series`,
`usd_reference`, `current_price_usd`, `identity_by_contract`, +`_1h`) and the
`backfill_sdex_ledgers` cursor table.

```
    ┌─name──────────────────┬─engine─────────────┐
 1. │ assets                │ ReplacingMergeTree │
 2. │ backfill_progress     │ ReplacingMergeTree │
 3. │ backfill_sdex_ledgers │ ReplacingMergeTree │
 4. │ current_price_usd     │ View               │
 5. │ current_prices        │ ReplacingMergeTree │
 6. │ identity_by_contract  │ View               │
 7. │ mv_ohlcv_15m_to_1h    │ MaterializedView   │
 8. │ mv_ohlcv_1d_to_1w     │ MaterializedView   │
 9. │ mv_ohlcv_1h_to_4h     │ MaterializedView   │
10. │ mv_ohlcv_1m_to_15m    │ MaterializedView   │
11. │ mv_ohlcv_1w_to_1M     │ MaterializedView   │
12. │ mv_ohlcv_4h_to_1d     │ MaterializedView   │
13. │ oracle_prices         │ ReplacingMergeTree │
14. │ price_ohlcv_15m       │ ReplacingMergeTree │
15. │ price_ohlcv_1M        │ ReplacingMergeTree │
16. │ price_ohlcv_1d        │ ReplacingMergeTree │
17. │ price_ohlcv_1h        │ ReplacingMergeTree │
18. │ price_ohlcv_1m        │ ReplacingMergeTree │
19. │ price_ohlcv_1w        │ ReplacingMergeTree │
20. │ price_ohlcv_4h        │ ReplacingMergeTree │
21. │ price_usd_series      │ View               │
22. │ price_usd_series_1h   │ View               │
23. │ usd_reference         │ View               │
24. │ usd_reference_1h      │ View               │
    └───────────────────────┴────────────────────┘
```

## 3. `backfill_progress` seed (must be 2) — VERIFIED ✅

```
SELECT count() FROM prices.backfill_progress  ->  2
```

## 4. Engine + sort key sanity (`price_ohlcv_1m`) — VERIFIED ✅

Confirmed live on 2026-06-22 — matches ADR 0003 (sort key includes
`quote_asset_id`) and ADR 0004 (per-source rows; `source` in the sort key):

```sql
CREATE TABLE prices.price_ohlcv_1m
(
    `timestamp` DateTime CODEC(DoubleDelta),
    `asset_id` UInt32,
    `quote_asset_id` UInt32,
    `source` LowCardinality(String),
    `open` Decimal(38, 14),
    `high` Decimal(38, 14),
    `low` Decimal(38, 14),
    `close` Decimal(38, 14),
    `volume_base` Decimal(38, 14) DEFAULT 0,
    `volume_quote` Decimal(38, 14) DEFAULT 0,
    `volume_quote_usd` Decimal(38, 14) DEFAULT 0,
    `close_usd` Decimal(38, 14) DEFAULT 0,
    `vwap` Decimal(38, 14),
    `trade_count` UInt32 DEFAULT 0,
    `version` UInt64
)
ENGINE = ReplacingMergeTree(version)
PARTITION BY toYYYYMM(timestamp)
ORDER BY (asset_id, quote_asset_id, source, timestamp)
SETTINGS index_granularity = 8192
```

## 5. Refreshable MVs registered

`system.view_refreshes` not captured as a separate listing; the 6 MVs are
present in §2 and the §6 smoke test proves `mv_ohlcv_1m_to_15m` is live and
functioning (propagation + replace-on-refresh both observed).

> CH-26.3 gotcha for next time: `system.view_refreshes.last_refresh_result`
> does **not** exist on this build — use `SELECT *` / `FORMAT Vertical`, or
> columns `status`, `last_success_time`, `next_refresh_time`, `exception`.

## 6. Refreshable MV chain smoke test (propagation) — VERIFIED ✅

Fixture `_1m` row (`source='smoke'`) → propagated into `_15m`:

```
after INSERT into price_ohlcv_1m (source='smoke'):
  price_ohlcv_1m:  1
  price_ohlcv_15m: 1     <-- PROPAGATION PROVED (mv_ohlcv_1m_to_15m)
  price_ohlcv_1h … _1M: 0   (their MVs refresh on 15m/1h/4h/1d cadence; not yet fired)
```

Cleanup (also confirms refreshable **replace-on-refresh** semantics):

```
DELETE FROM price_ohlcv_1m  WHERE source='smoke'   -> _1m  = 0
DELETE FROM price_ohlcv_15m WHERE source='smoke'   -> _15m = 1 (transient: MV is
    the owner of _15m and atomically replaces it; the hand-delete was overwritten
    by the next refresh rebuilding from _1m)
next refresh rebuilt _15m from the now-clean _1m   -> _15m = 0
final sweep: all granularities source='smoke' = 0  ✅
```

Lesson recorded: rollup tables (`_15m … _1M`) are MV-owned/derived — never
hand-delete them; delete from the base `_1m` and let/force the MV re-derive.

---

## Result

- [x] `prices` database + full schema applied to live Hetzner production
- [x] `price_ohlcv_1m` engine + sort key verified against ADR 0003/0004
- [x] Object set (24 objects) + `backfill_progress` seed (=2) verified
- [x] MV propagation smoke test confirmed (`_1m` → `_15m`), fixture cleaned up
