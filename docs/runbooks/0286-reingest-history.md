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

| #   | Precondition                                                                                                                                                                                                                                                                                                                                                                                                                   | Check                                                                       |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------- |
| 1   | **Phases 1 and 2 are live on prod and MEASURED.** The first-week measurement of task 0286 phase 1 (residual high/low bias, share of `pf_trade_count = 0` minutes per source, share of candles the divergence flag marks) is recorded on the task. Re-ingesting 64 M ledgers with an unmeasured definition is the expensive way to discover a mistake.                                                                          | lore task 0286, phase-1 AC list                                             |
| 2   | **Task 0282 is settled and deployed.** Under ADR 0287 a later dust-only write of a minute REPLACES a priced one outright, and this run writes every minute of history again. 0282 is what makes a second write of a bucket add instead of replace.                                                                                                                                                                             | the ledger-processor and backfill on prod carry 0282's fix                  |
| 3   | **Cleanup is disabled and stays disabled for the WHOLE run** (task 0200). The 02:00 UTC rule drops whole `1m` monthly partitions older than a week — which is every partition this run touches.                                                                                                                                                                                                                                | `prices-production-cleanup` reads DISABLED                                  |
| 4   | **`prices.pool_registry` is seeded** for the Soroban era. `events-backfill` reprices only registered AMM contracts; an empty registry silently reprices nothing.                                                                                                                                                                                                                                                               | [`seed-pool-registry.md`](seed-pool-registry.md)                            |
| 5   | **BE's `default.transactions` covers the range.** It is where `application_order` — the transaction index of fill order (D1) — comes from on the events path. Where it does not reach, the ingest falls back to today's `(transaction_id, event_index)` order with `transaction_index = 0` and WARNs once. Count the coverage BEFORE the run and record the fallback share after it; it is a stated deliverable, not a detail. | §6                                                                          |
| 6   | **Disk headroom for the FREEZE snapshots** on the CH host, and for two copies of the month under repair.                                                                                                                                                                                                                                                                                                                       | `ssh … 'df -h /var/lib/docker'`                                             |
| 7   | **The snapshots are taken by the CH ADMIN, not by `prices_writer`.** `prices_writer` does not hold `ALTER FREEZE PARTITION` and **cannot be granted it** (`users.xml` is read-only storage).                                                                                                                                                                                                                                   | [`repair-coarse-usd-values.md`](repair-coarse-usd-values.md), preconditions |
| 8   | **Time and bandwidth are budgeted.** ~64 M ledgers, ~5.7 TB downloaded, ~18 days at the measured 178.3 k ledgers/hour on a home line (task 0088: 746 partitions / 689 676 890 rows / 3.13 TB in 12.2 days). On an EC2 in us-east-2 it is hours, not days.                                                                                                                                                                      | §3                                                                          |

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

Partitions are `toYYYYMM(timestamp)`; the backfill is ranged in LEDGERS. Derive
one from the other out of the rows already in the table — `version DIV 1000` is
the ledger a row's last fill came from:

```sql
SELECT toYYYYMM(timestamp)               AS month,
       min(toUInt64(version) DIV 1000)   AS first_ledger,
       max(toUInt64(version) DIV 1000)   AS last_ledger
FROM prices.price_ohlcv_1m
WHERE toYYYYMM(timestamp) IN (<month>, <next month>)
GROUP BY month ORDER BY month;
```

Use `START` = the month's `first_ledger` and `END` = the next month's
`first_ledger − 1`. Two consequences to write into the month's log line:

- A minute is only whole within one run, so the **first and last minute of every
  run are undercounted** — the documented partition/run-boundary residual
  (`running-ingestion-components.md`, "This gap is wider than the handoff"). List
  them; they are the accepted reconciliation delta of step 4f.
- Quiet months can have a ledger with no candle at either end. Round `START` DOWN
  and `END` UP to the enclosing 64 000-ledger partition boundary when you can:
  the flush boundary then coincides with the run boundary and no extra minute is
  split.

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

| Source                              | Expectation                                                                                                                                                                                                                                                |
| ----------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sdex`                              | **Equal to the stroop** on `volume_base`, `volume_quote` and `trade_count` — except the run-boundary minutes of step 4a, and except the live era, where the old rows were written by the live processor.                                                   |
| `soroswap` / `phoenix` / `aquarius` | **Greater than or equal**, never less. Task 0282 lost trades from every minute bucket that spanned a reconcile run; this ingest does not. Account for the delta **per source and per month** — an unexplained increase is as much a finding as a decrease. |
| any source                          | `count()` may differ: a minute that traded only dust still exists, so candles are not lost — but a minute whose every fill was dropped upstream for a zero amount never existed in either shape.                                                           |

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

**First, the coverage precondition** — the join is a LEFT join and a missing row
is not an error, it is a silent downgrade to `transaction_index = 0`:

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

**Record the fallback share.** The run WARNs once when the `application_order`
join is unusable. The deliverable is the number, per month:

```sql
SELECT countIf(transaction_index = 0) / count() AS fallback_share
FROM ( … the chunk's fills … );   -- or the WARN count from the run log
```

A month whose fallback share is not ~0 has its AMM fills ordered by today's
`(transaction_id, event_index)` key. That is the pre-0286 order: `open`/`close`
for those minutes are no better than they were. Say so in the log line rather
than letting the month pass as repaired.

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
   exists — and `post_run_0228_it` green.

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
