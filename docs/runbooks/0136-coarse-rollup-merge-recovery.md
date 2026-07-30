# Runbook — recovering the frozen coarse OHLCV rollups (task 0136)

Restores `price_ohlcv_15m`, `_1h`, `_4h`, `_1d`, `_1w`, `_1M`, frozen since
**2026-07-21 02:44 UTC**. Diagnosis, evidence and the local reproduction are in
`lore/1-tasks/backlog/0136_BUG_coarse-rollup-tables-frozen-since-2026-07-21.md`.

**Mechanism (reproduced on local CH 26.3.10.60):** merges and mutations are
inert on exactly these six tables. `SYSTEM STOP MERGES` reproduces every
observable — mutation never attempted, no error, parts accumulating, `part_log`
showing inserts only. A pending mutation alone does **not** (control completed in
seconds). The flag is in-memory, exposed in no system table, and cleared by a
server restart — ch-prod-01 has not restarted since 2026-07-06, so nothing
cleared it.

**Trigger: unknown, and it does not change the recovery.** Both 07-17 operations
were ours ([[0095]] backup + APPEND MV recreate, [[0097]] STAGE 0 phoenix
delete), and `SYSTEM STOP MERGES` appears in none of our scripts or runbooks. It
was plausibly typed at the console and not recorded. Do not spend time on
attribution before recovering.

## Preconditions

- [ ] Read the "what resuming merges will change" section below. Row counts on
      the coarse tables **will** change — that is the intended effect of a
      pending delete, not a fault.
- [ ] **The six `price_ohlcv_*_bak` tables must still exist.** They are the only
      restore path. [[0105]] (drop the backups) is **blocked until this task is
      verified** — confirm it has not been run.
- [ ] Off-peak if possible. This generates real background I/O on a cluster
      shared with BE.
- [ ] BE notified that coarse `prices` data is stale and about to move.

## What resuming merges will change

Two things happen at once, and both are wanted:

1. **Merges collapse the accumulated parts.** `ReplacingMergeTree` deduplicates
   by `version` during merge, so duplicate-PK rows written by the [[0097]]
   idempotent pre-roll collapse. Row counts drop. This is by design — the
   pre-roll is documented as re-runnable _because_ RMT collapses what a second
   run adds.
2. **The six pending Phoenix deletes execute** (1,601 parts to rewrite in
   total). `phoenix` rows in the [[0097]] range are removed — which is what
   STAGE 0 existed to do, and its effect was already verified downstream on
   07-17.

Neither is data loss. But capture counts first (Step 0) so "the numbers moved"
can be told apart from "the numbers moved _wrongly_".

## Step 0 — pre-flight snapshot (read-only)

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec -i app-clickhouse-1 clickhouse-client --format=PrettyCompact --multiquery' <<'SQL'
SELECT '=== part + row counts (the before picture) ===' AS section;
SELECT table, count() AS parts, sum(rows) AS rows,
       formatReadableSize(sum(bytes_on_disk)) AS size
FROM system.parts WHERE database='prices' AND table LIKE 'price_ohlcv%' AND active
GROUP BY table ORDER BY table;

SELECT '=== per-source coarse counts (must be explainable after) ===' AS section;
SELECT source, count() AS rows_1h FROM prices.price_ohlcv_1h
GROUP BY source ORDER BY rows_1h DESC;

SELECT '=== backups still present? (0105 must NOT have run) ===' AS section;
SELECT name, metadata_modification_time, total_rows
FROM system.tables WHERE database='prices' AND name LIKE '%_bak' ORDER BY name;

SELECT '=== pending mutations ===' AS section;
SELECT table, mutation_id, parts_to_do, is_done FROM system.mutations
WHERE database='prices' AND is_done=0 ORDER BY table;

SELECT '=== uptime (a restart would have cleared the lock) ===' AS section;
SELECT uptime() AS seconds, now() - uptime() AS started_at;
SQL
```

**Gate:** six `_bak` tables present; `started_at` still 2026-07-06; six mutations
still `is_done = 0`. Save this output — it is the baseline for every later
comparison.

## Step 1 — probe on the smallest leaf table

`price_ohlcv_1M` is the **leaf** of the rollup chain (nothing rolls up from it),
the smallest by part count, and not on the critical path. If the hypothesis is
wrong, this costs nothing; if it is right, it costs one small table's merges.

```sql
SYSTEM START MERGES prices.price_ohlcv_1M;
```

Then watch for ~2 minutes:

```sql
SELECT database, table, elapsed, round(progress, 3) AS progress, is_mutation
FROM system.merges WHERE database = 'prices';

SELECT count() AS parts FROM system.parts
WHERE database='prices' AND table='price_ohlcv_1M' AND active;

