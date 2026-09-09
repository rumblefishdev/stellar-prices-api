# Runbook — repair missing USD values in the coarse OHLCV tables (task 0114)

Fills `close_usd` / `volume_quote_usd` in the coarse forever-tables
(`price_ohlcv_1h`, `_4h`, `_1d`, `_1w`, `_1M`) for the Soroban era, where the
rollup path captured pre-enrichment zeros and never revisited them. Hard
100%-zero block is **2025-02 → 2026-02**; the full repair span is **2024-02 →
present**.

The tool is `enrichment-worker`'s `coarse-repair` binary. It is **additive-only**
(re-inserts corrected rows the `ReplacingMergeTree` collapses by higher
`version`) and **partition-bounded** (one month at a time, so cost is independent
of table size).

It can also `FREEZE` each partition before touching it, but **in practice the
operator takes the snapshots** — `prices_writer` cannot hold the required
privilege (precondition 7 / Step 3b). Either way, every partition is snapshotted
before it is written.

> ⚠️ For 2025-02 → 2026-02 the `price_ohlcv_1m` source was dropped by cleanup on
> 2026-07-18. The coarse tables are the **sole surviving copy**. NEVER truncate or
> rebuild them — this tool never does; do not substitute the pre-roll's
> clean-slate path. **Never run without a verified snapshot**, whoever took it.

---

## Which tables

Only the **forever-tables**: `price_ohlcv_1h`, `_4h`, `_1d`, `_1w`, `_1M`.
`price_ohlcv_15m` has a 30-day retention (cleanup drops it), so it holds no deep
history to repair, and its recent 30 days are enriched live. Run the tool once
per forever-table.

## Preconditions

| #   | Check                                                                                                                                                                                                                           | How                                                   |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| 1   | **prices-api owner sign-off** — heavy RMT rewrites on the shared `ch-prod-01`.                                                                                                                                                  | —                                                     |
| 2   | **Low-traffic window** picked.                                                                                                                                                                                                  | Same courtesy as the pre-roll runbook.                |
| 3   | **Not colliding with the 0088 pre-roll.** The tail backfill is pre-Soroban (≤2020); this repair is 2024-02+. Disjoint partitions — safe to run concurrently, but confirm 0088's step-3 pre-roll is not writing the same months. | [[continue-soroban-backfill]]                         |
| 4   | **Disk headroom on the CH host** for `FREEZE` snapshots (hardlinks under `shadow/`, cheap, but non-zero). Pure SQL — **no container restart**.                                                                                  | `ssh … 'df -h /var/lib/docker'`                       |
| 5   | **mTLS writer certs present** on the operator box.                                                                                                                                                                              | `$HOME/prices-mtls/prices_writer.{crt,key}`, `ca.crt` |
| 6   | Version-arithmetic + additive-only already proven by test (`ch_enrich_it.rs`).                                                                                                                                                  | no action                                             |
| 7   | **`ALTER FREEZE PARTITION` — decide the snapshot path.** `prices_writer` does **not** hold it, and cannot be granted it (see below), so the tool's built-in `FREEZE` fails. Use the operator pre-freeze path in Step 3b.        | `SHOW GRANTS FOR CURRENT_USER`                        |

> **Snapshots must be taken by the CH admin, not by the tool** (learned the hard
> way on the 2026-07-23 prod run). `coarse-repair` connects as `prices_writer`,
> which holds only `SELECT, INSERT, ALTER DELETE, OPTIMIZE`. Its `FREEZE` is
> refused with `ACCESS_DENIED` — and the clickhouse driver surfaces that as a
> bare `Clickhouse(BadResponse(""))` with no message, so replay the statement
> over `curl` to see the real error.
>
> `GRANT ALTER FREEZE PARTITION ON prices.* TO prices_writer` does **not** fix
> it: `prices_writer` is defined in `users.xml`, which is read-only storage, so
> the grant fails with `ACCESS_STORAGE_READONLY` **even as the container's
> superuser**. Nobody can grant it at runtime. Editing the XML on a shared prod
> cluster is not worth it for a one-off repair — take the snapshots as admin
> instead (Step 3b) and run with `--skip-snapshot`.

---

## Step 0 — land the code

The `coarse-repair` bin and the partition-bounding change must be merged and built
from a real branch — do not run prod off an uncommitted working tree.

```bash
# from the repo, on the branch carrying the 0114 code
/branch            # feat branch off the 0114 work
/pr                # open PR → develop; CI must be green
# merge after review, then pull on the operator box
```

## Step 1 — build the tool on the operator box

```bash
cd ~/stellar-prices-api && git checkout develop && git pull --ff-only
# aws-mtls is REQUIRED for --transport hetzner
cargo build --release -p enrichment-worker --features aws-mtls --bin coarse-repair
```

Binary lands at `./target/release/coarse-repair`.

Point at the writer certs (same env as sdex-backfill):

```bash
export CH_DOMAIN=ch.sorobanscan.rumblefish.dev
export MTLS_CERT_PATH=$HOME/prices-mtls/prices_writer.crt
export MTLS_KEY_PATH=$HOME/prices-mtls/prices_writer.key
export MTLS_CA_PATH=$HOME/prices-mtls/ca.crt
```

## Step 2 — DRY RUN first (writes nothing)

Preview the months-with-zeros for each table. This only reads.

```bash
for TBL in price_ohlcv_1h price_ohlcv_4h price_ohlcv_1d price_ohlcv_1w price_ohlcv_1M; do
  echo "===== $TBL ====="
  ./target/release/coarse-repair \
    --transport hetzner --table "$TBL" \
    --start-month 202402 --end-month 202607 \
    --dry-run
done
```

Expect a per-month table showing `zeros_before` counts, heaviest across
2024-02 → 2026-02. `enriched = 0`, nothing written.

## Step 3 — record the baseline (before)

Hand this to the prod CH shell and **keep the output** — it is the before/after
evidence the ACs require. Repeat per table (`1h` shown).

`FINAL` is mandatory — these are `ReplacingMergeTree` tables and an
un-collapsed read counts superseded rows, inflating `pct_zero`.

