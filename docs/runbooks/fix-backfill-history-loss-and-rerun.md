# Runbook — Fix the backfill history-loss and re-run it correctly

**Audience:** any operator, no prior project knowledge assumed. Follow the steps
top to bottom, run each command, and check the **✅ Confirm** line under it before
moving on. If a Confirm check fails, stop and see **Troubleshooting** at the end.

> **Get sign-off first.** Several steps change a **shared production ClickHouse
> cluster** (used by another team) and disable a scheduled job. Before starting,
> tell the prices-API / data-platform owner you are running this runbook. The
> heavy step (5) runs large queries on the shared cluster — prefer a low-traffic
> window.

---

## 1. What is wrong and what this runbook does

The historical price backfill writes 1-minute candles into the table
`prices.price_ohlcv_1m`. That table is a **temporary 7-day feeder**, not permanent
storage. Permanent history is supposed to live in the coarser tables
(`price_ohlcv_1h`, `_1d`, …), which are filled by a one-off "pre-roll" step.

Right now that pre-roll step is never run, and a nightly cleanup job deletes the
old 1-minute data. So **everything the backfill writes older than ~2 years is
deleted before anything permanent captures it**, and the permanent tables the
downstream consumer reads are empty.

This runbook fixes it by running the pieces **in the correct order**:

```
stop backfill → disable nightly cleanup → reset backfill progress →
re-run backfill (fills 1m) → pre-roll (fills permanent tables) →
verify → re-enable nightly cleanup
```

**Heads-up on time:** the re-run (step 4) re-downloads the whole ledger history
and can take **several days** on a home/office connection. That is expected.

---

## 2. Prerequisites (get these before you start)

| #   | You need                                                                                                        | How to check                                                                                                                                              |
| --- | --------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Access to the backfill machine **`fishuser-hero`** (where the run + repo + certs live)                          | `ssh fishuser-hero` connects                                                                                                                              |
| 2   | The repo checked out on it at `~/stellar-prices-api`                                                            | `ls ~/stellar-prices-api/packages/prices-clickhouse/schema/preroll.sql`                                                                                   |
| 3   | The write certs at `~/prices-mtls/` (`prices_writer.crt`, `.key`, `ca.crt`)                                     | `ls ~/prices-mtls/`                                                                                                                                       |
| 4   | SSH access to the ClickHouse host                                                                               | `ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 'echo ok'`                                                                                       |
| 5   | AWS access to the prices account able to toggle an EventBridge rule (`events:DisableRule`, `events:EnableRule`) | `aws events describe-rule --profile soroban-explorer --region eu-central-1 --name prices-production-cleanup --query State` prints `ENABLED` or `DISABLED` |

If **#5** fails with an access/permission error, you cannot toggle the cleanup job
yourself — ask the prices-API owner to run steps **3.1** and **7** for you, and do
the rest.

**Run everything below from a shell on `fishuser-hero`** (unless a command already
starts with `ssh …`).

### Reference values (used throughout)

| Thing                        | Value                                                                           |
| ---------------------------- | ------------------------------------------------------------------------------- |
| ClickHouse host              | `ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161`                       |
| Run a query                  | `docker exec -i app-clickhouse-1 clickhouse-client --query='…'`                 |
| Nightly cleanup rule         | `prices-production-cleanup` (region `eu-central-1`, profile `soroban-explorer`) |
| Soroban activation ledger    | `50457424`                                                                      |
| Backfill top (do not exceed) | `63352611`                                                                      |
| Repo pre-roll SQL            | `~/stellar-prices-api/packages/prices-clickhouse/schema/preroll.sql`            |

For convenience, set a shortcut you can paste into any step:

```bash
CH() { ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client $*"; }
```

(Re-define this in each new shell you open.)

### Step 0 — Disk-headroom precheck (mandatory, before you disable cleanup)

Because Step 2 disables the nightly cleanup, the **entire** 1-minute history
accumulates in `prices.price_ohlcv_1m` during the multi-day re-run before Step 5
pre-rolls it away. Measured, this is small (~5–9 GB compressed at ~33 B/row), but
this is a **shared** cluster — confirm the host has room before disabling anything.