SELECT table, parts_to_do, is_done FROM system.mutations
WHERE database='prices' AND table='price_ohlcv_1M';
```

**Decision gate:**

- **Merges appear / parts fall / `parts_to_do` drops** → hypothesis **confirmed**.
  Continue to step 2.
- **Nothing happens within ~5 minutes** → hypothesis **dead**. Stop. Re-open the
  diagnosis; do not start merges on the remaining five. Record the negative
  result in 0136 — it is as valuable as a positive one.

**Rollback at any point:** `SYSTEM STOP MERGES prices.price_ohlcv_1M;` returns
the table to exactly its current state. Merges already completed are not undone,
but they are not harmful — they are the normal steady-state operation this table
has been denied for nine days.

## Step 2 — unblock the chain head

`price_ohlcv_15m` is the actual blocker: it is wedged at exactly
`parts_to_throw_insert = 5000`, which is why `mv_ohlcv_1m_to_15m` throws and
everything downstream starves.

```sql
SYSTEM START MERGES prices.price_ohlcv_15m;
```

Watch until parts drop **well below 5,000** (aim < 1,000):

```sql
SELECT count() AS parts, sum(rows) AS rows FROM system.parts
WHERE database='prices' AND table='price_ohlcv_15m' AND active;
```

This is the slow step — 5,027 parts / 4.55 GiB, plus a 52-part mutation. Expect
tens of minutes, not seconds. **Do not proceed while parts are still falling.**

Once below the threshold, `mv_ohlcv_1m_to_15m` should succeed on its next
1-minute refresh:

```sql
SELECT view, status, last_success_time, substring(exception,1,120) AS exception
FROM system.view_refreshes WHERE view = 'mv_ohlcv_1m_to_15m' FORMAT Vertical;
```

**Gate:** `exception` empty and `last_success_time` within the last two minutes.
That is the moment the nine-day freeze ends.

## Step 3 — the remaining four

Only after step 2 is green. One at a time, largest first since `_1h` is the next
bottleneck:

```sql
SYSTEM START MERGES prices.price_ohlcv_1h;   -- 6,325 parts / 7.79 GiB — the heaviest
SYSTEM START MERGES prices.price_ohlcv_4h;   -- 3,124 parts / 4.02 GiB
SYSTEM START MERGES prices.price_ohlcv_1d;   -- 1,395 parts / 1.64 GiB
SYSTEM START MERGES prices.price_ohlcv_1w;   --   811 parts / 615 MiB
```

Between each, confirm the pool is not saturated and BE's tables are not starving:

```sql
SELECT count() AS active, countIf(is_mutation) AS mutations FROM system.merges;
SELECT metric, value FROM system.metrics
WHERE metric IN ('BackgroundMergesAndMutationsPoolTask','BackgroundMergesAndMutationsPoolSize');
```

If `BackgroundMergesAndMutationsPoolTask` approaches the pool size, **pause** —
let it drain before starting the next table. There is no deadline here.

## Step 4 — verify the chain refills

The cascade is `1m → 15m → 1h → 4h → 1d → 1w → 1M`, each on its own cadence, so
the tips recover in sequence over hours, not at once.

```sql
SELECT * FROM (
    SELECT 'price_ohlcv_1m' AS t, max(timestamp) AS newest FROM prices.price_ohlcv_1m
    UNION ALL SELECT 'price_ohlcv_15m', max(timestamp) FROM prices.price_ohlcv_15m
    UNION ALL SELECT 'price_ohlcv_1h',  max(timestamp) FROM prices.price_ohlcv_1h
    UNION ALL SELECT 'price_ohlcv_4h',  max(timestamp) FROM prices.price_ohlcv_4h
    UNION ALL SELECT 'price_ohlcv_1d',  max(timestamp) FROM prices.price_ohlcv_1d
    UNION ALL SELECT 'price_ohlcv_1w',  max(timestamp) FROM prices.price_ohlcv_1w
    UNION ALL SELECT 'price_ohlcv_1M',  max(timestamp) FROM prices.price_ohlcv_1M
) ORDER BY t;
```

**Note the 07-21 → now gap will NOT self-heal.** The rollup MVs read a bounded
recent window; they do not reach back nine days. Once the tips are advancing
again, the 07-21..recovery hole must be closed with a **bounded, incremental**
pre-roll.

⚠️ **Use `preroll-incremental.sql` or a bounded variant — NEVER `preroll.sql`.**
The full script expects TRUNCATE-d coarse tables and would wipe every
already-pre-rolled row: that is precisely the [[0090]] history-loss incident.

## Step 5 — verify the data, not just the freshness

```sql
-- Per-source 1h counts vs the step-0 baseline. sdex must be unchanged;
-- phoenix should DROP (the delete finally applied); soroswap unchanged.
SELECT source, count() AS rows_1h FROM prices.price_ohlcv_1h
GROUP BY source ORDER BY rows_1h DESC;

-- Deep history intact — compare against the backup that predates all of this.
SELECT count() AS live FROM prices.price_ohlcv_1d FINAL WHERE timestamp < '2025-06-01';
SELECT count() AS backup FROM prices.price_ohlcv_1d_bak FINAL WHERE timestamp < '2025-06-01';
```

**Gate:** deep history matches the backup, and every per-source delta is
explainable by the Phoenix delete or RMT dedup. An unexplained drop means stop
and restore from `_bak`.

Then re-check the [[0072]] column this incident blocked:

```sql
SELECT countIf(change_7d_pct != 0) AS with_change_7d, count() AS assets
FROM prices.current_prices FINAL;
```

It populates on the next `mv_current_prices` refresh once `_1h` holds 7 days
again — so expect this to stay 0 until a week after recovery, or until the
07-21 gap is pre-rolled.

## Step 6 — close the detection gap

Nine days passed silently because eight of nine rollup MVs report success while
starving. **Recovery is not complete without this** — see [[0137]].

## Step 7 — release the cleanup hold

Only once step 5 has passed and the rollups have held for a watch period:
[[0105]] (drop the six `_bak` tables) is unblocked.

## Rollback

| Layer       | How                                                                     |
| ----------- | ----------------------------------------------------------------------- |
| Merge state | `SYSTEM STOP MERGES prices.<table>;` — returns to the frozen state      |
| Coarse data | restore from `prices.price_ohlcv_<g>_bak` (the reason [[0105]] is held) |
| Mutations   | **do NOT `KILL MUTATION`** — abandons a half-applied [[0097]] delete    |

Nothing in steps 1–3 is irreversible in the sense that matters: merges and
mutations are ordinary background operations this cluster performs hundreds of
thousands of times a day on every other table.
