---
id: "0082"
title: "Post-deploy verification — periodic workers write their tables + current_prices MV populates + alarms don't false-fire"
type: TEST
status: done
related_adr: ["0007"]
related_tasks: ["0070", "0056", "0083", "0084", "0086", "0096"]
tags: [layer-ops, milestone-M1, priority-medium, effort-small, aws, clickhouse, observability, post-deploy]
milestone: 1
links:
  - "../archive/0070_FEATURE_deploy-prices-ingestion-to-production-m1.md"
history:
  - date: 2026-07-06
    status: active
    who: okarcz
    note: >
      0083 resolved: enrichment + cleanup workers now write green in prod
      (RBAC grants applied; enrichment reworked to no-DDL inline subquery +
      redeployed; both live-invoked, no ACCESS_DENIED). CH restart audited
      non-destructive. Updated the worker-write + alarm criteria accordingly and
      flagged the EnrichmentRowsRemainingRecent stall-alarm floor (~48k
      exotic-quote candles) for 0056 tuning. Remaining open here: supply timeout
      (0084) and the soroswap-source watch. Also cross-linked 0086 (oracle ts bug).
  - date: 2026-07-06
    status: active
    who: okarcz
    note: >
      Activated right after the 0070 go-live to verify the periodic workers +
      MV + alarms while the freshly-deployed system is warm.
  - date: 2026-07-06
    status: backlog
    who: okarcz
    note: >
      Spawned from 0070 go-live. The ingestion path (ledger-processor) is verified
      end-to-end in prod; this closes the remaining post-deploy acceptance check —
      that each scheduled worker actually writes its table and the current_prices
      MV populates, and that the alarms are green under steady state.
  - date: 2026-07-15
    status: done
    who: okarcz
    note: >
      DONE. All verification ACs resolved or forwarded: workers write (enrichment/
      cleanup green via 0083; supply times out → 0084), current_prices MV populated,
      alarms sane (folded into 0056). Final open AC — soroswap rows in
      price_ohlcv_1m — investigated 2026-07-15: soroswap is absent (0 candles, 0
      unresolved) because the backfill doesn't preload pool_registry; root-caused
      and forwarded to new task 0096. Nothing left to verify here. Archived.
---

# Post-deploy verification of periodic workers + MV + alarms

## Summary

The 0070 go-live proved live ledger ingestion (rows in `price_ohlcv_1m`, AMM
resolving, DLQ=0). This task closes the one deferred 0070 acceptance criterion:
confirm the EventBridge-scheduled workers each write their table on a real invoke
and the `current_prices` materialised view populates, and that no alarm is
false-firing.

## Context

Workers deployed by 0070's EventBridge stack, each on its own schedule:
`asset-discovery` → `assets`/`pool_registry`, `oracle` → `oracle_prices`,
`supply` → `asset_supply`, `enrichment` → USD columns, `cleanup` → retention.

## Implementation

- Either wait for each worker's schedule to fire, or `aws lambda invoke` each once,
  and confirm its target table gets fresh rows (SSH `clickhouse-client` on prod CH).
- Confirm `prices.current_prices` (MV) populates from the live `price_ohlcv_1m`.
- Confirm `lag_seconds` metric is emitting and the >60s alarm + per-Lambda
  error/duration alarms are `OK` (not `ALARM`) under steady state.
- Sanity-check `soroswap`-source rows appear in `price_ohlcv_1m` once a Soroswap
  swap lands (only `aquarius`/`phoenix` had rows in the first go-live window).

## Findings (2026-07-06, first invoke round)

Manual `aws lambda invoke` of each worker against prod:

| Worker | Result |
|--------|--------|
| `oracle` | ✅ `{"queried":3,"skipped":0,"written":3}` → `oracle_prices` |
| `asset-discovery` | ✅ `seeded 1660 assets` (pools_total 0 — empty payload gave no ledger range to scan) |
| `supply` | ⚠️ slow — aws CLI read-timeout + 3 retries at 60s each; no error logged. Verify by `asset_supply` freshness, not the sync invoke. |
| `enrichment` | ❌ **ACCESS_DENIED** → spawned **0083** |
| `cleanup` | ❌ **ACCESS_DENIED** → spawned **0083** |

Root cause (from prod `system.query_log`): `prices_writer` lacks `SELECT ON
system.parts` (cleanup) and `CREATE TABLE ON prices.*` (enrichment's peg-pivot
ref table). Core ingestion unaffected. Fix tracked in **0083** (BE RBAC grant +
enrichment temp-table redesign).

## Findings (2026-07-06, table + alarm round)

