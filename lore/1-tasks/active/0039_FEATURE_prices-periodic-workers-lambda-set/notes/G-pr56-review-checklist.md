---
title: "PR #56 code-review checklist — periodic workers"
type: generation
status: developing
spawns: []
tags: [code-review, pr-56]
links:
  - "../../backlog/0068_FEATURE_current-prices-mv-v2-columns.md"
history:
  - date: 2026-06-25
    status: developing
    who: claude
    note: "Review of PR #56; blocking issues 1-5 fixed (commit 29d5cf9)."
---

# PR #56 — Code Review Checklist (task 0039: periodic workers)

Issues to resolve, ranked most-severe first. Check off as fixed.

## Correctness — blocking  ✅ all fixed (commit 29d5cf9)

- [x] **1. `current_prices` MV column order/count mismatch** — `packages/prices-clickhouse/schema/current.sql:28`
  The SELECT emits 6 columns into the 10-column `prices.current_prices`. ClickHouse `MATERIALIZED VIEW … TO` inserts **by position** (see `rollups.sql` convention). Refresh either errors (`NUMBER_OF_COLUMNS_DOESNT_MATCH`) or misaligns values (`market_cap_usd → change_7d_pct Decimal(10,4)` → `DECIMAL_OVERFLOW`, `now() → volume_24h_usd`).
  *Fix:* list all 10 target columns in table order, or use an explicit target column list `TO current_prices (asset_id, price_usd, volume_24h_usd, vwap_24h, market_cap_usd, updated_at)`.

- [x] **2. Oracle worker stores ms timestamp without `/1000`** — `packages/oracle-worker/src/lib.rs:246`
  Reflector timestamps are milliseconds; the event path divides (`soroban.rs:423`). The `.min(u32::MAX)` clamp saturates to year 2106. Every oracle row is mis-dated, diverges from event rows, and wins ReplacingMergeTree dedup.
  *Fix:* `timestamp: (pd.timestamp / 1000).min(u32::MAX as u64) as u32`.

- [x] **3. Oracle worker mints `asset_id`s but never `write_assets`** — `packages/oracle-worker/src/lib.rs:244`
  `get_or_assign` mints a new id for unseen identities, but the registry is never persisted (unlike `asset-discovery`). Produces orphan / colliding ids → oracle prices attributed to wrong asset.
  *Fix:* call `writer.write_assets(&registry)` after assignment, or only emit samples for already-registered assets.

- [x] **4. MV reads `close_usd`/`volume_quote_usd` that nothing deployed populates** — `packages/prices-clickhouse/schema/current.sql:31`
  Ingest writes `close_usd=0`; enrichment (task 0026) is blocked / not deployed. `current_prices` will serve all-zero price/volume/market_cap until enrichment is wired up.
  *Fix:* sequence with 0026, or document/guard that current_prices is empty until enrichment lands.

- [x] **5. Supply worker: single terminal write after sequential loop → timeout loses everything** — `packages/supply-worker/src/lib.rs:198`
  Thousands of sequential Horizon GETs can exceed the 5-min Lambda timeout before the only `write_supplies`, so nothing is persisted that run.
  *Fix:* flush in batches and/or bounded concurrency (`buffer_unordered`).

## Correctness — secondary

- [x] **6. `market_cap_usd` Float64 round-trip loses precision / can overflow** — `packages/prices-clickhouse/schema/current.sql:42`
  `toDecimal128(toFloat64(price)*toFloat64(supply), 14)` loses low-order digits for large caps and can exceed `Decimal(38,14)` → refresh fails for all assets. Distinct from #1.
  *Fixed:* exact `Decimal256` product (no Float64 loss) + `accurateCastOrNull('Decimal128(14)')` → out-of-range degrades to the `0` sentinel via `ifNull` instead of throwing. vwap_24h stays Float64 (price-magnitude, can't overflow).

## Cleanup / altitude

- [x] **7. Three near-identical ~40-line Lambda wiring blocks** — `infra/src/lib/stacks/eventbridge-stack.ts:228`
  Extract a `createWorkerLambda({name, assetDir, rule, memory, timeout, alarmPeriod, env})` factory for cleanup/supply/oracle (and asset-discovery).
  *Fixed:* added `createWorkerLambda(scope, props)` to `lambda-baseline.ts` (role + log group + ARM64 function + rule target + error alarm; returns `{function, role}` so asset-discovery still grants S3 read on its `role`). All four blocks now call it; asset-discovery passes its extra env (`BUCKET_NAME`, `STELLAR_NETWORK_PASSPHRASE`) via `environment`. Construct ids preserved (`${idPrefix}Role/LogGroup/Function/ErrorAlarm`) → `cdk synth` diff vs HEAD is empty except one cosmetic env-map key reorder (no deploy effect). typecheck + lint green.

- [x] **8. `symbol_to_identity` duplicates private `reflector_key_to_identity` and has drifted** — `packages/oracle-worker/src/lib.rs:62`
  Core maps `native`+`XLM`; copy maps only `XLM`. Make `reflector_key_to_identity` `pub` and reuse it.
  *Fixed:* made `reflector_key_to_identity` `pub` + re-exported from prices-ingest-core; deleted the local `symbol_to_identity` and reuse the shared fn.

- [x] **9. `TRACKED_SYMBOLS` duplicates the mapping domain; loop guard is dead code** — `packages/oracle-worker/src/lib.rs:238`
  `let Some(identity) = symbol_to_identity(symbol) else { continue }` can never `continue`. Iterate the mapping or drop the guard.
  *Fixed:* removing the duplicate `symbol_to_identity` leaves `TRACKED_SYMBOLS` (poll list) and `reflector_key_to_identity` (mapping) as distinct concerns, so the guard is now a genuine filter; added an invariant test that every tracked symbol resolves.

## Non-blocking note

- [ ] **CI builds no Lambda bootstrap binaries** — `cdk synth` needs a manual `cargo lambda build -p {cleanup,supply,oracle}-worker --release --arm64`. Pre-existing gap, now ×3; consider a CI build/guard.

## Verified OK (no action)

- cleanup-worker `toUInt32(partition)` + unquoted `DROP PARTITION <YYYYMM>` + strict `<` boundary — correct
- `i128_from_parts` negative handling — correct
- supply-worker SQL interpolation (named column list, injection-safe) — correct
- MV `LEFT JOIN asset_supply FINAL` does not fan-out the `sum` — correct
- `priceUpdater → assetSupply` rename consistent across `types.ts` / `validateConfig` / `production.json`
- `23`-statement schema test count — correct