```bash
CH() { ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client $*"; }

# projected full-history size = bytes_per_row × ~150M rows (mid estimate; upper bound ~267M)
CH --query="SELECT formatReadableSize(sum(data_compressed_bytes)) AS current_on_disk, sum(rows) AS current_rows, round(sum(data_compressed_bytes)/sum(rows),1) AS bytes_per_row, formatReadableSize(round(sum(data_compressed_bytes)/sum(rows)) * 267000000) AS projected_full_upper_bound FROM system.parts WHERE database='prices' AND table='price_ohlcv_1m' AND active"

# free disk on the ClickHouse host
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 "df -h /var/lib/docker"
```

**✅ Confirm — GO / NO-GO:** the `df -h` **`Avail`** column is **≥ 20 GB** (comfortably
above the ~15 GB transient peak, i.e. the projected full history + rollup tables +
merge overhead). Compare it against `projected_full_upper_bound` from the first query.

- **Avail ≥ 20 GB** → proceed. Keep this runbook as written.
- **Avail < 20 GB** → **STOP.** Do not disable cleanup. Escalate to the prices-API /
  cluster owner: either free space, or use the bounded per-chunk pre-roll approach
  (roll each chunk up before the next, so `1m` never holds more than one chunk).

---

## 3. Step 1 — Stop the backfill that is currently running

```bash
pkill -f run-full-backfill.sh 2>/dev/null; pkill -f sdex-backfill 2>/dev/null
sleep 3
pgrep -af sdex-backfill
```

**✅ Confirm:** the last command prints **nothing** (no backfill process running).

---

## 4. Step 2 — Disable the nightly cleanup job

This stops the job that deletes old 1-minute data, so the re-run's history
survives long enough to be pre-rolled.

```bash
aws events disable-rule --profile soroban-explorer --region eu-central-1 \
  --name prices-production-cleanup

aws events describe-rule --profile soroban-explorer --region eu-central-1 \
  --name prices-production-cleanup --query State --output text
```

**✅ Confirm:** the second command prints **`DISABLED`**.

> If you get a permissions error, ask the prices-API owner to disable the rule
> `prices-production-cleanup` and confirm it reads `DISABLED`, then continue.

---

## 5. Step 3 — Reset the backfill progress

The backfill skips ledgers it thinks are already done. Because their data was
deleted, we must clear that record so the re-run reprocesses everything. This
table is backfill-only bookkeeping; clearing it does not affect live data.

```bash
CH() { ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client $*"; }

CH --query='TRUNCATE TABLE prices.backfill_sdex_ledgers'
CH --query='SELECT count() FROM prices.backfill_sdex_ledgers'
```

**✅ Confirm:** the count prints **`0`**.

---

## 6. Step 4 — Re-run the full backfill

This re-downloads and re-writes the whole history into `price_ohlcv_1m`. With the
cleanup job disabled (step 2), it will now persist.

### 6.1 Create the run script