```bash
cat > /tmp/0114_cov.sql <<'SQL'
SELECT toYYYYMM(timestamp) AS month,
       count() AS rows,
       countIf(close_usd = 0) AS zero_usd,
       round(100 * zero_usd / rows, 1) AS pct_zero
FROM prices.price_ohlcv_1h FINAL
WHERE volume_quote > 0 AND toYYYYMM(timestamp) BETWEEN 202402 AND 202607
GROUP BY month ORDER BY month FORMAT PrettyCompact;
SQL

ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec -i app-clickhouse-1 clickhouse-client' < /tmp/0114_cov.sql
```

## Step 3b — snapshot every affected partition (as CH admin)

Because of precondition 7 the operator takes the snapshots, not the tool. Freeze
**before** repairing, and use the tool's own naming so the rollback and cleanup
steps below apply unchanged.

```bash
cat > /tmp/0114_freeze.sh <<'EOF'
#!/bin/sh
# Freeze every active partition of $1 in the repair span. Idempotent-ish: an
# already-frozen partition errors with DIRECTORY_ALREADY_EXISTS, which is
# reported as `already-frozen` rather than silently counted as success — a
# re-freeze would otherwise overwrite a pre-repair snapshot with a post-repair
# one, destroying the rollback point.
TBL="$1"
clickhouse-client -q "SELECT DISTINCT partition FROM system.parts
  WHERE database='prices' AND table='$TBL' AND active
    AND toUInt32(partition) BETWEEN 202402 AND 202607
  ORDER BY partition FORMAT TabSeparated" |
while read p; do
  NAME="repair_0114_prices_${TBL}_${p}"
  if clickhouse-client -q "ALTER TABLE prices.$TBL FREEZE PARTITION $p WITH NAME '$NAME'" 2>/tmp/freeze_err; then
    echo "frozen $TBL $p"
  elif grep -q DIRECTORY_ALREADY_EXISTS /tmp/freeze_err; then
    echo "already-frozen $TBL $p (pre-existing snapshot KEPT)"
  else
    echo "FAILED $TBL $p: $(cat /tmp/freeze_err)" >&2
  fi
done
EOF

scp -i ~/.ssh/sorban-prod_ed25519 /tmp/0114_freeze.sh deploy@168.119.73.161:/tmp/
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker cp /tmp/0114_freeze.sh app-clickhouse-1:/tmp/ && docker exec app-clickhouse-1 sh /tmp/0114_freeze.sh price_ohlcv_1h' \
  | tee /tmp/0114_freeze_1h.log
```

Verify the count matches the months the dry run reported, and that the snapshots
hold real data, **before** any write:

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec app-clickhouse-1 ls /var/lib/clickhouse/shadow/ | grep -c repair_0114_prices_price_ohlcv_1h_'
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec app-clickhouse-1 du -sh /var/lib/clickhouse/shadow/'
```

A near-zero `du` means the partitions were not captured — stop.

## Step 4 — the real run (snapshot ON), one table at a time

Start with `price_ohlcv_1h` (the table BE consumes). Review its result before
moving to the next table.

Run under `tmux` — the full `1h` span measured **~7 min/month**, so budget 3–4 h.

```bash
tmux new -s repair0114
./target/release/coarse-repair \
  --transport hetzner --table price_ohlcv_1h \
  --start-month 202402 --end-month 202607 \
  --skip-snapshot 2>&1 | tee /tmp/0114_run_1h.log
```

> `--skip-snapshot` is correct **only because Step 3b already took the
> snapshots**. The tool prints a loud warning saying partitions have no backup;
> that warning is false here and the `ls`/`du` output from Step 3b is the
> evidence. Without Step 3b the warning is exactly as serious as it sounds — this
> span is the sole surviving copy.

Keep the default batch size. `--batch-size 100000` cuts per-batch re-scans but
raises per-query memory on a shared cluster (the concern tracked by task 0113).

The summary prints per month: `zeros_before | enriched | zeros_after |
snapshot`. `zeros_after` settling to a small residual is expected — those are
exotic quotes with no USD path (the genuine `no_reference` floor), not a failure.

### Log lines that look like failures but are not

Do **not** abort the run on these. They are written for the live 1m pass, where
they do signal a real fault; on the historical coarse repair they are normal.

| Log line                                                                                               | Why it is expected here                                                                                                                                                                                                                                  |
| ------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `enrichment pass enriched 0 rows despite a non-empty backlog — check oracle↔asset-id reconciliation …` | Emitted for any month that is **entirely exotic quotes**. The preceding `peg-pivot tier made no progress — remaining candles have no USD reference (exotic quotes)` is the accurate description. 202402 is wholly like this (~620 k rows, `enriched 0`). |
| `oracle tier drained — handing remaining candles to peg-pivot tier`, `remaining` unchanged             | `prices.oracle_prices` starts **2025-09**, so the oracle tier no-ops across most of the span **by design**. The peg-pivot / stablecoin-direct tiers do the work.                                                                                         |
| `snapshot DISABLED for this partition` (per month)                                                     | Correct under the Step 3b path — the operator already froze it. Verify once via `ls`/`du`, then ignore the repetition.                                                                                                                                   |

**These are the signals that DO mean stop:**

- A month reporting **exactly 200,000** enriched — `one_shot` did not take
  effect; `coarse-repair.rs` hard-codes `max_batches: 20`, which at the default
  10 k batch caps a run at 200 k rows. Not a data limit.
- `enriched 0` on a month known to hold **peg- or pivot-reachable** rows (any
  month 2025-01 onward holds 150 k–340 k USDC-quoted rows) — that is the silent
  no-op this repair exists to avoid.
- A `FreezeDenied` error — snapshots are missing; see precondition 7.

### Timing

Measured 2026-07-23 on `price_ohlcv_1h`: **~7 min per month** at ~2.8 M
candidates, so the 30-month span runs **4–5 h**. Cost scales with candidate
count, not partition size. The dry run's scan rate (~12 M rows/s) does **not**
predict this — it measures enumeration only, and underestimated the run by ~2×.

**Merge pressure outlives the process.** Re-inserted RMT rows keep collapsing in
background merges after the tool exits. `zeros_after` reads `FINAL` so it is
correct immediately, but the shared host keeps working for a while.

Then repeat for `_4h`, `_1d`, `_1w`, `_1M`, one at a time.

## Step 5 — verify (after)

**a) Coverage dropped.** Re-run the Step-3 query. `pct_zero` for 2024-02 → 2026-02
should fall from 86–100% to the exotic-only floor.

**b) Check the reference, not the value ceiling.** The correct test is whether
the implied USD reference is right for the era — **not** whether every
`close_usd` looks sane. Some inputs are junk and the repair faithfully
multiplies them (see the dust-trade note below), so a value ceiling will always
"fail" on data the repair cannot fix.

```sql
SELECT b.asset_code AS base, q.asset_code AS quote,
       round(toFloat64(p.close_usd) / nullIf(toFloat64(p.close), 0), 8) AS implied_ref_usd,
       count() AS rows
