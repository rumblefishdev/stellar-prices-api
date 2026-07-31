# Runbook — recovering the frozen coarse OHLCV rollups (task 0136)

Restores `price_ohlcv_15m`, `_1h`, `_4h`, `_1d`, `_1w`, `_1M`, frozen since
**2026-07-21 02:44 UTC**. Diagnosis, evidence and both local test harnesses are
in `lore/1-tasks/backlog/0136_BUG_coarse-rollup-tables-frozen-since-2026-07-21.md`.

**Mechanism.** Background merges are inert on exactly these six tables. Trace
logging on ch-prod-01 showed they emit **no merge-machinery lines at all**, while
`price_ohlcv_1m` and the rest of the cluster log normally: ClickHouse is not
declining to merge them, it is **never asking**. Each table's background
operations assignee — created in `IStorage::startup()` — is not running. That
sits one level below every merge-policy setting, which is why every knob checked
was innocent and why `SYSTEM START MERGES` did nothing (it releases a lock on a
decision; there is no process left to make the decision).

From there the failure is mechanical: `mv_ohlcv_1m_to_15m` appends once a minute,
nothing merges the parts, `price_ohlcv_15m` reaches `parts_to_throw_insert =
5000` and starts rejecting inserts with `Code: 252 TOO_MANY_PARTS`. Its rows stop
advancing, so `_1h` reads an empty recent window, appends nothing, and reports
success — and the starvation cascades down the chain in silence.

**Recovery: `DETACH TABLE` + `ATTACH TABLE`, one table at a time.** `ATTACH`
constructs a fresh storage object and runs `startup()`, rebuilding the assignee.
It is the per-table equivalent of a server restart, scoped to one table in one
database.

> **A cluster restart is explicitly out of scope.** ch-prod-01 is shared with BE
> and we are authorised to touch `prices.*` only. Every command in this runbook
> names a single `prices` table.

**Trigger: unknown, and it does not change the recovery.** Both 07-17 operations
were ours ([[0095]] backup + APPEND MV recreate, [[0097]] STAGE 0 phoenix
delete), and a controlled local test showed a pending mutation alone does **not**
freeze a table. Do not spend time on attribution before recovering.

## What the local test established (2026-07-31)

`scripts/test-0136-detach-attach-recovery.sh`, local docker CH **26.3.10.60**
(the prod pin). Four tests, all green:

| Test                                                                                                          | Result                                                                                                                              |
| ------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| **T4 — prod-shaped.** Table pinned at `parts_to_throw_insert`, inserts failing with the identical `Code: 252` | detach 106 ms / attach 105 ms → **parts 30 → 1 within 10 s, INSERT accepted again**                                                 |
| **T1 — pending mutation.** Wedged table carrying an unexecuted `ALTER DELETE`                                 | mutation **survives** the detach and **completes** after attach (`is_done` 0 → 1); parts 12 → 3; `MergeParts` + `MutatePart` resume |
| **T3 — dependent MV.** `DETACH` of a refreshable MV's `TO` target while the MV is live                        | **allowed**; MV errors `Code: 60` during the window only, then **self-recovers after `ATTACH` with no recreate**                    |
| **Data integrity**                                                                                            | `count() FINAL` unchanged (200 → 200) across detach/attach                                                                          |

### Limits of that evidence — read before trusting it

- **The local wedge is built with `SYSTEM STOP MERGES`, which production
  falsified.** The test proves `DETACH`/`ATTACH` rebuilds a table's scheduler and
  clears in-memory state; it cannot prove it clears prod's specific unknown
  cause. The mechanism argument is what carries: `ATTACH` builds a new storage
  object from scratch, so no in-memory state survives it.
- **`OPTIMIZE` as a by-hand stopgap is UNVALIDATED.** It returned `Code: 236
Cancelled merging parts` locally — but only because the simulation stops
  merges, which prod does not. Untested either way; not used in this runbook.