```bash
cat > ~/run-full-backfill.sh <<'SCRIPT'
#!/usr/bin/env bash
set -uo pipefail

export CH_DOMAIN=ch.sorobanscan.rumblefish.dev
export MTLS_CERT_PATH=$HOME/prices-mtls/prices_writer.crt
export MTLS_KEY_PATH=$HOME/prices-mtls/prices_writer.key
export MTLS_CA_PATH=$HOME/prices-mtls/ca.crt

BIN=$HOME/stellar-prices-api/target/release/sdex-backfill
LOG=$HOME/backfill.log
CHUNK=320000
ACTIVATION=50457424      # Soroban activation — combined mode starts here
FLOOR=63352611           # top of the backfill (live ingestion owns 63352612+)
TAIL_END=50457423        # pre-Soroban tail top (activation - 1)

tip() { curl -s 'https://horizon.stellar.org/' \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["core_latest_ledger"])'; }

run_chunk() {  # mode start end — retries the SAME range on failure (idempotent)
  local mode=$1 start=$2 end=$3 t
  while true; do
    t=$(tip)
    echo ">>> [$mode] $start..$end tip=$t $(date -u)" | tee -a "$LOG"
    "$BIN" --mode "$mode" --start "$start" --end "$end" --tip "$t" \
      --transport hetzner --verbose 2>&1 | tee -a "$LOG"
    [ "${PIPESTATUS[0]}" -eq 0 ] && return 0
    echo ">>> [$mode] $start..$end FAILED — retry in 60s" | tee -a "$LOG"
    sleep 60
  done
}

echo "=== FULL BACKFILL START $(date -u) ===" | tee -a "$LOG"
# Phase 1: combined (SDEX + AMM + oracle), activation forward to the floor
START=$ACTIVATION
while [ "$START" -le "$FLOOR" ]; do
  END=$(( START + CHUNK - 1 )); [ "$END" -gt "$FLOOR" ] && END=$FLOOR
  run_chunk combined "$START" "$END"
  START=$(( END + 1 ))
done
echo "=== PHASE 1 COMPLETE $(date -u) ===" | tee -a "$LOG"
# Phase 2: pre-Soroban tail, SDEX only, one resumable run
run_chunk sdex-only 1 "$TAIL_END"
echo "=== FULL BACKFILL COMPLETE $(date -u) ===" | tee -a "$LOG"
SCRIPT
chmod +x ~/run-full-backfill.sh
```

### 6.2 Make sure the binary exists (build if missing)

```bash
ls ~/stellar-prices-api/target/release/sdex-backfill 2>/dev/null \
  || ( cd ~/stellar-prices-api && cargo build --release -p sdex-backfill --features aws-mtls )
```

**✅ Confirm:** `~/stellar-prices-api/target/release/sdex-backfill` exists.

### 6.3 Make sure the machine won't sleep during the multi-day run

```bash
sudo systemctl mask sleep.target suspend.target hibernate.target hybrid-sleep.target
```

(Undo later with `sudo systemctl unmask …` the same targets.)

### 6.4 Start it inside tmux (survives disconnects)

```bash
tmux new -s backfill        # detach later: Ctrl-b then d   |   reattach: tmux attach -t backfill
~/run-full-backfill.sh
```

A healthy start prints `pre-flight: all checks passed` and `backfill starting …
to_process: 5`. One startup `WARN` about "combined mode starts after activation"
is **normal**. Once it is downloading, detach with **Ctrl-b then d**.

### 6.5 Wait for it to finish