**Tables (`system.tables`):** ✅ `asset_supply` **1164** (so `supply` *did*
complete server-side despite the CLI timeout), ✅ `current_prices` **1627** (MV
populating via `mv_current_prices`), ✅ rollup chain filling (`_15m` 18k / `_1h`
10k / `_4h` 6k from `_1m` 46.9k; `_1d/_1w/_1M` still 0 — longer refresh cadence),
✅ `oracle_prices` 157, `assets` 1685, `pool_registry` 521, `unresolved_pools` 0.
`asset_metadata` 0 (enrichment down, 0083).

**Sources:** `aquarius` 451, `phoenix` 15, `sdex` 46.5k — all fresh. **`soroswap`
still 0** after 30+ min with `unresolved_pools = 0` — either low activity or a
Soroswap-specific resolution gap; **watch**, investigate if it persists.

**Alarms** — two expected, two are real observability gaps → folded into **0056**:
- `cleanup-errors`, `enrichment-errors` = ALARM — **expected** (0083 RBAC).
- `sdex-push-freshness` = ALARM while SDEX is fresh → **false positive** (watches
  the backfill push signal, not live ingestion). → 0056 finding A.
- No ledger-processor `lag_seconds`/error alarm exists at all. → 0056 finding B.
- `supply-errors` = ALARM — **NOT benign** (logs confirmed): supply hits the
  300s Lambda timeout (`Status: timeout`) on **every** invoke incl. scheduled +
  async retries, writing `asset_supply` only partially (1164/1685). Real defect →
  spawned **0084** (batch/checkpoint the asset walk).

## Findings (2026-07-06, 0083 resolution round)

**0083 DONE — enrichment + cleanup now write green in prod** (see [[0083]]):
- BE extended `prices_writer` (`SELECT ON system.parts` + `ALTER DELETE ON
  prices.*`); **cleanup** live-invoked green (dropped expired partitions
  `price_ohlcv_1m=202311` + `oracle_prices=197001`, no ACCESS_DENIED).
- **enrichment** reworked to a no-DDL inline ASOF subquery, redeployed +
  live-invoked green — `close_usd` populating (81k+ rows), **zero orphan
  `*_xlmusd_ref_*` tables**, no ACCESS_DENIED. `asset_metadata` is a separate
  column, not enrichment output.
- BE also restarted `ch-prod-01` in that window — audited **non-destructive**
  (no data loss, no gaps, ingestion continuous, all MVs scheduled).

**Alarm implications:** `cleanup-errors` + `enrichment-errors` should now clear
to `OK` (the RBAC crash that raised them is fixed) — **re-check both are OK**.
New watch: the `Prices/Enrichment` **`EnrichmentRowsRemainingRecent`** stall alarm
has a permanent floor of ~48k unenrichable **exotic-quote** recent candles (no
USD/XLM/USDC reference path); its threshold must sit above that floor or it will
false-fire → track the threshold with the 0056 alarm-tuning items.

Two workers still open: `supply` (times out → **0084**) and the `soroswap`-source
watch (below). Oracle data-quality bug spawned: **0086** (Reflector ×1000
timestamps → junk `1970-01` `oracle_prices` rows).

## Acceptance Criteria

- [~] Each periodic worker writes its table on a live invoke — `oracle`,
      `asset-discovery`, **`enrichment` ✅, `cleanup` ✅ (0083 resolved
      2026-07-06)**; `supply` writes but **times out mid-walk → 0084**.
- [x] `current_prices` MV populated from live candles (1627 rows via
      `mv_current_prices`; rollup chain `_15m/_1h/_4h` also filling).
- [~] Deploy alarms sane under steady state — `cleanup`/`enrichment` error alarms
      should now clear post-0083 (**re-check OK**); `sdex-push-freshness`
      false-positive + missing ledger-processor lag alarm + `EnrichmentRowsRemaining`
      floor **folded into 0056** (findings A/B + enrichment-floor).
- [x] `soroswap`-source rows confirmed present in `price_ohlcv_1m` — **investigated
      2026-07-15: they are NOT present** (0 soroswap candles, `unresolved_pools`=0 for
      soroswap → swaps invisible to the backfill, despite 221 soroswap pools seeded in
      `pool_registry`). Root-caused as a backfill coverage gap (backfill doesn't preload
      `pool_registry`, the twin of the live-only 0078 fix) → forwarded to **task 0096**.
- [~] `supply` writes `asset_supply` (1164 rows) but **does not complete** — it
      times out at 300s on every invoke; full-walk fix tracked in **0084**.
