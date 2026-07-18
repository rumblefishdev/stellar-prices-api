---
id: "G-prod-reapply-runbook"
title: "Operator runbook — re-apply corrected rollup MV DDL to live ch-prod-01"
type: G
task: "0071"
status: mature
spawned_from: []
spawns: []
related_notes: []
links:
  - "../../../../../packages/prices-clickhouse/schema/rollups.sql"
  - "../../../../../packages/prices-clickhouse/schema/preroll.sql"
  - "../../../archive/0051_FEATURE_clickhouse-prices-schema-and-mv-chain-migration/notes/G-live-schema-state.md"
---

# Operator runbook — re-apply corrected rollup MV DDL to `ch-prod-01`

> **Operator-executed.** The live DDL against production is run by the operator,
> NOT by an automated session (prepare-not-deploy). This note is the script.

## What you're fixing

The 6 live rollup MVs (`prices.mv_ohlcv_*`) on prod were created from buggy DDL
where the bucket alias `AS timestamp` shadowed the source `timestamp` column, so
`open`/`close`/`close_usd` at `_15m … _1M` tie-broke to an arbitrary row instead
of the true first/last by time. The repo's `schema/rollups.sql` + `preroll.sql`
are already corrected (qualified `t.timestamp`, `FROM … AS t`). This re-applies
them to the running cluster.

## ⚠️ Why the bucket alias is still `AS timestamp` (and cannot be renamed)

A natural "fix" is to rename the bucket to `ts_bucket` so it can't shadow the
source column. **This does not work for the MVs.** A `TO`-table materialized
view matches its SELECT output to the target columns **BY NAME**, so a
differently-named bucket is rejected:

```
Code: 8. DB::Exception: SELECT query outputs column with name 'ts_bucket',
which is not found in the target table. ... (THERE_IS_NO_COLUMN)
```

Verified on CH 26.3.10.60 via `prices-clickhouse/tests/rollup_chain_it.rs`
(task 0071). The bucket key is therefore **forced** to be `AS timestamp`, the
shadow is unavoidable, and qualifying the source as `t.timestamp` is the only
remedy — which is exactly what the corrected schema does. (`preroll.sql`'s plain
`INSERT … SELECT` maps by position and *could* rename, but keeps `AS timestamp`
for consistency.) **Do not attempt the rename.**

## 0. Setup (local)

```bash
cd ~/Projects/stellar/stellar-prices-api
git checkout develop && git pull          # ensure corrected schema files are present

# Same Route A access used for 0051 (loopback default admin via docker exec).
# Box: ch-prod-01 = 168.119.73.161, container app-clickhouse-1, CH 26.3.10.
CH_SSH='ssh root@168.119.73.161'          # or your ssh alias for ch-prod-01
CH='docker exec -i app-clickhouse-1 clickhouse-client'
```

## 1. Connectivity + decide the scenario

```bash
$CH_SSH "$CH -q 'SELECT version()'"                            # expect 26.3.10.x
$CH_SSH "$CH -q 'SELECT count() FROM prices.price_ohlcv_1m'"   # the leaf the chain derives from
```

- **`price_ohlcv_1m` count = 0 → EMPTY-DB path.** No mis-rolled buckets exist;
  the entire fix is replacing the MV *definitions* (§3–§4). Skip §5 recompute.
- **count > 0 → recompute needed.** After §3–§4, also run §5 to correct the
  already-written coarse buckets.

## 2. Capture rollback DDL (cheap insurance)

```bash
$CH_SSH "$CH -q \"SELECT name, create_table_query FROM system.tables \
  WHERE database='prices' AND name LIKE 'mv_ohlcv_%' FORMAT Vertical\"" \
  > /tmp/0071_mv_rollback_$(date +%Y%m%d).sql
```

## 3. DROP the six buggy MVs

Dropping an MV does **not** touch its target table — no rollup rows lost by the DROP.

```bash
$CH_SSH "$CH --multiquery -q \"
DROP VIEW IF EXISTS prices.mv_ohlcv_1m_to_15m;
DROP VIEW IF EXISTS prices.mv_ohlcv_15m_to_1h;
DROP VIEW IF EXISTS prices.mv_ohlcv_1h_to_4h;
DROP VIEW IF EXISTS prices.mv_ohlcv_4h_to_1d;
DROP VIEW IF EXISTS prices.mv_ohlcv_1d_to_1w;
DROP VIEW IF EXISTS prices.mv_ohlcv_1w_to_1M;\""
```