FROM prices.price_ohlcv_1h p FINAL
JOIN prices.assets b FINAL ON b.asset_id = p.asset_id
JOIN prices.assets q FINAL ON q.asset_id = p.quote_asset_id
WHERE toYYYYMM(p.timestamp) = <month> AND p.close_usd > 0
GROUP BY base, quote, implied_ref_usd ORDER BY rows DESC LIMIT 20
```

`implied_ref_usd` must be **1.0 exactly** for USDC/USDT-quoted rows (the
stablecoin-direct 1:1 cast) and the **real XLM/USD price for that month** for
XLM-quoted rows. On the 2026-07-23 pilot over 202502 it was 0.312–0.408, which
is correct for February 2025 — the strongest evidence the pivot tier works.

> **Dust-trade candles — expected, not a repair defect.** A tail of absurd
> `close_usd` values survives the repair (202502: 40 rows > $1M, max $29.6M,
> 0.004%). Every one is a single dust trade — `trade_count = 1`, a few XLM of
> volume, a nonsense unit price like 94,810,046 XLM/token — multiplied by a
> _correct_ reference. Live-enriched data has the same tail (`price_ohlcv_1h`
> 202607, written by the live path: max $24.0M, 0.0015%), so this predates the
> repair and is inherited from source candles. `volume_quote_usd` is unaffected
> (those rows carry ~$3 of volume), so BE's volume analytics do not see it.
> Tracked separately as a data-quality task.

**c) Spot-check liquid pairs against an independent source.** Pick a couple of
well-known pairs and eyeball `close_usd` against a known market price for that
month (e.g. CoinGecko historical). It should be in the right ballpark.

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --query=\"
     SELECT a.asset_code, toDate(timestamp) AS d,
            round(toFloat64(close), 6) AS close_quote,
            round(toFloat64(close_usd), 6) AS close_usd
     FROM prices.price_ohlcv_1d p
     JOIN prices.assets a ON a.asset_id = p.asset_id
     WHERE p.timestamp BETWEEN toDateTime('2025-06-01') AND toDateTime('2025-06-02')
       AND p.close_usd > 0
     ORDER BY p.volume_quote DESC LIMIT 10 FORMAT PrettyCompact\""
```

## Step 6 — the recurring guard (automated: its own Lambda)

> ⚠️ **Changed by task 0218 (2026-08-24).** The guard used to be folded into the
> enrichment Lambda, running after its 1m pass. It is now
> **`prices-<env>-coarse-sweep`**, its own crate, function and EventBridge rule
> (`cron(30 * * * ? *)`), because sitting behind the 1m pass's `?` meant it was
> **skipped** whenever that pass errored and **starved** when it ran long — it
> never executed in production at all. Everything below about _what it does_ still
> holds; the operational details (which function carries the env vars, the off
> switch, the alarms) are updated in place.

The one-off historical repair (Steps 0–7) and the ongoing guard are the same job.
The guard used to be **not** a separate cron or a separate Lambda — folded into the
existing hourly `enrichment` Lambda (`prices-<env>-enrichment`), which already
owns `close_usd` for `price_ohlcv_1m`. After each 1m pass it also re-sweeps the
recent coarse partitions, so any USD value the rollup path freezes going forward
is corrected **within the hour** instead of permanently. Permanent corruption
becomes at-most-one-hour lag.

