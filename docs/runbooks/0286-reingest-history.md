# Runbook — re-ingesting the candle history under ADR 0287 (task 0286, phase 3)

How to rebuild every candle the chain has ever produced from its raw fills, one
month at a time, so that a 2015 candle and a candle written tomorrow mean the
same thing.

**Why.** Phases 1 and 2 changed what a candle's prices MEAN: they come only from
the price-forming trades of the bucket — the fills whose price is the resting
offer's own `n/d`, or whose two amounts are large enough that rounding them by a
unit moves the price by less than 0.1 %. Rows written before that keep the old
meaning through the column DEFAULTs (`pf_trade_count DEFAULT trade_count` — "every
fill formed price"), which is honest but wrong: the seven dust days of
`docs/ohlcv-outlier-prints-analysis.md` are still ~26 % off, and through the
pivot tier they still drag every XLM-quoted asset with them. Only a re-ingest
repairs that. There is no in-place fix — the information needed (which fill
crossed which offer) exists nowhere but the ledgers.

**Applies to:** every monthly partition of `prices.price_ohlcv_1m` on ch-prod-01,
the six coarse tables rolled from it, and the whole-history re-enrichment that
follows.

**Read first:**
[`0286-candle-definitions-rollout.md`](0286-candle-definitions-rollout.md) — the
phase-1 rollout. This runbook assumes everything in it is done and green.

---

## 1. Preconditions

| #   | Precondition                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Check                                                                                                             |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------- |
| 1   | **Phases 1 and 2 are live on prod and MEASURED.** The first-week measurement of task 0286 phase 1 (residual high/low bias, share of `pf_trade_count = 0` minutes per source, share of candles the divergence flag marks) is recorded on the task. Re-ingesting 64 M ledgers with an unmeasured definition is the expensive way to discover a mistake.                                                                                                                                                                                                                                                                                                                                                                                                        | lore task 0286, phase-1 AC list                                                                                   |
| 2   | **Task 0282's fix (PR #313) is deployed and checked.** Under ADR 0287 a later dust-only write of a minute REPLACES a priced one outright. #313 does not make writes sum — it ends every live reconcile run on a whole minute, so the live processor no longer writes a minute twice. This run's own second-write paths are the run boundaries of §4a, which is why they must be minute-aligned.                                                                                                                                                                                                                                                                                                                                                              | the ledger-processor on prod is built from `2cb5b2b` or later; task 0282 records the full-day raw-vs-stored check |
| 3   | **Cleanup is disabled and stays disabled for the WHOLE run** (task 0200). The 03:00 UTC rule (`cron(0 3 * * ? *)`) drops whole `1m` monthly partitions older than a week — which is every partition this run touches.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | `prices-production-cleanup` reads DISABLED                                                                        |
| 4   | **`prices.pool_registry` is seeded** for the Soroban era. `events-backfill` reprices only registered AMM contracts; an empty registry silently reprices nothing.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | [`seed-pool-registry.md`](seed-pool-registry.md)                                                                  |
| 5   | ~~**BE's `default.transactions` covers the range.**~~ **VOID since 2026-09-17 (task 0304).** BE moved `application_order` onto `default.soroban_events` and dropped that table's `transaction_id`, so the events path reads the apply order straight off the event row — there is no join to miss, no coverage to count and no fallback share to record. The only value check left is that a NEGATIVE `application_order` degrades rather than wrapping.                                                                                                                                                                                                                                                                                                     | §6                                                                                                                |
| 6   | **Disk headroom for the FREEZE snapshots** on the CH host, and for two copies of the month under repair.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | `ssh … 'df -h /var/lib/docker'`                                                                                   |
| 7   | **The snapshots are taken by the CH ADMIN, not by `prices_writer`.** `prices_writer` does not hold `ALTER FREEZE PARTITION` and **cannot be granted it** (`users.xml` is read-only storage).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | [`repair-coarse-usd-values.md`](repair-coarse-usd-values.md), preconditions                                       |
| 8   | **Time and bandwidth are budgeted.** ~64 M ledgers, ~5.7 TB downloaded, ~18 days at the measured 178.3 k ledgers/hour on a home line (task 0088: 746 partitions / 689 676 890 rows / 3.13 TB in 12.2 days). On an EC2 in us-east-2 it is hours, not days.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | §3                                                                                                                |
| 9   | **For the AMM side of the LIVE-ERA months only (2026-07 onwards): task 0285's reverse question is answered.** 0285 measured stored soroswap trades at 4× the registry-joined raw count, and one reading is that the live processor writes candles for pools that are NOT in `prices.pool_registry`. `events-backfill` reprices registered pools only, so for such a month `DROP PARTITION` would delete candles the re-ingest cannot put back. SDEX (98 % of the candles) and every pre-live AMM month do not depend on this — their old rows were written under the same registry rule the re-ingest applies. If it is still open when the loop reaches those months, leave their AMM sources to the live processor (§5) and record them as un-re-ingested. | lore task 0285: does the live path write candles for pools absent from `pool_registry`?                           |

---

## 2. Why the loop has the shape it has

```
per month:  FREEZE  →  clear markers  →  DROP 1m partition  →  re-ingest
            →  reconcile  →  DROP the coarse partitions  →  pre-roll  →  next
1w and 1M:  once, at the very end
```

Four of those steps exist because of one fact and nothing else:

- **An old row OUTRANKS a re-ingested one.** `price_ohlcv_*` is a
  ReplacingMergeTree keyed on `version`, the ingest writes
  `version = max(ledger·1000 + operation_index)` — unchanged by this task — and
  the enrichment worker re-inserts every row it prices at `version + 1`, once per
  pass. So a re-ingested row arrives BELOW the row it is meant to replace and is
  silently discarded. **`DROP PARTITION` is not a tidiness step; without it the
  re-ingest is a no-op.** The same is true of the coarse partitions before the
  pre-roll.
- **The `backfill_sdex_ledgers` completion markers make the re-ingest a no-op
  too.** The SDEX backfill skips any ledger it has already indexed
  (`ingest.rs`), and every ledger of history is marked. Clear the month's markers
  or the run downloads nothing, writes nothing and exits successfully.
- **The 1w tier goes last** because a week straddles months: rolled per month it
  would be built from half a week of new days and half a week of old ones. The
  month (1M) rolls from the DAY since phase 1, so it could be done per month —
  it is kept to the end anyway, with 1w, because both are cheap from a finished
  `price_ohlcv_1d`.
- **FREEZE before the DROP** is the only rollback. It is hardlinks under
  `shadow/`, so it is cheap, and it is worthless if taken after the fact.

---

## 3. Cost, pacing, and what runs where

- Sized in whole **64 000-ledger archive partitions**; the backfill flushes per
  partition. Five partitions ≈ 320 000 ledgers ≈ 3–4 h on a home line.
- A month of Stellar is roughly 500 000 ledgers (5 s ledgers), i.e. ~8
  partitions — so a month is about a half-day of wall clock at home rates, and
  the whole chain is the ~18 days of precondition 8.
- Run everything under `tmux`. An interrupted month is resumable: the markers you
  cleared in step 4c are re-written per ledger as it completes, so a re-run of
  the same range picks up where it stopped (`--start` the same, it skips what is
  done).
- `events-backfill` reads `default.*` AND writes `prices.*`, so it runs **on the
  CH host as the `default` user** against `localhost:8123` — the prices mTLS user
  cannot read `default.*`
  ([`events-sourced-amm-reprice.md`](events-sourced-amm-reprice.md)).
- Never put a password in `argv` (`/proc/<pid>/cmdline` is world-readable). Use
  `read -rs CH_PW` and the environment, as the AMM runbook does.

---

## 4. The month loop

Everything below is per month `YYYYMM`, oldest first. Keep a log line per month:
the ledger range, the before/after reconciliation numbers, and the fallback share
from §6.

### 4a. Pick the month's ledger range

Partitions are `toYYYYMM(timestamp)`; the backfill is ranged in LEDGERS. The
range must be **minute-aligned**, not merely month-aligned: the accumulators live
for one invocation, so a minute split across two runs is written twice and the
second write — if its half is dust — ERASES the priced first one
(`0286-candle-definitions-rollout.md` §1a, the run-boundary row).

A month starts at a minute boundary (`00:00:00`), so a range that starts at the
first ledger of the month is aligned by construction:

- `START` = the FIRST ledger whose `closed_at` is at or after the month's first
  second. Read it from Horizon — `GET /ledgers/{seq}` returns `closed_at`, and
  `GET /ledgers?order=asc&cursor=…` walks the neighbourhood — and confirm that
  `closed_at(START − 1)` is in the previous month.
- `END` = the next month's `START − 1`. Nothing else: derive the next month's
  `START` the same way and subtract one, so the two ranges meet exactly on the
  minute edge.

To find the neighbourhood cheaply, the rows already in the table still help —
`version DIV 1000` is the ledger a row's last fill came from:

```sql
SELECT toYYYYMM(timestamp)               AS month,
       min(toUInt64(version) DIV 1000)   AS first_ledger,
       max(toUInt64(version) DIV 1000)   AS last_ledger
FROM prices.price_ohlcv_1m
WHERE toYYYYMM(timestamp) IN (<month>, <next month>)
GROUP BY month ORDER BY month;
```

but it answers "which ledgers wrote this month's rows", not "where does the
month start" — a quiet month has no candle at either end, so take its answer as
a starting point for the Horizon lookup, not as `START`.

Two more things to write into the month's log line:

- Round `START` DOWN and `END` UP to the enclosing 64 000-ledger partition
  boundary **when the month's first ledger happens to sit on one** — the flush
  boundary then coincides with the run boundary and nothing at all is split. Do
  not round otherwise: it would move `START` into the previous month.
- The backfill logs the alignment of BOTH ends of the run
  (`run boundary is minute-aligned`, INFO / `run boundary is NOT minute-aligned`,
  WARN). Record which of the two you got. A WARN means the straddling minute has
  to be rebuilt from ONE pass covering both sides before step 4f is meaningful.

### 4b. FREEZE, as CH admin

The `1m` partition, plus **every coarse partition that overlaps the month** —
15m/1h/4h/1d share the month's partition id; 1w and 1M can hold the month's
trades in the PREVIOUS partition id when the week or month starts earlier, so
freeze the neighbouring one too.

```sql
ALTER TABLE prices.price_ohlcv_1m FREEZE PARTITION 202403
  WITH NAME 'reingest_0286_prices_price_ohlcv_1m_202403';
```

Use `repair-coarse-usd-values.md` Step 3b's loop script verbatim with the
`reingest_0286_` prefix — including its `DIRECTORY_ALREADY_EXISTS` handling,
which reports `already-frozen` rather than overwriting an existing snapshot with
a post-DROP one and destroying the rollback point. Then confirm the snapshots
exist and are not empty:

```bash
ssh … 'docker exec app-clickhouse-1 ls /var/lib/clickhouse/shadow/ | grep -c reingest_0286_'
ssh … 'docker exec app-clickhouse-1 du -sh /var/lib/clickhouse/shadow/'
```

A near-zero `du` means nothing was captured — **stop.**

### 4c. Record the "before" numbers

This is what "reconciled against the snapshot" means in step 4f. Run it BEFORE
the DROP and keep the output:

```sql
SELECT source,
       count()            AS candles,
       sum(trade_count)   AS trades,
       sum(volume_base)   AS volume_base,
       sum(volume_quote)  AS volume_quote
FROM prices.price_ohlcv_1m FINAL
WHERE toYYYYMM(timestamp) = <month>
GROUP BY source ORDER BY source FORMAT TabSeparated;
```

(If you later need a row-level diff rather than these sums, attach the frozen
parts to a scratch table with the `ATTACH PARTITION … FROM '/var/lib/clickhouse/
shadow/…'` idiom in `repair-coarse-usd-values.md` → "Rollback".)

### 4d. Clear the month's completion markers

⚠️ **Skip this and the whole month is a silent no-op.** The backfill's resume set
short-circuits every ledger it has already indexed.

```sql
ALTER TABLE prices.backfill_sdex_ledgers DELETE WHERE sequence BETWEEN <START> AND <END>;
```

Mutations are asynchronous. Confirm it finished and the count is zero before the
run — not after:

```sql
SELECT count() FROM system.mutations
WHERE database = 'prices' AND table = 'backfill_sdex_ledgers' AND NOT is_done;
SELECT count() FROM prices.backfill_sdex_ledgers WHERE sequence BETWEEN <START> AND <END>;
```

**Both zero, or stop.** (`fix-backfill-history-loss-and-rerun.md` §5 does the
same thing with a `TRUNCATE`, because it was rebuilding the whole chain at once.
Do not truncate here — the markers of every other month are still the resume set
this run depends on.)

### 4e. DROP the month's `1m` partition, then re-ingest

```sql
ALTER TABLE prices.price_ohlcv_1m DROP PARTITION <month>;
```

Then, **one writer per source** (see the warning below):

```bash
tmux new -s reingest

export CH_DOMAIN=ch.sorobanscan.rumblefish.dev
export MTLS_CERT_PATH=$HOME/prices-mtls/prices_writer.crt
export MTLS_KEY_PATH=$HOME/prices-mtls/prices_writer.key
export MTLS_CA_PATH=$HOME/prices-mtls/ca.crt

./target/release/sdex-backfill \
  --mode sdex-only \
  --start <START> --end <END> \
  --tip <live tip> \
  --transport hetzner \
  --verbose
```

`--tip` is the live chain tip, so `progress_pct` means something:
`curl -s 'https://horizon.stellar.org/' | python3 -c 'import sys,json;print(json.load(sys.stdin)["core_latest_ledger"])'`.
For a Soroban-era month, expect and ignore the startup WARN about the mode not
matching the range; the AMM side of that month is §6's job.

⚠️ **Do not use `--mode combined` in phase 3.** Combined mode also derives AMM
candles from the archives, and §6's `events-backfill` writes the same
`(timestamp, asset, quote, source)` rows from BE's events — at the SAME `version`
(`ledger·1000 + operation_index` on both paths). Equal versions are not summed
and not ordered: whichever wrote last defines the row, and the month's AMM
volumes become a coin flip. One writer per source, always.

A healthy run prints `pre-flight: all checks passed`, then
`partition indexing complete` per partition, then
`=== sdex-backfill complete ===`.

### 4f. Reconcile against the "before" numbers

Re-run the step-4c query and compare, per source:

| Source                                                                                                                                      | Expectation                                                                                                                                                                                                                                                                                                                                                                                                             |
| ------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sdex`, outside the live-loss window                                                                                                        | **Equal to the stroop** on `volume_base`, `volume_quote` and `trade_count`. The run's two boundary minutes are WHOLE whenever step 4a logged `run boundary is minute-aligned` at both ends; a `NOT minute-aligned` WARN at either end makes that one minute an expected shortfall until it is rebuilt from a single pass.                                                                                               |
| any source, days written LIVE between 2026-07-16 and 2026-09-17 12:04 UTC (the break 0282 dates to the durable cursor, and the #313 deploy) | **HIGHER than the snapshot, by 0282's measured shortfall** — the snapshot is the damaged side here, so "equal" is the wrong expectation and would itself be a finding. Take the expected delta per source from task 0282 (Aquarius ~50 % of its trades; SDEX and Phoenix as that task records them) and log expected vs got per day. A day that comes back equal, or short, is a defect in one of the two measurements. |
| `soroswap` / `phoenix` / `aquarius`, outside that window                                                                                    | **Greater than or equal**, never less. Task 0282 lost trades from every minute bucket that spanned a reconcile run; this ingest does not. Account for the delta **per source and per month** — an unexplained increase is as much a finding as a decrease.                                                                                                                                                              |
| any source                                                                                                                                  | `count()` may differ: a minute that traded only dust still exists, so candles are not lost — but a minute whose every fill was dropped upstream for a zero amount never existed in either shape.                                                                                                                                                                                                                        |

Differences in `open`/`high`/`low`/`close` are the POINT of the exercise and are
not reconciled. Differences in volume are a defect. Record both.

### 4g. DROP the overlapping coarse partitions and pre-roll the month

The coarse rows for this month were rolled from the old `1m` and, for most of
them, enriched afterwards — so they outrank anything you roll now. Drop them
first, fine to coarse:

```sql
ALTER TABLE prices.price_ohlcv_15m DROP PARTITION <month>;
ALTER TABLE prices.price_ohlcv_1h  DROP PARTITION <month>;
ALTER TABLE prices.price_ohlcv_4h  DROP PARTITION <month>;
ALTER TABLE prices.price_ohlcv_1d  DROP PARTITION <month>;
```

Then roll the four fine tiers from the new `1m`, bounded to the month. Take the
statements from the repo file — never from a copy on the box, which is the
regression risk task 0145 names (`preroll-incremental-presoroban.md`):

```bash
# The first four statements of the GENERATED, parameterised pre-roll: 15m, 1h,
# 4h, 1d. 1w and 1M are deliberately left out until §7.
awk '/INSERT INTO prices\.price_ohlcv_1w/{exit} {print}' \
  packages/prices-clickhouse/schema/preroll-live-gap.sql > /tmp/0286_preroll_fine.sql

ssh … "docker exec -i app-clickhouse-1 clickhouse-client --multiquery \
   --param_start_ts='2024-03-01 00:00:00' --param_end_ts='2024-04-01 00:00:00' \
   --receive_timeout=86400 --max_bytes_before_external_group_by=4000000000" \
  < /tmp/0286_preroll_fine.sql && echo "PREROLL OK"
```

`{start_ts}` is inclusive, `{end_ts}` EXCLUSIVE, and the generator aligns each
tier's lower bound to its own bucket, so a 4 h bucket straddling the month start
is rebuilt whole rather than from its tail.

⚠️ **`preroll-incremental.sql` and `preroll-amm-reprice.sql` are HISTORICAL.**
They carry the pre-0286 candle definition, they roll the month from the week, and
their positional `INSERT … SELECT`s fail with Code 20 against the eighteen-column
tables — by design.

### 4h. Next month

Nothing carries between months: the ingest keeps no price state across minutes or
chunks, and each month's coarse rows are rebuilt from its own minutes.

---

## 5. Order of the months

Oldest first. Two reasons: the oldest months are the ones the analysis measured
(the seven dust days of 2023 and earlier), so a mistake shows up early; and the
newest month touches the live era, where the backfill's `--end` must stay below
the live cursor — **never let a backfill range reach the live ingest's cursor for
the same source** (`running-ingestion-components.md`, "Safe operating rules").
Hand the live era over on a minute boundary, or leave it to the live processor
entirely and record it as un-re-ingested.

---

## 6. The Soroban AMM history

The AMM side of every month at or after the Soroban activation ledger
**50 457 424** comes from `events-backfill`, not from the archives: it is the
path that carries BE's `application_order` (the D1 transaction index) and the
full event group per swap.

⚠️ **The coverage precondition below is VOID (task 0304).** It existed because
`application_order` came from a `LEFT JOIN default.transactions`, where a
missing row was a silent downgrade to `transaction_index = 0`. Since 2026-09-17
the column is on the event row itself and the join is gone. The query is kept
for the record; it answers a question nothing asks any more:

```sql
-- On the CH host, as `default`. Per month of the range.
SELECT toYYYYMM(toDateTime(closed_at)) AS month, count() AS transactions
FROM default.transactions FINAL
WHERE ledger_sequence BETWEEN <START> AND <END>
GROUP BY month ORDER BY month;
```

Then dry-run, then run:

```bash
read -rs CH_PW
CLICKHOUSE_PASSWORD="$CH_PW" ~/events-backfill \
  --start <START> --end <END> \
  --clickhouse-url http://localhost:8123 --dry-run --verbose

CLICKHOUSE_PASSWORD="$CH_PW" ~/events-backfill \
  --start <START> --end <END> \
  --clickhouse-url http://localhost:8123 --verbose
```

It chunks at 320 k ledgers and keeps the open minute across chunks, so its own
chunk boundaries are safe; its RUN boundary is not — use the same `START`/`END`
as the month's SDEX run so both sides share one boundary minute.

~~**Record the fallback share.**~~ **VOID since 2026-09-17 (task 0304).** The
deliverable existed because the `application_order` join could be unusable for a
whole month, leaving that month's AMM fills in the pre-0286
`(transaction_id, event_index)` order while the run reported success. The join
is gone: `application_order` is a non-nullable column on every event row, so a
month cannot be silently ordered the old way any more.

What survives is the value check. `application_order` is `Int16`, and a negative
value is not a position — `resolve_transaction_index` degrades it to 0 and counts
it. The run's last summary line reports it:

```
negative apply order:      0
```

**Anything but `0` means those fills ARE in the wrong order and the month is NOT
repaired** — do not record it as done. The run also emits one WARN, once, naming
the first ledger and `application_order` it saw.

---

## 7. After the last month: 1w, 1M, and the re-enrichment

**7a. The two wide tiers, once, over the whole range.** A week straddles months,
so it is only correct once every month is done.

```sql
TRUNCATE TABLE prices.price_ohlcv_1w;
TRUNCATE TABLE prices.price_ohlcv_1M;
```

Then run the `1w` and `1M` statements of `schema/preroll.sql` (the full-range
file; `1M` reads `price_ohlcv_1d AS t FINAL` since phase 1 renamed the month's
source). The MV `mv_ohlcv_1d_to_1M` keeps them current afterwards.

**7b. Re-enrich the whole history.** Every re-ingested row carries
`close_usd = 0` and `volume_quote_usd = 0`, so the enrichment worker will re-price
the entire estate — this is the cost the ledger estimate in precondition 8 does
NOT include. For scale: task 0268's campaign re-priced 9.94 M candles in ~24 min;
0228 estimated ~190 M candles at ~4 h 40 m. This is all of them, and it runs on
the same worker under the same concurrency.

**7b-2. Re-price the coarse tiers. The drain above never reaches them.** The
enrichment worker prices `price_ohlcv_1m` and nothing else
(`CLICKHOUSE_TABLE: 'price_ohlcv_1m'`,
`infra/src/lib/stacks/eventbridge-stack.ts:483`). The coarse sweep covers only
the current and the previous month (`COARSE_SWEEP_LOOKBACK_MONTHS: '2'`, same
file `:627`). The rollup MVs re-aggregate only a recent window (`rollups.sql`).
Every coarse row that §4g and §7a rolled was built from a `1m` that had not been
priced yet, so it was written with `close_usd = 0`. Without this step every
15m/1h/4h/1d/1w/1M row older than two months keeps that zero for good, and
~130 unguarded `argMax(close_usd, …)` sites read it as a price.

The fix is the ordinary fill-the-zeros mode of `coarse-repair`
([`repair-coarse-usd-values.md`](repair-coarse-usd-values.md), Steps 1–5), run
over the whole history on all six tables. It does not read `1m`. It prices each
coarse row from that table's own reference candles, with the same tiers as the
worker, including the measured USDC rate on the pivot.

- **Start only when the `1m` drain is finished.** Both passes load the same
  shared cluster, and 7c/7e measure the finished state. "Finished" means the
  frontier gauge reads `0`. Run this from the local machine:

  ```bash
  aws cloudwatch get-metric-statistics \
    --namespace Prices/Enrichment --metric-name EnrichmentFrontierMonthsPending \
    --dimensions Name=Environment,Value=production \
    --statistics Maximum --period 3600 \
    --start-time "$(date -u -d '-3 hour' +%FT%TZ)" --end-time "$(date -u +%FT%TZ)"
  ```

  You need at least one datapoint, and every datapoint must read
  `Maximum` = `0`. An empty `Datapoints` list means the drain has stopped
  publishing. It does not mean the drain is done.

- **Pass NO `--reset-*` flag.** A reset discards stored values. Here nothing
  is wrong, only missing, and 7d already rules the 0228 reset out. Every refusal
  the reset modes carry is skipped when no `--reset-quote-asset-id` is given,
  and none of them applies to this step. That covers `--snapshots-verified`,
  the `--pivot-window-s` floor, the plain-reset-on-a-pivot-leg refusal and the
  task-0208 epoch check (`assert_reset_is_admissible` returns at once).
- **`--skip-snapshot` is required, and here it is safe.** It is required
  because `prices_writer` cannot `FREEZE` (precondition 7). Without the flag
  the first month fails with `FreezeDenied`. It is safe because this mode only
  writes rows still at `close_usd = 0` and never discards a value. It is also
  safe because §4g and §7a can rebuild every month's coarse rows from its
  re-ingested `1m`. The tool warns "only safe when the 1m source can rebuild
  this coarse table", and that condition is true here. **Exception:** a month
  recorded as un-re-ingested (§5, precondition 9) whose `1m` is gone. Its
  coarse partitions are the only copy, so freeze them before the run with
  `repair-coarse-usd-values.md` Step 3b's script, using the `repair_0286_`
  prefix and that month as the range.
- **Go from fine to coarse, one table at a time:** `15m`, `1h`, `4h`, `1d`,
  `1w`, `1M`. The `1d→1w` (60 d) and `1d→1M` (400 d) MVs re-roll recent buckets
  from `1d`. Once `1d` is priced, those re-rolls carry its rate rather than a
  zero.

**① [operator box, in `tmux`] Build, and set the range.**

```bash
tmux new -s coarse0286

cd ~/stellar-prices-api && git checkout develop && git pull --ff-only
cargo build --release -p enrichment-worker --features aws-mtls --bin coarse-repair

export CH_DOMAIN=ch.sorobanscan.rumblefish.dev
export MTLS_CERT_PATH=$HOME/prices-mtls/prices_writer.crt
export MTLS_KEY_PATH=$HOME/prices-mtls/prices_writer.key
export MTLS_CA_PATH=$HOME/prices-mtls/ca.crt

# The sweep owns the current and the previous month, so stop two months back.
END=$(date -u -d "$(date -u +%Y-%m-15) -2 month" +%Y%m); echo "END=$END"

# A 1w/1M row's reference can be the previous bucket, which a one-day ASOF
# window silently drops (coarse-repair --help, --pivot-window-s).
pivot_window() { case "$1" in price_ohlcv_1w) echo 604800 ;; price_ohlcv_1M) echo 2678400 ;; *) echo 86400 ;; esac; }
```

`--start-month 201501` below is the lowest month the tool accepts. The tool
lists only months that hold enrichable zeros, so an empty month costs nothing.

**② [operator box] Dry run, all six tables. It writes nothing.**

```bash
for TBL in price_ohlcv_15m price_ohlcv_1h price_ohlcv_4h price_ohlcv_1d price_ohlcv_1w price_ohlcv_1M; do
  echo "===== $TBL ====="
  ./target/release/coarse-repair \
    --transport hetzner --table "$TBL" \
    --start-month 201501 --end-month "$END" \
    --pivot-window-s "$(pivot_window "$TBL")" \
    --dry-run