- **Do not quote the 105 ms attach for production.** That was 30 parts.
  `price_ohlcv_15m` has ~5,000 part descriptors to load at attach time. Expect
  meaningfully slower, and see the hang gate in Step 1.

## Preconditions

- [ ] Read "what recovery will change" below. Row counts on the coarse tables
      **will** move — that is the intended effect of a pending delete plus
      deduplication, not a fault.
- [ ] **The six `price_ohlcv_*_bak` tables must still exist.** They are the only
      restore path. [[0105]] (drop the backups) is **blocked until this task is
      verified** — confirm it has not been run.
- [ ] Off-peak if possible. This generates real background I/O on a cluster
      shared with BE.
- [ ] BE notified that coarse `prices` data is stale and about to move.
- [ ] A **second SSH session** open before you start, so a hung `DETACH` can be
      observed via `system.processes` without waiting on the first one.

## What recovery will change

Three things happen at once, and all are wanted:

1. **Merges collapse the accumulated parts.** `ReplacingMergeTree` deduplicates
   by `version` during merge, so duplicate-PK rows written by the [[0097]]
   idempotent pre-roll collapse. **Physical** row counts drop hard — locally
   2,400 → 466 on a test table.
2. **`count() FINAL` should NOT change**, because those duplicates were never
   visible to readers. Confirmed locally. This is the sharper version of the
   older "row counts will drop" warning: storage shrinks, readers see the same
   data.
3. **The six pending Phoenix deletes execute** (1,601 parts to rewrite in total).
   This _is_ a real logical change: `phoenix` rows are removed from parts that
   existed when the mutation was created on 07-17. Rows inserted **after** that
   point are untouched by it — a `ALTER DELETE` only applies to parts existing at
   creation time — so expect a partial, not total, phoenix reduction.

Neither 1 nor 3 is data loss. But capture counts first (Step 0) so "the numbers
moved" can be told apart from "the numbers moved _wrongly_".

> ⚠️ **`count()` is a useless health probe during recovery.** RMT dedup makes it
> fall while writes are landing perfectly — during the local test a target table
> read 800 → 400 while its MV was inserting every 5 seconds. **Use
> `system.part_log`** (`NewPart` proves inserts land, `MergeParts` proves merging
> resumed). Every gate below is written against `part_log` for this reason.

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

SELECT '=== FINAL counts — these should NOT move ===' AS section;
SELECT 'price_ohlcv_15m' AS t, count() AS rows_final FROM prices.price_ohlcv_15m FINAL
UNION ALL SELECT 'price_ohlcv_1h', count() FROM prices.price_ohlcv_1h FINAL
UNION ALL SELECT 'price_ohlcv_1M', count() FROM prices.price_ohlcv_1M FINAL;

SELECT '=== backups still present? (0105 must NOT have run) ===' AS section;
SELECT name, metadata_modification_time, total_rows
FROM system.tables WHERE database='prices' AND name LIKE '%_bak' ORDER BY name;

SELECT '=== pending mutations ===' AS section;
SELECT table, mutation_id, parts_to_do, is_done FROM system.mutations
WHERE database='prices' AND is_done=0 ORDER BY table;

SELECT '=== uptime ===' AS section;
SELECT uptime() AS seconds, now() - uptime() AS started_at;
SQL
```

**Also capture the table definitions** — the safety net if an `ATTACH` ever needs
to be replaced by a hand-written `CREATE`:

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec -i app-clickhouse-1 clickhouse-client --multiquery' <<'SQL' > 0136-create-table-backup.sql
SHOW CREATE TABLE prices.price_ohlcv_15m;
SHOW CREATE TABLE prices.price_ohlcv_1h;
SHOW CREATE TABLE prices.price_ohlcv_4h;
SHOW CREATE TABLE prices.price_ohlcv_1d;
SHOW CREATE TABLE prices.price_ohlcv_1w;
SHOW CREATE TABLE prices.price_ohlcv_1M;
SQL
```