**Why a sweep is needed at all (and why it isn't "just make enrichment correct").**
The rollup MVs re-aggregate only a bounded recent window (`1m→15m` 2 h, `15m→1h`
8 h, `1h→4h` 1 day, …; `rollups.sql`). A 1m row enriched _after_ its window closes
— routine enrichment lag, or a multi-day stall like the 0111 outage — never rolls
its correction up, so the coarse row stays zero forever. Making enrichment fast
enough that lag never exceeds the window is the _root_ fix (**task 0111**); this
sweep is the **backstop** for when it doesn't, and this system's ordering has
broken repeatedly (cursor freeze, cleanup mid-backfill, the 4-day outage).

### What it does each run

- Runs on **its own hourly schedule**, independent of the 1m pass.
  ⚠️ Since 0218 a sweep failure **fails its own invocation** (it is no longer
  swallowed): there is no 1m pass to protect, and a swallowed error was
  indistinguishable from a run that swept nothing. The enrichment worker is
  unaffected either way — separate function, separate alarm.
- **Bounded** (`one_shot = false`): each table/month runs at most
  `COARSE_SWEEP_MAX_BATCHES` batches, then stops; overflow defers to the next
  hourly run, so a run cannot approach the 5-min timeout.
- **Partition-bounded to a trailing window** recomputed from the ClickHouse server
  clock each run — so it only ever touches recent partitions, never a
  full-history scan (task 0111).
- **No snapshot** — recent live-era partitions are not the sole copy and
  `prices_writer` cannot FREEZE anyway. Same additive `INSERT … SELECT` as the
  manual repair, so it is non-destructive by construction.
- **Steady state is cheap:** recent partitions already sit at the `no_reference`
  floor, so each table/month early-exits after ~2 no-op batches.

### Enable / disable / tune — env vars on the enrichment Lambda

Set in CDK (`infra/src/lib/stacks/eventbridge-stack.ts`, the enrichment Lambda's
`environment`). Takes effect on the next deploy of the `*-EventBridge` stack.

| var                             | default                               | meaning                                                                                                                                                                                                                                        |
| ------------------------------- | ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `COARSE_SWEEP_TABLES`           | `price_ohlcv_15m,_1h,_4h,_1d,_1w,_1M` | comma list of coarse tables to sweep. **This is the on/off switch — clear it to disable the sweep, no code change.** Non-coarse names (`price_ohlcv_1m` / typos) are dropped at cold start with a warning; they do NOT fire the failure alarm. |
| `COARSE_SWEEP_LOOKBACK_MONTHS`  | `2`                                   | trailing months swept, inclusive of the current month (2 = current + previous). Covers month-boundary rollups + multi-day lag.                                                                                                                 |
| `COARSE_SWEEP_MAX_BATCHES`      | `20`                                  | per-tier batch budget per month; caps a catch-up run.                                                                                                                                                                                          |
| `COARSE_SWEEP_TIME_BUDGET_SECS` | `120`                                 | wall-clock budget per invocation; the sweep stops after this long (and always a margin before the Lambda deadline) and defers the rest, so a slow catch-up can never hit the hard timeout and fail the invocation.                             |

> **Emergency off switch (no deploy):** set **`prices-<env>-coarse-sweep`**'s
> `COARSE_SWEEP_TABLES` env var to empty — ⚠️ **not** the enrichment Lambda,
> which no longer carries it (task 0218). The next run logs
> `coarse sweep disabled` and does nothing; the 1m pass is unaffected because it
> is a different function. A redeploy restores the CDK value.
>
> To stop it entirely, disable the `prices-<env>-coarse-sweep` EventBridge rule —
> but note that trips the `-no-invocations` alarm after three hours, by design.

Note `_15m` **is** in the recurring scope even though it is excluded from the
_historical_ repair (see §Which tables): the historical repair skips it because
its 30-day retention holds no deep history to fix, but the guard keeps its recent
~30 days correct going forward (the 2-month lookback simply finds less `_15m`
data, since cleanup has dropped the older partition — harmless).

### Metrics to watch (`Prices/Enrichment` namespace, `Environment` dimension)

- `CoarseSweepRowsEnriched` — coarse rows corrected per run. **Steady state ≈ 0**
  once the tables sit at the floor. A _sustained_ non-zero is the actionable
  signal: the rollup path is re-freezing zeros, enrichment lag is exceeding the MV
  windows (task 0111 territory) and the guard is earning its keep.
- `CoarseSweepTableFailures` — tables whose pass **errored** this run. **Alarm on
  `> 0`** — the dead-sweep signal. Config skips are deliberately excluded so a
  benign typo can't false-fire it.
- `CoarseSweepRuns` / `CoarseSweepFailedRuns` (task 0218) — every completed run
  reports itself, so the three states are distinguishable: **no datapoint at
  all** = never ran (the `-no-invocations` alarm treats missing data as
  breaching), `Runs=1 FailedRuns=0` = ran, `Runs=1 FailedRuns=1` = ran and failed.
- `CoarseSweepDeadlineHit` / `CoarseSweepDurationMs` / `CoarseSweepTablesSwept`
  (task 0218) — a run cut short by its wall-clock budget. A sustained `1` means
  `COARSE_SWEEP_TIME_BUDGET_SECS` is too small for the backlog, and tables at the
  tail of the list are being deferred every run.

⚠️ Since 0218 the sweep has its **own** `prices-<env>-coarse-sweep-errors`,
`-duration-near-timeout` and `-no-invocations` alarms. The enrichment worker's
`-errors` alarm no longer has anything to do with the sweep in either direction.

- `CoarseSweepTablesSkipped` — non-coarse names left in the config. Informational
  hygiene, not an alarm series.

> No `CoarseSweepRowsRemaining` metric is published: the trailing window's
> `zeros_after` is dominated by the permanent multi-million exotic `no_reference`
> floor, so it would sit near-constant whether or not the sweep is keeping up —
> useless as a lag signal. `RowsEnriched` is the signal; the floor size comes from
> the composition query on demand.

### Verifying it after deploy

Use the same per-quote-class composition query as §Step 5: in the current month
the reachable classes (stablecoin, XLM-pivot) should stay near-zero `pct_zero`
and the exotic class stays 100% (by design). If a reachable class climbs over
time, the sweep is falling behind — check that `CoarseSweepRowsEnriched` is
non-zero (it is trying) and investigate enrichment lag (task 0111).

## Step 7 — clean up the snapshots

Once satisfied the repair is good and you no longer need the rollback point,
release the frozen snapshots (frees the `shadow/` hardlinks). The names are in
the Step-4 summary output (`repair_0114_prices_<table>_<month>`).

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --query=\"
     SYSTEM UNFREEZE WITH NAME 'repair_0114_prices_price_ohlcv_1h_202502'\""
# repeat per snapshot name reported by the run.
```

---

## Rollback — if a month's repair looks wrong

Each repaired partition has a frozen "before" snapshot. To revert one month:

1. Locate the snapshot on the CH host: it is under
   `/var/lib/clickhouse/shadow/repair_0114_prices_<table>_<month>/`.
2. Detach the current (repaired) partition, then attach the frozen copy back:

```bash
# on the CH host, via the prod client. <month> is the numeric YYYYMM.
ALTER TABLE prices.<table> DETACH PARTITION <month>;
ALTER TABLE prices.<table> ATTACH PARTITION <month>
  FROM '/var/lib/clickhouse/shadow/repair_0114_prices_<table>_<month>/';
```

3. Verify the partition’s USD columns match the pre-repair state, then
   `SYSTEM UNFREEZE WITH NAME …`.

> The additive design means a bad run does **not** destroy the pre-repair rows at
> write time (the frozen snapshot is untouched hardlinks), so this restore is
> always available until you unfreeze. Automating this as a `--revert` flag is a
> tracked follow-up.

## Appendix A — reset mode, for a _wrong_ value rather than a missing one (task 0182)

Everything above fills zeros and is purely additive. This appendix covers the one
mode that **discards a stored value**. Read it in full before using the flags.

### When this applies

The repair above cannot see a row whose `close_usd` is wrong but non-zero —
every tier filters on `close_usd = 0`, which is exactly what makes them
idempotent. After a _pricing_ defect that is a problem: task 0172 found that
USDT-quoted candles had been valued at par by a peg tier, and those 44,657 rows
are inert. The writer is fixed; nothing will ever revisit what it already wrote.

⚠️ **A plain dry run over this shape reports "no months with enrichable zeros".**
Before task 0182 that all-clear was indistinguishable from a genuinely clean
table. The `--reset-*` flags widen the month enumeration so those rows count.

### The flags

```bash
--reset-quote-asset-id <ID>   # the quote leg to re-open
--reset-not-before <UNIX_TS>  # epoch below which stored values are left alone
```

They require each other. Together they re-insert the matching rows with **both**
USD columns at 0 and `version + 1`, ahead of the normal tiers, which then
recompute them.

### What reset mode refuses outright

All five are hard errors, not warnings, because each one ends with rows zeroed
that nothing can refill:

| Refusal                                                        | Why                                                                                                                                                                                                                                                                                                                                |
| -------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--skip-snapshot` + `--reset-*` without `--snapshots-verified` | Rollback for a bad reset **is** `ATTACH PARTITION` from the frozen copy. On prod `--skip-snapshot` is the _correct_ flag (Step 3b: the admin freezes, `prices_writer` cannot), so it is not refused — but "the admin did it" and "nobody did it" must not look identical. Verify under `shadow/`, then add `--snapshots-verified`. |
| `--pivot-window-s` below the table's bucket width              | On `_1w`/`_1M`/`_1d` a bucket whose reference is the previous bucket falls outside a short window. Before a reset that left a row unenriched; now it discards the value first.                                                                                                                                                     |
| A quote leg that is not a peg or pivot reference               | A mistyped id (`11` for `111`) passes the oracle check, because an unknown asset has no oracle rows either.                                                                                                                                                                                                                        |
| A bounded pass (`one_shot = false`)                            | The peg-pivot tier is gated on the oracle tier draining, so a bounded pass can defer the only tier that refills.                                                                                                                                                                                                                   |
| `oracle_prices` rows for the quote leg                         | See below.                                                                                                                                                                                                                                                                                                                         |

### The epoch is not optional tuning

Below the date the pivot's reference market begins there is nothing to recompute
from, so a reset row stays at `close_usd = 0` **permanently** — an ambiguous zero
read unguarded by ~130 `argMax(close_usd, …)` sites, which is worse than the
wrong number it replaced.

For canonical USDT (`asset_id = 111` on prod) the epoch is **2021-02-07 =
`1612656000`**, the start of its USDC market. Task 0172 separately measured it at
genuine par until June 2022, so the `$1` already stored below that date is
_correct_ — this flag protects real data, it does not merely skip work.

### Prerequisite: purge the oracle rows FIRST

The oracle tier runs **before** the peg-pivot tier and wins where it applies. If
`prices.oracle_prices` still holds rows for the quote leg, the reset is undone by
the next statement in the same pass, and the run reports a healthy repair over
unchanged values — now labelled `method = 'oracle'`, which reads as _more_
authoritative than what it replaced.

The tool refuses rather than letting that happen:

```
USD reset refused: prices.oracle_prices still holds N row(s) for quote asset_id …
```

That is task 0196 (done for USDT on 2026-08-13). If you see this error, purge and
verify 0 before re-running — do not work around it.

### Extra verification, beyond Step 5

The summary gains a `reset` column per month and a closing line:

```
NNN row(s) re-opened by the USD reset, NNN recomputed
```

They should match. If `rows_reset` exceeds `rows_enriched` the tool prints a loud
block on stderr naming the shortfall — that run zeroed values it could not
recompute. **Stop; do not continue to the next table.**

> ⚠️ **Triage before you roll back.** The shortfall has a known false positive —
> see the next section. On the 2026-08-18 `_1d` run it fired for 8 rows and the
> correct action was to continue, not to roll back.

### The shortfall has a known false positive: a correct value of zero

`rows_enriched` is a **population difference, not a count of rows written**. The
pass measures `candidates_before` _after_ the reset has re-opened its rows
(`ch_enrich.rs:986-996` — deliberately, or the repair would score itself zero),
then accumulates the drop in the number of rows still at `close_usd = 0`. So a
row the pivot writes with a computed value of **zero** never leaves the candidate
population and is never counted as enriched — even though it was recomputed
exactly as designed. The tier then re-selects it, makes no progress, and stops.

`close_usd = r.usd × close`, stored as `Decimal(38, 14)`. Two shapes produce a
legitimate zero:

| shape                  | before the reset | after                                                        |
| ---------------------- | ---------------- | ------------------------------------------------------------ |
| `close = 0`            | `0 × $1` = 0     | `0 × rate` = 0 — the reset re-opened a row it did not change |
| `close` below ~`4e-14` | ~`1e-14`         | underflows to 0 at 14 decimal places                         |

Both are dust — tokens at a price the stored scale cannot represent once
multiplied by a sub-$1 rate.

**Triage query.** Swap in the table you just ran. It asks the question that
matters, which is not _how many rows are at zero_ but _did anything with a usable
price end up at zero_:

```sql
SELECT count() AS stranded_with_real_close
FROM prices.price_ohlcv_1d FINAL
WHERE quote_asset_id = 111
  AND timestamp >= toDateTime(1612656000)
  AND close_usd = 0
  AND close > 0.00000000000005
```

- **0** — every stranded row is dust. The shortfall is the false positive. **Do
  not roll back:** those rows had no representable value to lose, and a re-run
  reproduces the result identically, because the cause is the data and not the
  flags.
- **above 0** — the genuine failure this check exists for. Roll that table back
  from its snapshot and stop.

⚠️ **Do not reach for `--pivot-window-s` first.** The stderr block suggests it,
and it is the right suspect when a whole span fails, but it is unrelated to this
case — the window is never what strands a dust row. Run the triage query before
changing any flag.

⚠️ **List the rows before deciding**, so "dust" is something you saw rather than
something you assumed:

```sql
SELECT timestamp, asset_id, source, close, volume_base, volume_quote, close_usd
FROM prices.price_ohlcv_1d FINAL
WHERE quote_asset_id = 111
  AND timestamp >= toDateTime(1612656000)
  AND close_usd = 0
ORDER BY timestamp
```

⚠️ A threshold picked by eye will mislead you here. A first pass at the triage
used `close < 1e-11` and returned 59 — three orders of magnitude above where the
multiplication actually underflows, so it counted dust rows that had priced
perfectly well. Anchor the bound on the arithmetic (`rate × close` rounding to
zero at 14 dp), not on what looks small.

**Worked example — 0182, `price_ohlcv_1d`, 2026-08-18.** 41,573 re-opened, 41,565
recomputed, shortfall 8. All eight had `close` of 0, 1e-14, 2e-14 or 3e-14. Five
were already at `close_usd = 0` before the reset touched them — re-opened without
being changed — and three fell from ~1e-14 to 0. `stranded_with_real_close`
returned 0, the run was correct, and the campaign continued. `_1h` (357,274
reset) and `_4h` (157,858) produced no shortfall at all.

> The check fires on _any_ excess, while the rationale it is built on
> (`ch_enrich.rs:407`) describes `rows_reset` **far exceeding** `rows_enriched`.
> Strictness is the right default for a guard against data destruction, but it
> means 8 rows in 41,573 — 0.02% — produce a stop-everything message. Read the
> shortfall as _"look at these rows"_, not as _"the run failed"_.

Then assert the defect cannot still be present. For the USDT case the fingerprint
is an implied rate of ~1.0:

```sql
SELECT toYYYYMM(timestamp) AS m,
       count()                              AS candles,
       round(avg(close_usd / close), 6)     AS implied_rate
FROM prices.price_ohlcv_1d FINAL
WHERE quote_asset_id = 111
  AND timestamp >= toDateTime(1612656000)
  AND close > 0 AND close_usd > 0
GROUP BY m ORDER BY m
```

`implied_rate` must track USDT's measured market value (~1.00 until 2022-04, then
falling to ~0.13), **not** sit at 1.0 throughout.