> Note: re-streaming `rollups.sql` WITHOUT dropping first is a no-op — it uses
> `CREATE MATERIALIZED VIEW IF NOT EXISTS`, so it silently skips the existing
> (buggy) MVs. The DROP is mandatory.

## 4. Re-create from the corrected `rollups.sql`

```bash
$CH_SSH "$CH --multiquery" < packages/prices-clickhouse/schema/rollups.sql
$CH_SSH "$CH -q \"SELECT name FROM system.tables \
  WHERE database='prices' AND name LIKE 'mv_ohlcv_%' ORDER BY name\""   # expect 6
```

**Confirm the corrected definition is now live** (this is the real proof on an
empty DB — listing 6 names is NOT proof the DROP took, since the buggy MVs are
also 6). The shown SQL must read `FROM prices.price_ohlcv_1m AS t FINAL` and
`argMin(open, t.timestamp)` / `argMax(close, t.timestamp)` — a bare `timestamp`
inside the argMin/argMax means the DROP was skipped; re-run §3.

```bash
$CH_SSH "$CH -q 'SHOW CREATE TABLE prices.mv_ohlcv_1m_to_15m FORMAT TSVRaw'"
```

## 5. Recompute mis-rolled buckets — ONLY if §1 found data

Skip entirely on the empty-DB path. Pick one based on the MV refresh mode
(`SELECT view, refresh_mode FROM system.view_refreshes WHERE database='prices'`):

- **REPLACE mode** — each MV rewrites its bounded window from corrected logic on
  the next refresh. Force it now instead of waiting:

  ```bash
  $CH_SSH "$CH --multiquery -q \"
  SYSTEM REFRESH VIEW prices.mv_ohlcv_1m_to_15m;
  SYSTEM REFRESH VIEW prices.mv_ohlcv_15m_to_1h;
  SYSTEM REFRESH VIEW prices.mv_ohlcv_1h_to_4h;
  SYSTEM REFRESH VIEW prices.mv_ohlcv_4h_to_1d;
  SYSTEM REFRESH VIEW prices.mv_ohlcv_1d_to_1w;
  SYSTEM REFRESH VIEW prices.mv_ohlcv_1w_to_1M;\""
  ```

- **APPEND mode, or coarse history older than the largest MV window** — TRUNCATE
  the coarse tables and rebuild deterministically from `_1m` via `preroll.sql`:

  ```bash
  $CH_SSH "$CH --multiquery -q \"
  TRUNCATE TABLE prices.price_ohlcv_15m; TRUNCATE TABLE prices.price_ohlcv_1h;
  TRUNCATE TABLE prices.price_ohlcv_4h;  TRUNCATE TABLE prices.price_ohlcv_1d;
  TRUNCATE TABLE prices.price_ohlcv_1w;  TRUNCATE TABLE prices.price_ohlcv_1M;\""
  $CH_SSH "$CH --multiquery" < packages/prices-clickhouse/schema/preroll.sql
  ```

## 6. Verify (the 0059 integration-test assertion, live)

Mismatch count between stored `_15m` and the true first/last recomputed from
`_1m` must be **0**:

```bash
$CH_SSH "$CH -q \"
SELECT count() AS mismatched_buckets
FROM prices.price_ohlcv_15m AS r FINAL   -- alias BEFORE FINAL (CH syntax; 'FINAL AS r' is a parse error)
INNER JOIN (
  SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
         asset_id, quote_asset_id, source,
         argMin(open, t.timestamp) AS open,
         argMax(close, t.timestamp) AS close,
         argMax(close_usd, t.timestamp) AS close_usd
  FROM prices.price_ohlcv_1m AS t FINAL
  GROUP BY timestamp, asset_id, quote_asset_id, source
) AS expect USING (timestamp, asset_id, quote_asset_id, source)
WHERE r.open != expect.open OR r.close != expect.close OR r.close_usd != expect.close_usd\""
```

