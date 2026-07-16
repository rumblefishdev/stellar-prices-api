# Incremental pre-roll — pre-Soroban SDEX tail

**Run this once, at the END of the pre-Soroban SDEX tail backfill**, to roll the
`[genesis, activation)` `1m` candles up into the coarse forever-tables
(`_15m … _1M`) **without** disturbing the already-pre-rolled Soroban-era coarse.

- **Script:** [`packages/prices-clickhouse/schema/preroll-incremental.sql`](../../packages/prices-clickhouse/schema/preroll-incremental.sql)
- **Task:** 0088 · **Related:** 0090 (the full-rebuild pre-roll + cleanup coordination), 0095 (rollup MVs → APPEND)
- **Status:** prepared + verified locally (prod-pinned CH 26.3.10.60); **NOT yet run against prod.**

## Why a separate script (the trap)

`schema/preroll.sql` is a **full rebuild** meant to run after `TRUNCATE`-ing the
coarse tables. **Do NOT use it here.** The Soroban-era coarse (`activation →
~2026-07`) is already durably pre-rolled (0090) and its source `1m` partitions
were dropped, so a TRUNCATE + full re-roll would rebuild coarse from only the
pre-Soroban tail and **wipe the Soroban-era history**. `preroll-incremental.sql`
**appends** the pre-Soroban buckets only and leaves every existing coarse row
untouched.

**Why appending is safe (verified):** coarse tables are
`ReplacingMergeTree(version)`, `version = ledger*1000 + op`. Pre-Soroban ledgers
are all `< activation`, so every appended row has a **lower version** than any
Soroban-era row for the same key → on merge RMT keeps the Soroban row. At the one
activation-boundary bucket (the `1d/1w/1M` bucket spanning the activation moment)
the Soroban-side value is preserved and this script's pre-Soroban partial simply
loses. Confirmed locally: boundary month kept the Soroban row; 2020 buckets
appended at all six granularities; a second run did not double-count.

## Pre-flight (confirm before running)

1. **Tail is done.** `pgrep -af sdex-backfill` is gone / the run reports complete,
   and the floor reached activation-1:
   ```sql
   SELECT max(sequence) FROM prices.backfill_sdex_ledgers WHERE sequence < 50457424;
   -- expect ~50,457,423
   ```
2. **Get the exact activation boundary** (the `{boundary}` param — the first
   Soroban-only-source candle marks activation):
   ```sql
   SELECT min(timestamp) FROM prices.price_ohlcv_1m
   WHERE source IN ('aquarius','phoenix','soroswap');
   ```
   Use that (or the known activation timestamp, ~`2024-02-20`) as `--param_boundary`.
3. **Cleanup still DISABLED** (it must stay off until AFTER this pre-roll, or the
   nightly job drops the pre-Soroban `1m` before it is rolled up):
   ```bash
   aws events describe-rule --name prices-production-cleanup --region eu-central-1 \
     --profile soroban-explorer --query 'State'   # expect "DISABLED"
   ```
4. **Disk headroom** on ch-prod-01 (`df -h /var/lib/docker`) — the pre-Soroban 1m
   is already resident; the coarse append is small, but leave margin.

## Run

From your shell against prod CH (hand the SQL to the operator's client; do not
run prod DDL from an agent). Substitute the confirmed boundary:

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec -i app-clickhouse-1 clickhouse-client \
     --param_boundary="2024-02-20 00:00:00" --multiquery' \
  < packages/prices-clickhouse/schema/preroll-incremental.sql
```

- **STAGE 1** (`15m ← 1m`) is chunked by year to fit the ~5.59 GiB query quota
  (0090); each statement scans one year via partition pruning. If a single year
  still hits the quota, split it into halves or drop `FINAL` on the intermediate
  stages (they carry no dups).
- **STAGE 2** (the `1h→1M` chain) runs once, bounded `< boundary`. Run STAGE 1 to
  completion first.
- Add earlier-year blocks only if a count shows `1m` rows before 2015 (earliest
  SDEX candle is ~2016-03).

## Post-run verification

```sql
-- Coarse now covers the pre-Soroban years (spot years):
SELECT '1d' t, toYear(timestamp) y, count() FROM prices.price_ohlcv_1d
WHERE timestamp < {boundary} GROUP BY y ORDER BY y;

-- Boundary month preserved the Soroban value (version-wins), not clobbered:
SELECT timestamp, close, version FROM prices.price_ohlcv_1M FINAL
WHERE timestamp = toStartOfMonth({boundary}) AND source='sdex' LIMIT 5;
```

Expect non-zero coarse rows for 2017–2023 and the boundary-month row unchanged
from its pre-run (Soroban) value.

## After it succeeds — re-enable cleanup

Only once the pre-roll is verified (coarse is durable), re-enable the nightly
retention so the redundant `1m` partitions drop:

```bash
aws events enable-rule --name prices-production-cleanup --region eu-central-1 \
  --profile soroban-explorer
```

## Accepted residual + optional repair

The activation month/week's coarse buckets reflect only their **post-activation**
slice (the pre-activation SDEX partial loses on version). This drops at most the
`[2024-01-01 … activation)` SDEX sliver from the boundary calendar buckets. If
that sliver matters, recompute just those boundary buckets from the **full** `1m`
(both sides) — possible only while the boundary-month Soroban `1m` is still
resident (i.e. before cleanup is re-enabled).