### Run it once

Reset mode is **not a fixed point across invocations**: a second run sees the
refilled rows and re-opens them again, recomputing values that are already
correct. It is value-idempotent but it is not free and it bumps `version` each
time. Run it once per table, verify, and move on. The recurring hourly sweep pins
the reset off and can never inherit it.

## Appendix B — re-enrich USDC-quoted candles from the measured rate (task 0268)

The appendix above corrects a value that was wrong because a tier used the wrong
_reference_. This one corrects a value that was wrong because a tier used **no
reference at all**.

### When this applies

Every USDC-quoted candle stamped before **2026-03-11 14:00 UTC** carries
`close_usd = close × $1.00`, written by the peg tier, because until task 0268
nothing in the enrichment could read a measured USDC/USD rate. USDC is not a
dollar: it closed at **0.9681** on 2023-03-11 and traded as low as ~0.88
intraday. Task 0247 measured **654,291** such candles on prod with an implied
rate of exactly 1.0.

This is a wrong value, not a missing one, so — exactly as in Appendix A — the
normal repair cannot see it: every tier filters on `close_usd = 0`.

The difference from Appendix A, and the whole point of the mode: the reset is
scoped to buckets the imported series can actually refill. A bucket with no
imported rate is **not re-opened at all**. See "The dry run is the gate" below
for why that matters more than it sounds.

