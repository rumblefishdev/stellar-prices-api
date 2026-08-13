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

## Step 6 — the recurring guard (automated: folded into the enrichment Lambda)

The one-off historical repair (Steps 0–7) and the ongoing guard are the same job.
The guard is **not** a separate cron or a separate Lambda — it is folded into the
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

- Runs **after** the 1m pass and is **best-effort**: any sweep failure is logged
  and swallowed, so it can never fail the invocation or delay/regress the 1m pass.
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

> **Emergency off switch (no deploy):** in the AWS console set the enrichment
> Lambda's `COARSE_SWEEP_TABLES` env var to empty. The next hourly run skips the
> sweep; the 1m pass is unaffected. A redeploy restores the CDK value.

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
  `> 0`** — the dead-sweep signal. (The enrichment `-errors` alarm will **not**
  catch a sweep failure: the sweep is best-effort and never fails the invocation.)
  Config skips are deliberately excluded so a benign typo can't false-fire it.
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

## Appendix — reset mode, for a _wrong_ value rather than a missing one (task 0182)

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

| Refusal                                           | Why                                                                                                                                                                            |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `--skip-snapshot` together with `--reset-*`       | Rollback for a bad reset **is** `ATTACH PARTITION` from the frozen copy. Step 3b's operator-taken snapshots are still the path — just drop `--skip-snapshot` from the command. |
| `--pivot-window-s` below the table's bucket width | On `_1w`/`_1M`/`_1d` a bucket whose reference is the previous bucket falls outside a short window. Before a reset that left a row unenriched; now it discards the value first. |
| A quote leg that is not a peg or pivot reference  | A mistyped id (`11` for `111`) passes the oracle check, because an unknown asset has no oracle rows either.                                                                    |
| A bounded pass (`one_shot = false`)               | The peg-pivot tier is gated on the oracle tier draining, so a bounded pass can defer the only tier that refills.                                                               |
| `oracle_prices` rows for the quote leg            | See below.                                                                                                                                                                     |

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
recompute. **Stop; do not continue to the next table.** Roll that table back from
its snapshot (Rollback section below), then check `--pivot-window-s` against the
table's bucket width.

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

## Notes

- The repair reuses the exact enrichment tiers (`ch_enrich.rs`): USDC → ×$1,
  XLM and USDT → ×(that asset's own USDC market), exotic → left zero by design.
  ⚠️ USDT moved from the peg tier to the pivot tier in task 0172; older revisions
  of this runbook said "USDC/USDT → ×$1", which is the defect, not the design.
- The pivot tier computes its XLM/USDC reference from the **same coarse table**,
  forward-filling from earlier months, so a month's first buckets keep a valid
  anchor even when bounded to one partition.
- All figures are `FINAL`-collapsed reads; do not compare without `FINAL`.