On an empty DB this returns 0 trivially — so it proves nothing. The substantive
proof on an empty DB is the §4 `SHOW CREATE` definition check; combined with the
`prices-clickhouse/tests/rollup_chain_it.rs` integration test (passes on the
prod-pinned image), that is sufficient. A live smoke test is optional and NOT
recommended — but if you want one, MIND THE WINDOW:

> ⚠️ The live MV filters `WHERE t.timestamp >= now() - INTERVAL 2 HOUR`. Fixture
> rows with a stale/epoch timestamp (e.g. 1700000000 ≈ Nov 2023) are excluded by
> the refresh, so `_15m` stays empty and the check looks "stuck"/blank. Use
> `now()`-relative timestamps INSIDE the 2-hour window:

```bash
$CH_SSH "$CH --multiquery -q \"
INSERT INTO prices.price_ohlcv_1m
  (timestamp, asset_id, quote_asset_id, source, open, high, low, close,
   volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES
  (toStartOfInterval(now(), INTERVAL 15 MINUTE) + INTERVAL 60 SECOND,  999,1,'smoke',10,40,10,15,1,1,0,0,12,1,1),
  (toStartOfInterval(now(), INTERVAL 15 MINUTE) + INTERVAL 120 SECOND, 999,1,'smoke',30,40,10,40,1,1,0,0,35,1,2);\""
$CH_SSH "$CH -q 'SYSTEM REFRESH VIEW prices.mv_ohlcv_1m_to_15m'"; sleep 5
$CH_SSH "$CH -q \"SELECT open, close FROM prices.price_ohlcv_15m FINAL WHERE source='smoke' AND asset_id=999\""   # expect 10, 40
```

Cleanup (always run if you inserted any smoke rows — delete from the BASE `_1m`
and let the MV re-derive; never hand-delete the rollup tables, 0051 lesson):

```bash
$CH_SSH "$CH -q \"DELETE FROM prices.price_ohlcv_1m WHERE source='smoke'\""
$CH_SSH "$CH -q 'SYSTEM REFRESH VIEW prices.mv_ohlcv_1m_to_15m'"
$CH_SSH "$CH -q \"SELECT count() FROM prices.price_ohlcv_1m WHERE source='smoke'\""   # expect 0
```

## 7. Record provenance + close

Append result (date, CH version, empty-vs-recompute path taken, before/after
mismatch counts) to 0051's `G-live-schema-state.md` or here, then complete 0071
via `/lore-framework-tasks` and archive.

### Applied — 2026-06-29 (ch-prod-01)

- **Box / version:** `ch-prod-01` (168.119.73.161), CH **26.3.10.60**, Route A
  (`docker exec app-clickhouse-1 clickhouse-client`).
- **Scenario:** EMPTY-DB path — `SELECT count() FROM prices.price_ohlcv_1m` = **0**,
  so no mis-rolled buckets existed; §5 recompute correctly skipped.
- **Action:** §3 DROP of all six `prices.mv_ohlcv_*` + §4 re-create from the
  corrected `rollups.sql` (single `--multiquery` stream).
- **Proof (§4 def-check):** `SHOW CREATE TABLE prices.mv_ohlcv_1m_to_15m` shows
  `FROM prices.price_ohlcv_1m AS t FINAL` with `argMin(open, t.timestamp)` /
  `argMax(close, t.timestamp)` / `argMax(close_usd, t.timestamp)` and
  `WHERE t.timestamp >= now() - toIntervalHour(2)` — the corrected, qualified
  form. No bare `timestamp` in any argMin/argMax.
- **§6 mismatch query:** returns **0** (trivial on the empty DB; the def-check
  above is the substantive proof).
- **No restart / no data loss:** DDL only, `prices.*`-scoped; the shared box and
  BE's `default.*` untouched.
- **First-attempt note:** an initial pass re-streamed `rollups.sql` WITHOUT the
  §3 DROP — a no-op (`CREATE … IF NOT EXISTS` skipped the existing buggy MVs).
  The DROP-then-recreate above is what actually replaced the definitions.

## Acceptance-criteria mapping

| 0071 AC | Covered by |
|---|---|
| Six MVs re-created from corrected DDL | §3 + §4 |
| Mis-rolled buckets recomputed | §5 (skipped if empty) |
| Live spot-check argMin-open / argMax-close at ≥ `_15m` | §6 |
| Provenance recorded | §7 |