### Preconditions

All six, in order. None is optional.

**Set the epoch ONCE, first.** Every query below that mentions the oracle epoch
reads it as the client parameter `{epoch:UInt32}`, so the value is typed one
time in this session and nowhere else:

```sql
SET param_epoch = 1773237600   -- prices_clickhouse::USDC_ORACLE_EPOCH_S, 2026-03-11 14:00 UTC
```

(`clickhouse-client` keeps it for the session; over HTTP pass `?param_epoch=`
on each request.) The tool logs the same value at startup as `reset_not_after`;
if the two differ, stop. The literal above is the only hand-typed copy in this
runbook, and a unit test
(`the_runbook_hand_types_the_oracle_epoch_once_and_it_is_the_constant`) pins
it to the constant — two hand-typed epochs are how a precondition ends up
measuring the wrong window and reporting 0 over the exact assumption it exists
to check.

0. **The server is in UTC.** Run `SELECT timezone()` and stop unless it says
   `UTC`. Every day and hour boundary in this repo — the candle tables' unzoned
   `toStartOfInterval`, the views' day buckets, the loader's and the tiers'
   ASOF floors — is computed in the SERVER's timezone, so on a non-UTC server
   imported rows land on the wrong day and every gate below misreads. Task
   0267's loader refuses to write on a non-UTC server; this campaign has no
   such guard in code, so this line IS the guard.

1. **Task 0267's `external` rows are loaded.** A count of **0 is a hard
   refusal**, not a no-op — the tool exits with
   `ResetRequiresExternalRates` and writes nothing.

   The procedure that produces those rows is
   `docs/runbooks/load-external-usdc-rate.md` — run it to completion first.
   ⚠️ It writes in two steps: a shadow load under
   `method = 'external-candidate'`, then a promote to `method = 'external'`.
   **This query counts only the promoted word.** So a count of 0 here alongside
   rows under `external-candidate` does not mean the load failed — it means the
   promote has not run, and the fix is that runbook's step 5, not a re-load.

   ```sql
   SELECT count() AS rows, min(timestamp) AS first, max(timestamp) AS last
   FROM prices.usd_rate FINAL
   WHERE asset_kind = 'credit' AND asset_code = 'USDC'
     AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
     AND contract_address = '' AND method = 'external'
   ```

   `first` must reach back at least as far as the earliest month you intend to
   repair. Rows above that floor are simply not re-opened; rows below it never
   were.