Check progress any time (re-define `CH` if it's a new shell):

```bash
# has it finished?
grep -c 'FULL BACKFILL COMPLETE' ~/backfill.log        # 0 = still running, 1 = done
# how far along (climbs toward 63352611, then the tail runs):
CH --query='SELECT task_name, current_ledger, status FROM prices.backfill_progress FINAL FORMAT PrettyCompact'
```

**✅ Confirm before moving on:** `grep -c 'FULL BACKFILL COMPLETE' ~/backfill.log`
prints **`1`**. Do **not** proceed to step 5 until it does.

---

## 7. Step 5 — Pre-roll into the permanent tables

Now that `price_ohlcv_1m` holds the full history, roll it up into the permanent
coarse tables. We clear those tables first so the result is clean and repeatable.

> This runs **large aggregation queries on the shared cluster** and can take a
> while. The command below tells ClickHouse to spill to disk instead of using too
> much memory, and not to time out. Idempotent — safe to re-run if interrupted.

```bash
# 7.1 clear the permanent + intermediate rollup tables
for T in 15m 1h 4h 1d 1w 1M; do
  CH --query="TRUNCATE TABLE prices.price_ohlcv_$T"
done

# 7.2 apply the pre-roll (all 6 steps, in order) from the repo file
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --multiquery \
     --receive_timeout=86400 --max_bytes_before_external_group_by=4000000000" \
  < ~/stellar-prices-api/packages/prices-clickhouse/schema/preroll.sql \
  && echo "PREROLL OK"
```

**✅ Confirm:** the command prints **`PREROLL OK`** with no ClickHouse error above it.
(If it errors partway, fix the cause and just run **7.2** again — it is idempotent.)

---

## 8. Step 6 — Verify the fix

Check that the permanent tables now hold history going back to the start of the
chain (2015-era for SDEX), not just the last ~2 years.

```bash
for T in price_ohlcv_1h price_ohlcv_1d; do
  echo "== $T =="
  CH --query="SELECT source, min(timestamp) AS oldest, max(timestamp) AS newest, count() \
              FROM prices.$T GROUP BY source ORDER BY source FORMAT PrettyCompact"
done
```

**✅ Confirm all of these:**

- `price_ohlcv_1h` and `price_ohlcv_1d` each return **rows** (not empty).
- For `source = sdex`, `oldest` is back in **2015–2016** (the tail was backfilled),
  not mid-2024.
- `count()` is a large number for `sdex`.

If any of these fail, **do not continue to step 7** (re-enabling cleanup would
delete un-rolled 1-minute data). See Troubleshooting.

---

## 9. Step 7 — Re-enable the nightly cleanup

Only after step 6 passes. This lets the cleanup job resume; it will drop the
now-redundant old 1-minute partitions, while the permanent tables keep the history.

```bash
aws events enable-rule --profile soroban-explorer --region eu-central-1 \
  --name prices-production-cleanup

aws events describe-rule --profile soroban-explorer --region eu-central-1 \
  --name prices-production-cleanup --query State --output text
```

**✅ Confirm:** prints **`ENABLED`**.

Then undo the sleep change from 6.3 (optional, tidiness):

```bash
sudo systemctl unmask sleep.target suspend.target hibernate.target hybrid-sleep.target
```

---

## 10. Final confirmation checklist

Tick every box before declaring the fix complete:

- [ ] Step 0 — free disk on the ClickHouse host was `≥ 20 GB` before disabling cleanup.
- [ ] Step 1 — no `sdex-backfill` process was running before the re-run.
- [ ] Step 2 — cleanup rule read `DISABLED` before the re-run.
- [ ] Step 3 — `backfill_sdex_ledgers` count was `0`.
- [ ] Step 4 — `backfill.log` contains `=== FULL BACKFILL COMPLETE ===`.
- [ ] Step 5 — pre-roll printed `PREROLL OK`.
- [ ] Step 6 — `price_ohlcv_1h` **and** `price_ohlcv_1d` have `sdex` rows going back to ~2015.
- [ ] Step 7 — cleanup rule reads `ENABLED` again.

When all seven are ticked, the backfill history is permanently stored and the
system is back to its normal retention behaviour.

---

## 11. Troubleshooting

- **Step 4 says `pre-flight` failed / no AWS:** the ledger download needs no
  credentials; ensure the machine has internet and `aws`, `jq`, `python3` installed.
- **The run keeps retrying one chunk:** transient network/ClickHouse blips are
  retried automatically (see `~/backfill.log`). It resumes; nothing is lost.
- **Step 5 pre-roll errored midway:** it is safe to re-run **7.2** as-is
  (ReplacingMergeTree collapses duplicates). If it fails on memory, ask the
  cluster owner to run it, or lower `--max_bytes_before_external_group_by`.
- **Step 6 shows coarse tables still empty or only recent:** the pre-roll did not
  complete — re-run **7.2** and re-check. Do **not** re-enable cleanup (step 7)
  until step 6 passes.
- **Something went wrong and you must stop safely:** the only thing you must not
  leave in a bad state is the cleanup rule. If the permanent tables are **not**
  yet populated, leave the rule **DISABLED** (so no history is deleted) and
  escalate. If they **are** populated (step 6 passed), the rule can be `ENABLED`.

## 12. Background (for reviewers)

Full root-cause analysis, evidence, and the design rationale are in lore task
**0090** (`lore/1-tasks/backlog/0090_FEATURE_backfill-preroll-and-cleanup-coordination.md`).
The rollup SQL is `packages/prices-clickhouse/schema/preroll.sql` (historical,
full-range) vs `schema/rollups.sql` (live, 2-hour window). Retention lives in
`packages/cleanup-worker/` (rule `prices-{env}-cleanup`). A diagnostic that proves
the extractor itself is correct is `packages/prices-ingest-core/examples/decode_probe.rs`.