done 2>&1 | tee /tmp/0286_coarse_dry.log
```

Expect every month from the table's first candle to `$END`, with `zeros_before`
close to the month's count of candles that have volume. **A table that reports
`0 month(s)` is a STOP**, not an all-clear. After a re-ingest there is nothing
already priced to explain it. Budget the run from the dry run's total: the rate
measured under task 0276 was ~7 k rows/s, and the repair runbook's Appendix C
explains why that is a floor. Time the first month and re-plan from that.

**③ [CH host, via SSH] Baseline.** Keep the output. This script is also the
after-check in ⑤.

```bash
cat > /tmp/0286_coarse_zeros.sh <<'EOF'
#!/bin/sh
# Per coarse tier: every close_usd = 0 row OLDER than the coarse sweep's
# two-month lookback, split by why it is zero. Read-only.
for TBL in price_ohlcv_15m price_ohlcv_1h price_ohlcv_4h price_ohlcv_1d price_ohlcv_1w price_ohlcv_1M; do
clickhouse-client --format PrettyCompactNoEscapes -q "
WITH
  (SELECT any(asset_id) FROM prices.assets FINAL
    WHERE asset_code = 'XLM' AND issuer_address = '' AND contract_address = '') AS xlm,
  (SELECT any(asset_id) FROM prices.assets FINAL
    WHERE asset_code = 'USDC' AND contract_address = ''
      AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN') AS usdc,
  (SELECT any(asset_id) FROM prices.assets FINAL
    WHERE asset_code = 'USDT' AND contract_address = ''
      AND issuer_address = 'GCQTGZQQ5G4PTM2GL7CDIFKUBIPEC52BROAQIAPW53XBRJVN6ZJVTG6V') AS usdt,
  (SELECT ifNull(minOrNull(timestamp), toDateTime('2106-01-01 00:00:00')) FROM prices.$TBL FINAL
    WHERE asset_id = xlm  AND quote_asset_id = usdc AND close > 0) AS xlm_ref_from,
  (SELECT ifNull(minOrNull(timestamp), toDateTime('2106-01-01 00:00:00')) FROM prices.$TBL FINAL
    WHERE asset_id = usdt AND quote_asset_id = usdc AND close > 0) AS usdt_ref_from
SELECT
  '$TBL' AS tbl,
  countIf(close_usd = 0) AS zero_usd,
  countIf(close_usd = 0 AND volume_quote = 0) AS no_volume,
  countIf(close_usd = 0 AND volume_quote > 0 AND close = 0) AS no_price,
  countIf(close_usd = 0 AND volume_quote > 0 AND close > 0
          AND quote_asset_id NOT IN (xlm, usdc, usdt)) AS other_quote,
  countIf(close_usd = 0 AND volume_quote > 0 AND close > 0
          AND ((quote_asset_id = xlm  AND timestamp < xlm_ref_from)
            OR (quote_asset_id = usdt AND timestamp < usdt_ref_from))) AS before_reference,
  countIf(close_usd = 0 AND volume_quote > 0 AND close > 0
          AND (quote_asset_id = usdc
            OR (quote_asset_id = xlm  AND timestamp >= xlm_ref_from)
            OR (quote_asset_id = usdt AND timestamp >= usdt_ref_from))) AS reachable_left
FROM prices.$TBL FINAL
WHERE timestamp < toStartOfMonth(now() - INTERVAL 1 MONTH)"
done
EOF

scp -i ~/.ssh/sorban-prod_ed25519 /tmp/0286_coarse_zeros.sh deploy@168.119.73.161:/tmp/
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker cp /tmp/0286_coarse_zeros.sh app-clickhouse-1:/tmp/ && docker exec app-clickhouse-1 sh /tmp/0286_coarse_zeros.sh' \
  | tee /tmp/0286_coarse_zeros_before.txt
```

The cutoff, `toStartOfMonth(now() - INTERVAL 1 MONTH)`, is the first instant
the sweep covers. Everything below it belongs to this step.

**④ [operator box, same `tmux`] The real run, one table per command.** Read
each summary before you start the next table.

```bash
TBL=price_ohlcv_15m   # then price_ohlcv_1h, _4h, _1d, _1w, _1M
./target/release/coarse-repair \
  --transport hetzner --table "$TBL" \
  --start-month 201501 --end-month "$END" \
  --pivot-window-s "$(pivot_window "$TBL")" \
  --skip-snapshot 2>&1 | tee "/tmp/0286_coarse_${TBL}.log"
```

The repair runbook's "Log lines that look like failures but are not" applies
unchanged. In particular, every month before XLM/USDC's first candle
(2021-02) reports `enriched 0`, because nothing can price it. **Stop** on
either of these:

- a month reporting exactly `200000` enriched, which means `one_shot` did not
  take effect;
- `enriched 0` on any month from 2021-03 on. Every such month holds
  USDC-quoted candles, so a zero there is a silent no-op.

**⑤ [CH host, via SSH] After-check, per tier.** Re-run ③ and write the output
to `/tmp/0286_coarse_zeros_after.txt`. Per table:

| Column             | Meaning                                                                                                   | Required after the run                                                               |
| ------------------ | --------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `reachable_left`   | Quoted in USDC, or in XLM/USDT from that leg's first USDC candle on. A tier can price these.              | **~0.** Every remaining row must be explained with the drill-down below.             |
| `before_reference` | Quoted in XLM/USDT before that leg's own USDC market exists in this table. Nothing can price them.        | Unchanged from the baseline. This is the explained residue (the task-0208 boundary). |
| `other_quote`      | No peg or pivot path. From 2025-09 the oracle tier prices some; the rest is the `no_reference` floor.     | Equal to the baseline or lower. Never higher.                                        |
| `no_price`         | `close = 0`: a bucket with no price-forming fill. `0` is the correct USD value (ADR 0287, `rollups.sql`). | Unchanged. The repair cannot and must not touch it.                                  |
| `no_volume`        | `volume_quote = 0`, which no tier prices.                                                                 | Unchanged.                                                                           |
| `zero_usd`         | The sum of the five above.                                                                                | Roughly the baseline minus the total the run's summary lines report as enriched.     |

If `reachable_left` is not ~0, list where the rows sit. Swap in the table (run
it in `clickhouse-client` on the CH host):

```sql
WITH
  (SELECT any(asset_id) FROM prices.assets FINAL
    WHERE asset_code = 'XLM' AND issuer_address = '' AND contract_address = '') AS xlm,
  (SELECT any(asset_id) FROM prices.assets FINAL
    WHERE asset_code = 'USDC' AND contract_address = ''
      AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN') AS usdc,
  (SELECT any(asset_id) FROM prices.assets FINAL
    WHERE asset_code = 'USDT' AND contract_address = ''
      AND issuer_address = 'GCQTGZQQ5G4PTM2GL7CDIFKUBIPEC52BROAQIAPW53XBRJVN6ZJVTG6V') AS usdt,
  (SELECT ifNull(minOrNull(timestamp), toDateTime('2106-01-01 00:00:00')) FROM prices.price_ohlcv_1h FINAL
    WHERE asset_id = xlm  AND quote_asset_id = usdc AND close > 0) AS xlm_ref_from,
  (SELECT ifNull(minOrNull(timestamp), toDateTime('2106-01-01 00:00:00')) FROM prices.price_ohlcv_1h FINAL
    WHERE asset_id = usdt AND quote_asset_id = usdc AND close > 0) AS usdt_ref_from
SELECT toYYYYMM(timestamp) AS month, quote_asset_id, count() AS candles,
       min(close) AS min_close, max(close) AS max_close
FROM prices.price_ohlcv_1h FINAL
WHERE timestamp < toStartOfMonth(now() - INTERVAL 1 MONTH)
  AND close_usd = 0 AND volume_quote > 0 AND close > 0
  AND (quote_asset_id = usdc
    OR (quote_asset_id = xlm  AND timestamp >= xlm_ref_from)
    OR (quote_asset_id = usdt AND timestamp >= usdt_ref_from))
GROUP BY month, quote_asset_id ORDER BY candles DESC LIMIT 30;
```

Two shapes are an explained residue:

- the leg's USDC market was silent for longer than `--pivot-window-s` before
  the bucket (a thin month of 2021–2022, clustered by month);
- a `close` so small that `close × rate` truncates to 0 at 14 decimal places.
  `max_close` shows this, as in the repair runbook's Appendix A triage.

Anything else is a defect. This mode only ever selects rows still at zero, so
re-running ④ for that table is safe. It re-prices only what is still
missing.

Record the before and after tables on task 0286. 7c's `post_run_0228_it` reads
the coarse tiers, so it can pass only after this step.

**Rollback.** Nothing was discarded. To undo one table-month, `DROP` that
coarse partition and re-run §4g for the month (§7a for `1w`/`1M`). The re-roll
now reads a priced `1m`.

**7c. The 0228 after-check**, on the now-repaired XLM/USDC pivot reference.
Capture the `VERSION_BEFORE` values **before** the enrichment campaign starts:

```bash
CLICKHOUSE_URL=… CH_DATABASE=prices \
POST_RUN_0228_VERSION_BEFORE_1W=… POST_RUN_0228_VERSION_BEFORE_1M=… \
  cargo test -p enrichment-worker --test post_run_0228_it -- --ignored
```

(From a laptop against prod: drop `CLICKHOUSE_URL`, export `CH_DOMAIN` and the
`MTLS_*` paths, add `--features aws-mtls`.) Both tests are expected to fail
before and pass after.

**7d. Task 0228's reset-mode campaign is SUPERSEDED, not run.** Its purpose was
to refill `close_usd` on rows the pivot had skipped; the re-ingest zeroes those
columns anyway and 7b refills them from scratch. Record it as superseded on the
task — running it in addition would be a second full pass over the estate for no
new rows.

**7e. The USDT-peg after-check (task 0212).** `price_ohlcv_1m` went into this
run holding 1 564 045 USDT-quoted rows valued at `close × $1` (2018-05-15 →
2026-08-13) — about 7.4× the real rate; tasks 0172 and 0182 repaired the writer
and the coarse tiers and never touched `1m`. This run replaces every one of
those rows and 7b re-prices them, so 0212 needs no repair of its own — **but
only if the USDT pivot actually writes** (it never had until task 0215; confirm
`CAST(111 AS UInt32) AS ref_asset_id` statements in `system.query_log` during
7b, not only XLM's `CAST(4 …)`). Otherwise the rows end at zero instead of
wrong, which is the trade 0172 made on 2026-08-13.

Measure it on `1m`, not on the tiers the pre-roll just wrote:

```sql
-- 111 = the canonical USDT on prod (task 0209). Its real rate is ~0.14.
SELECT
    countIf(close_usd / close <  0.5) AS pivot_written,
    countIf(close_usd / close >= 0.9) AS peg_written,
    min(timestamp) AS oldest_priced, max(timestamp) AS newest_priced
FROM prices.price_ohlcv_1m FINAL
WHERE quote_asset_id = 111 AND close > 0 AND close_usd > 0;
```

Expected: `peg_written = 0`, `pivot_written > 0` over the same span. Run it once
more on `price_ohlcv_1h` to confirm the pre-roll carried no peg value up.

---

## 8. Acceptance

The phase-3 criteria of lore task 0286, in the order they can be checked:

1. **Every monthly partition re-ingested**, with its step-4f reconciliation
   recorded: SDEX equal to the stroop (boundary minutes and the live era
   documented), AMM sources `≥` with the delta explained per source and month.
2. **The coarse tiers pre-rolled from the new `1m`**, and OHLC ordering intact on
   each — there is no clamp anywhere in the rollup, so this holds only because
   every price aggregate reads the same price-forming subset:

   ```sql
   SELECT count() AS violations FROM prices.price_ohlcv_1d FINAL
   WHERE NOT (low <= open AND low <= close AND open <= high AND close <= high);
   ```

3. **The seven dust days.** XLM/USDC `1d` closes on the days named in
   `docs/ohlcv-outlier-prints-analysis.md` are within 5 % of Bitstamp's. This is
   the measurement the "last price-forming fill" rule was never tested against —
   it is measured here, on the 1 181-day set, and recorded on the task.
4. **No candle priced from a quantised XLM/USDC close.** Task 0278's before-figure
   was 22 760 / 143 577 / 85 699 / 30 064 / 10 938 candles on
   15m / 1h / 4h / 1d / 1w. After the re-ingest and pre-roll it is **zero**.
5. **The whole history re-enriched** — `close_usd > 0` wherever a reference
   exists, on `1m` (7b's drain) AND on every coarse tier (step 7b-2:
   `reachable_left` ~0 on each of the six tables, every other column
   explained) — and `post_run_0228_it` green.
6. **No USDT-quoted `1m` row carries the $1 peg** (step 7e): `peg_written = 0`
   and `pivot_written > 0` on `1m`, and on one coarse tier. This closes task 0212.

---

## 9. Rollback

Per month, while its snapshots still exist:

```sql
ALTER TABLE prices.price_ohlcv_1m DROP PARTITION <month>;
ALTER TABLE prices.price_ohlcv_1m ATTACH PARTITION <month>
  FROM '/var/lib/clickhouse/shadow/reingest_0286_prices_price_ohlcv_1m_<month>/';
```

Same for each coarse table dropped in 4g. The completion markers cleared in 4d do
NOT need restoring — they are backfill bookkeeping, and the restored rows are
already the indexed result.

Release a month's snapshots only once you are certain, and only after its
reconciliation is recorded:

```sql
SYSTEM UNFREEZE WITH NAME 'reingest_0286_prices_price_ohlcv_1m_<month>';
```

---

## Related

- [`0286-candle-definitions-rollout.md`](0286-candle-definitions-rollout.md) —
  phase 1: the schema, the workers, the MVs. A precondition for this runbook.
- [`repair-coarse-usd-values.md`](repair-coarse-usd-values.md) — the FREEZE /
  ATTACH / UNFREEZE idiom, and the grant `prices_writer` does not hold.
- [`continue-soroban-backfill.md`](continue-soroban-backfill.md) — the
  `sdex-backfill` invocation, the `tmux` + mTLS block, and the per-chunk
  verification queries.
- [`events-sourced-amm-reprice.md`](events-sourced-amm-reprice.md) — the
  `events-backfill` invocation, its run identity, and the pool-registry
  precondition.
- [`fix-backfill-history-loss-and-rerun.md`](fix-backfill-history-loss-and-rerun.md)
  — the precedent for clearing the resume markers, and the pre-roll's memory and
  timeout flags.
- [`running-ingestion-components.md`](running-ingestion-components.md) — the
  disjoint-range rule and the boundary-minute residual this run inherits.
- ADR 0287 §1–§8; lore task 0286, phases 2 and 3.