2. **Confirm 0267 stamps its rows at the START of their UTC bucket.** The tier
   resolves the rate at the bucket's END with an ASOF `rts < bend`, so a
   bucket-start stamp gives every candle in a bucket that bucket's rate. A
   bucket-END convention resolves every one to the **previous** bucket's rate —
   an off-by-one error that produces entirely plausible numbers and fails
   nowhere.

   ⚠️ 0267 loads at **two grains** (`--grain daily|hourly`), so the rows are at
   full hours, of which the midnights are a subset. Check the hour, and check
   that the midnights are all still present:

   ```sql
   SELECT countIf(timestamp != toStartOfHour(timestamp, 'UTC')) AS not_full_hour,
          countIf(timestamp  = toStartOfDay(timestamp, 'UTC'))  AS midnights,
          count() AS rows
   FROM prices.usd_rate FINAL
   WHERE asset_kind = 'credit' AND asset_code = 'USDC'
     AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
     AND contract_address = '' AND method = 'external'
   ```

   Expect `not_full_hour = 0`, `rows` equal to precondition 1's count, and
   `midnights` equal to the number of covered days (1 872 on the versioned
   files, one per day of the daily series). Anything else — stop and settle the
   convention with whoever owns 0267 before running.

   Two traps in that one expression, both of which this runbook has stepped in.
   **Not `toTime()`**: it anchors the time-of-day to 1970-01-02, so an
   expectation written against it halts a correct load. And **name the zone**:
   `timestamp` is a bare `DateTime` and nothing pins the server's timezone
   (`docker-compose.yml` sets no `TZ`; `ch-prod-01`'s is undocumented), so an
   unzoned `toStartOfDay`/`toStartOfHour` resolves locally — on a UTC+2 server
   the unzoned day form reports every single row as not-midnight and this gate
   blocks a correct load.

3. **Confirm `oracle_prices` holds no canonical-USDC reading before the epoch.
   BLOCKING.** Two things rest on "no poll priced USDC before
   `USDC_ORACLE_EPOCH_S`": the API's read-time label (a scaled USDC-quoted
   candle below the epoch is reported `method: external`), and the external
   tier's recomputation of `volume_quote_usd` on every pre-epoch candidate
   (its "oracle values win" argument is that no oracle reading exists there
   to win). The epoch was measured on `usd_rate`, but the oracle tier READS
   `prices.oracle_prices`, and `usd_rate`'s oracle rows are copied out of it
   behind a watermark — so the table to ask is `oracle_prices`:

   ```sql
   SELECT count() AS pre_epoch_readings, min(timestamp) AS first
   FROM prices.oracle_prices
   WHERE asset_id = ( SELECT asset_id FROM prices.assets FINAL
                      WHERE asset_code = 'USDC' AND contract_address = ''
                        AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN' )
     AND oracle_name = 'reflector'
     AND timestamp < toDateTime({epoch:UInt32})
   ```

   Must be **0**. A non-zero count is a STOP for this mode: the tool refuses
   the run with `ResetBlockedByPreEpochOracleRows` (it runs this exact count,
   independent of `--reset-not-before`), and do not work around it — triage
   the rows first (purge a mis-attribution as task 0196 did, or move the
   epoch, which is a code change with its own test). Zero here also settles
   the `usd_rate` premise the label arm rests on, since every `usd_rate`
   oracle row originates in this table.

4. **The cleanup worker stays DARK.** Its EventBridge rule is disabled on
   purpose — it shredded the 0182/0201 repair campaign. Confirm it is still
   disabled before starting, exactly as Appendix A requires.

5. **FREEZE snapshots exist and were verified.** Same rule, same reason, same
   `--snapshots-verified` assertion as Appendix A. A reset with no rollback
   point is not a repair.

### The granularities that actually hold deep history

`price_ohlcv_1h`, `_4h`, `_1d`, `_1w`, `_1M`. Do not attempt the others:

- `_15m` has a 30-day retention, so it holds no 2023 rows to repair;
- `_1m` was largely dropped by the cleanup worker for 2025-02 → 2026-02, and
  `--table price_ohlcv_1m` is refused by the tool outright — it is the live base
  table the scheduled Lambda owns.

For `_1w` and `_1M`, end the campaign at `--end-month 202602`, not `202603`:
the bucket that STARTS in early March 2026 straddles the epoch (its start is
below it, so it is eligible; ten of its days are above it, where the oracle
priced things). Inspect that one bucket by hand if it matters — task file,
Issues 8.

### The flags

```bash
--reset-quote-asset-id <USDC_ID>    # canonical USDC's asset_id on prod
--reset-not-before 0                # all of deep history
--reset-require-external-rate       # the 0268 mode
```

There is a fourth flag, `--reset-not-after`, and it is deliberately NOT in the
block above: it defaults to `prices_clickhouse::USDC_ORACLE_EPOCH_S`
whenever `--reset-require-external-rate` is passed — the **same constant** the
API's `external` label arm keys on and the value you set as `param_epoch`
above. The tool logs the resolved value at startup (`reset_not_after`,
`defaulted = true`). Pass it explicitly only if you mean something else; two
hand-typed epochs are how the wire label and the reset window drift apart with
nothing failing loudly.

The tool refuses a window that can match nothing: `--reset-not-before` at or
above `--reset-not-after` (a mistyped year, the epoch pasted into the wrong
flag) exits with `ResetWindowEmpty` before a connection is opened, dry run or
not. Without that refusal the run would report a clean, empty repair.