**Gate:** six `_bak` tables present; six mutations still `is_done = 0`; the
`SHOW CREATE` file is non-empty and saved **off the cluster**. Save all output —
it is the baseline for every later comparison.

## Step 1 — probe on the smallest leaf table

`price_ohlcv_1M` is the **leaf** of the rollup chain (nothing rolls up from it),
the smallest by part count, and not on the critical path. If the approach is
wrong, this is the cheapest possible place to find out.

Run detach and attach as **one invocation** so there is no human-sized gap
between them:

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec -i app-clickhouse-1 clickhouse-client --multiquery' <<'SQL'
DETACH TABLE prices.price_ohlcv_1M;
ATTACH TABLE prices.price_ohlcv_1M;
SQL
```

> ⚠️ **Plain `DETACH`, never `DETACH ... PERMANENTLY`.** Plain detach leaves the
> metadata file in place, so `ATTACH TABLE prices.price_ohlcv_1M` reads it
> straight back — and even if the attach failed outright, the table returns by
> itself at the cluster's next restart. It cannot be lost.

**If the command does not return within ~60 seconds, it is hung.** From the
second SSH session:

```sql
SELECT query_id, elapsed, substring(query,1,80) AS q FROM system.processes
WHERE query ILIKE '%DETACH%' OR query ILIKE '%ATTACH%';
```

A hung `DETACH` means a background task is holding a lock — the second live
theory for the freeze. **Stop there.** Do not touch the other five tables. The
table is unavailable until it resolves; escalate rather than improvise.

Then watch for ~5 minutes:

```sql
-- did merging actually resume? (this is the gate — NOT count())
SELECT event_type, count() AS n, max(event_time) AS latest
FROM system.part_log
WHERE database='prices' AND table='price_ohlcv_1M'
  AND event_time > now() - INTERVAL 10 MINUTE
GROUP BY event_type ORDER BY n DESC;

SELECT count() AS parts FROM system.parts
WHERE database='prices' AND table='price_ohlcv_1M' AND active;

SELECT table, parts_to_do, is_done FROM system.mutations
WHERE database='prices' AND table='price_ohlcv_1M';
```

**Decision gate:**

- **`MergeParts` appears in `part_log` / parts fall from 750 / `parts_to_do`
  drops from 60** → approach **confirmed**. Continue to Step 2.
- **`part_log` still empty after ~5 minutes** → approach **dead**. Stop. Do not
  detach the remaining five. Record the negative result in 0136 — it is as
  valuable as a positive one, and it would mean the assignee failure survives a
  storage rebuild, which is new information.

## Step 2 — unblock the chain head

`price_ohlcv_15m` is the actual blocker: wedged at exactly
`parts_to_throw_insert = 5000`, which is why `mv_ohlcv_1m_to_15m` throws and
everything downstream starves. This is the step that ends the freeze.

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec -i app-clickhouse-1 clickhouse-client --multiquery' <<'SQL'
DETACH TABLE prices.price_ohlcv_15m;
ATTACH TABLE prices.price_ohlcv_15m;
SQL
```

**Expect the `ATTACH` to take noticeably longer than Step 1** — it loads ~5,000
part descriptors. Apply the same 60-second hang gate, but do not confuse _slow_
with _hung_: check `system.processes` before concluding anything.

`mv_ohlcv_1m_to_15m` will fail its refresh during the detach window with `Code:
60 ... does not exist`. That is expected and self-correcting — verified in the
local T3 test, where the MV resumed on its own after attach with no recreate.

Watch until parts drop **well below 5,000** (aim < 1,000):

```sql
SELECT count() AS parts, sum(rows) AS rows FROM system.parts
WHERE database='prices' AND table='price_ohlcv_15m' AND active;
```

This is the slow step — 5,027 parts / 4.55 GiB, plus a 52-part mutation. Expect
tens of minutes, not seconds. **Do not proceed while parts are still falling.**

Once below the threshold, the MV should succeed on its next 1-minute refresh:

```sql
SELECT view, status, last_success_time, substring(exception,1,120) AS exception
FROM system.view_refreshes WHERE view = 'mv_ohlcv_1m_to_15m' FORMAT Vertical;

-- corroborate with the thing that cannot lie: inserts landing again
SELECT max(event_time) AS newest_insert FROM system.part_log
WHERE database='prices' AND table='price_ohlcv_15m' AND event_type='NewPart';
```

**Gate:** `exception` empty, `last_success_time` within the last two minutes, and
a `NewPart` newer than the attach. That is the moment the nine-day freeze ends.

## Step 3 — the remaining four

Only after Step 2 is green. **One at a time**, largest first since `_1h` is the
next bottleneck, re-running the Step 1 `part_log` gate after each:

```sql
DETACH TABLE prices.price_ohlcv_1h;  ATTACH TABLE prices.price_ohlcv_1h;   -- 6,325 parts / 7.79 GiB — heaviest
DETACH TABLE prices.price_ohlcv_4h;  ATTACH TABLE prices.price_ohlcv_4h;   -- 3,124 parts / 4.02 GiB
DETACH TABLE prices.price_ohlcv_1d;  ATTACH TABLE prices.price_ohlcv_1d;   -- 1,395 parts / 1.64 GiB
DETACH TABLE prices.price_ohlcv_1w;  ATTACH TABLE prices.price_ohlcv_1w;   --   811 parts / 615 MiB
```

Between each, confirm the pool is not saturated and BE's tables are not starving:

```sql
SELECT count() AS active, countIf(is_mutation) AS mutations FROM system.merges;
SELECT metric, value FROM system.metrics
WHERE metric IN ('BackgroundMergesAndMutationsPoolTask','BackgroundMergesAndMutationsPoolSize');
```

If `BackgroundMergesAndMutationsPoolTask` approaches the pool size, **pause** —
let it drain before the next table. There is no deadline here.

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
recent window; they do not reach back ten days. Once the tips are advancing
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

-- FINAL counts vs step 0 — these should be unchanged except for phoenix.
SELECT 'price_ohlcv_15m' AS t, count() AS rows_final FROM prices.price_ohlcv_15m FINAL
UNION ALL SELECT 'price_ohlcv_1h', count() FROM prices.price_ohlcv_1h FINAL
UNION ALL SELECT 'price_ohlcv_1M', count() FROM prices.price_ohlcv_1M FINAL;

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

Ten days passed silently because eight of nine rollup MVs report success while
starving. **Recovery is not complete without this** — see [[0137]].

## Step 7 — release the cleanup hold

Only once Step 5 has passed and the rollups have held for a watch period:
[[0105]] (drop the six `_bak` tables) is unblocked.

## Rollback

| Layer                                 | How                                                                                                                                                 |
| ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Table missing after a failed `ATTACH` | Re-run `ATTACH TABLE prices.<table>;` — plain detach leaves the metadata file in place. Last resort: recreate from the Step 0 `SHOW CREATE` capture |
| Coarse data                           | restore from `prices.price_ohlcv_<g>_bak` (the reason [[0105]] is held)                                                                             |
| Mutations                             | **do NOT `KILL MUTATION`** — abandons a half-applied [[0097]] delete                                                                                |

There is no "undo" for a successful recovery, and none is wanted: merges and
mutations are ordinary background operations this cluster performs hundreds of
thousands of times a day on every other table. The six tables have simply been
denied them for ten days.

## Appendix — superseded attempt (2026-07-30)

`SYSTEM START MERGES prices.price_ohlcv_1M` was run and **failed its gate**: six
minutes on, parts unchanged at 750, `parts_to_do` unchanged at 60, `part_log`
empty. The stopped-merges hypothesis is **falsified in production**.

Kept because it is what forced the trace-logging pass that produced the real
finding, and because `SYSTEM START MERGES` will keep looking like the obvious
first move to anyone reading this cold. It is not. It releases a lock on a
decision that nothing is currently making.