`--reset-require-external-rate` narrows the candidate set to
`close_usd = close` (the peg tier's exact signature) **on the days the imported
series covers**. Both halves matter: the first keeps oracle- and external-priced
candles out, the second is task 0182's lesson as a predicate.

Dry run first, one table at a time:

```bash
./coarse-repair --table price_ohlcv_1d \
  --start-month 202001 --end-month 202603 \
  --reset-quote-asset-id <USDC_ID> --reset-not-before 0 \
  --reset-require-external-rate \
  --dry-run
```

Then the real run, dropping `--dry-run` and adding `--skip-snapshot
--snapshots-verified` per Appendix A's rules. Repeat for `_1h`, `_4h`, `_1w`,
`_1M`.

### ⚠️ The dry run is the gate — and zero candidates is a STOP

**A dry run reporting ZERO candidate months is not an all-clear.** That exact
false green is how task 0182 stayed invisible for a month: the driver enumerated
one predicate while the statement acted on another, so a table with 44,657 wrong
values reported "no months with enrichable zeros" and looked identical to a
clean one.

Expected order of magnitude, so you can tell a real result from a silent
mismatch: task 0247 measured **654,291** pre-oracle USDC-quoted candles at an
implied rate of exactly 1.0 across all granularities, and 0182's comparable
campaign touched **567,232** rows in about **4 hours**. If a dry run over
2020-2026 reports zero months, or a few dozen rows, something is wrong with the
predicate or with precondition 1 — do not proceed.

### The baseline (before)

Per table, the population about to change:

```sql
SELECT count() AS pegged
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN ( SELECT asset_id FROM prices.assets FINAL
             WHERE asset_code = 'USDC' AND contract_address = ''
               AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
           ) AS u ON u.asset_id = p.quote_asset_id
WHERE p.close_usd = p.close AND p.close_usd > 0
  AND p.timestamp < toDateTime({epoch:UInt32})
```

And the falsifier's own row, per granularity, before the run:

```sql
SELECT toFloat64(close_usd) / toFloat64(close) AS implied_rate
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN ( SELECT asset_id FROM prices.assets FINAL
             WHERE asset_code = 'XLM' AND issuer_address = ''
               AND contract_address = '' ) AS x ON x.asset_id = p.asset_id
INNER JOIN ( SELECT asset_id FROM prices.assets FINAL
             WHERE asset_code = 'USDC' AND contract_address = ''
               AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
           ) AS u ON u.asset_id = p.quote_asset_id
WHERE p.timestamp >= toDateTime(1678492800) AND p.timestamp < toDateTime(1678579200)
```

It must read **exactly 1.0** before the run. On `_1h`/`_4h`/`_1d` the
after-check expects it to have moved to ~0.9681.

On `_1w` and `_1M` it will read ~1.0 AFTER the run too — the tier prices a
bucket from ONE rate resolved at the bucket's END (decision G), and the week
containing 03-11 ends on 03-13, the month on 04-01, when USDC was back at par.
The depeg is invisible at those grains by construction; `_1d` is the coarsest
grain that can carry it. So for those two the after-check verifies the
MECHANISM instead, and needs the bucket's `max(version)` from before the run.
**Record both numbers** — the after-check takes them as input and refuses to
pass without them:

```sql
SELECT max(version) AS version_before_1w
FROM prices.price_ohlcv_1w AS p FINAL
INNER JOIN ( SELECT asset_id FROM prices.assets FINAL
             WHERE asset_code = 'XLM' AND issuer_address = ''
               AND contract_address = '' ) AS x ON x.asset_id = p.asset_id
INNER JOIN ( SELECT asset_id FROM prices.assets FINAL
             WHERE asset_code = 'USDC' AND contract_address = ''
               AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
           ) AS u ON u.asset_id = p.quote_asset_id
WHERE p.timestamp = toDateTime(1678060800);   -- 2023-03-06, the week of the depeg

SELECT max(version) AS version_before_1M
FROM prices.price_ohlcv_1M AS p FINAL
INNER JOIN ( SELECT asset_id FROM prices.assets FINAL
             WHERE asset_code = 'XLM' AND issuer_address = ''
               AND contract_address = '' ) AS x ON x.asset_id = p.asset_id
INNER JOIN ( SELECT asset_id FROM prices.assets FINAL
             WHERE asset_code = 'USDC' AND contract_address = ''
               AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
           ) AS u ON u.asset_id = p.quote_asset_id
WHERE p.timestamp = toDateTime(1677628800);   -- 2023-03-01, the month of the depeg
```

### The abort signal

`rows_reset` far exceeding `rows_enriched` means values were discarded and not
recomputed — the one outcome worse than the defect. The tool prints this itself
and tells you to stop. **Do not continue to the next table.** Roll the current
one back from its FREEZE snapshot and work out why the reset re-opened rows the
external tier could not refill: under this mode the predicates are shared, so a
large shortfall points at precondition 2 (a day-end stamping convention) rather
than at the reset.

### After — the falsifier

`native` on 2023-03-11 must now read **~3% below** its USDC-denominated close
on every grain whose bucket ENDS inside the depeg — `_1h`, `_4h`, `_1d`. As SQL,
per table, it is the baseline query above: the implied rate must have moved
from `1.0` to **0.9681** (exactly, against a daily series; lower if 0267 ships
hourly rows for the stress days). The test holds those grains UNDER `0.99` — a
ceiling par cannot satisfy and a PARTIAL pass (a few bps under par) cannot
either — rather than inside a band around 0.9681, because a band wide enough
to be safe contains 1.0.

`_1w` and `_1M` are judged by the MECHANISM, not the rate (see "The baseline"
above for why the rate cannot show anything there): the bucket's `max(version)`
must exceed the value you recorded before the run, no row with volume may sit at
`close_usd = 0`, and the par signature `close_usd = close` may survive only if
the imported series' own rate at the bucket end is exactly 1.0 — in which case
the stored value is right and `/ohlcv` will nonetheless label it `assumed-par`
(task file, Issues 9; a known ambiguity of the read-time label, not a repair
defect). Hand the two recorded versions to the test as environment variables.

On every grain the test also counts rows at `close_usd = 0` **with volume** over
the same window: any such row is the 0182 outcome (reset, never refilled) and
fails the check outright. Rows at zero WITHOUT volume are the permanent
volume-zero floor no tier prices; they are reported as context, never as a
failure.

As a test, which also checks the control date:

```bash
CLICKHOUSE_URL=... CH_DATABASE=prices \
POST_RUN_0268_VERSION_BEFORE_1W=<version_before_1w> \
POST_RUN_0268_VERSION_BEFORE_1M=<version_before_1M> \
  cargo test -p enrichment-worker --test post_run_0268_it -- --ignored
```

Both tests are expected to FAIL before the pass and pass after it. The second
one (`usdc_is_back_at_par_a_few_days_later`) exists so the first cannot be
satisfied by a table priced uniformly low.

Then walk `/ohlcv` for a non-USDC asset over the repaired span and confirm the
`method` field reads `external` on the scaled pre-epoch buckets and
`assumed-par` on any bucket the series did not cover. Requesting
`USDC:<issuer>` itself still returns `peg` — that is USDC's own series and is
deliberately unchanged.

### Rollback

Identical to Appendix A: `ALTER TABLE … ATTACH PARTITION … FROM …` out of the
frozen copies, per month. The reset is a versioned INSERT, never a mutation, so
the pre-reset rows are still on disk under their old version.

### Run it once

Reset mode is **not a fixed point across invocations** — the same warning as
Appendix A, with one addition specific to this mode: a second run re-opens the
already-corrected rows and rewrites them from the same imported series, so the
values do not change but `version` climbs and the FREEZE rollback point becomes
less useful with every pass. Run it once per table, verify, move on.

## Notes

- The repair reuses the exact enrichment tiers (`ch_enrich.rs`): USDC → ×$1,
  XLM and USDT → ×(that asset's own USDC market), exotic → left zero by design.
  ⚠️ USDT moved from the peg tier to the pivot tier in task 0172; older revisions
  of this runbook said "USDC/USDT → ×$1", which is the defect, not the design.
- The pivot tier computes its XLM/USDC reference from the **same coarse table**,
  forward-filling from earlier months, so a month's first buckets keep a valid
  anchor even when bounded to one partition.
- All figures are `FINAL`-collapsed reads; do not compare without `FINAL`.
