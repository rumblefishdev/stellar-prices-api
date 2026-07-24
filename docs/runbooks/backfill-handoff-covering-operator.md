# SDEX Historical Backfill — Handoff (task 0088)

**Prepared:** 2026-07-24 · **For:** covering operator while the primary operator is away · **Escalate to:** the primary operator

This document is written for someone **new to the project**. Read the _Background_
once, then work top-to-bottom. Every step is tagged with the machine it runs on:

- **[LAPTOP]** — your own laptop. Reaches ClickHouse over the internet (public host).
- **[FISHUSER-HERO]** — the backfill machine on the primary operator's LAN (`192.168.1.106`). See _Access_ below — you may not be able to reach it from another network.

> ⚠️ **The single most important rule:** the nightly cleanup job
> `prices-production-cleanup` **must stay DISABLED** until the very last step. If it
> turns on while the backfill is running, it _deletes the backfill's output as fast
> as it is written_ and re-creates the exact history gap we are trying to close.
> Re-check it before every phase. Never enable it yourself except at Phase 5.

---

## 1. Background (plain language)

We are rebuilding the **historical price history** for the Stellar DEX ("SDEX")
by replaying the blockchain from genesis and writing 1-minute price candles into a
production ClickHouse database on Hetzner.

There are **two passes**:

- **Pass 1 (running now):** walks _upward_ from ledger 1 toward the Soroban
  activation point (ledger `50,457,423`). This is the long multi-day download.
- **Pass 2 (recovery, not started):** re-walks the bottom slice `[1, 23,423,999]`.
  An earlier run wrote that slice, but the cleanup job (which was accidentally on)
  deleted it. So after pass 1 finishes we must re-run just the bottom to fill the
  2015 → 2018 hole.

After both passes, a one-time **pre-roll** copies the 1-minute data up into the
permanent "coarse" tables (hourly/daily/…). Only _then_ is cleanup re-enabled.

**The full sequence you are shepherding:**

```
Pass 1 finishes  →  clear markers  →  Pass 2  →  pre-roll  →  re-enable cleanup
```

There is also a dead `soroban_amm` leg you will see in status output. **Ignore it** —
it is a separate recovery the primary operator handles; it is not part of this handoff.

---

## 2. Current status (as of 2026-07-24 ~11:48 UTC)

| Metric                                          | Value                                                   |
| ----------------------------------------------- | ------------------------------------------------------- |
| **Pass 1 frontier** (highest ledger backfilled) | **38,719,999**                                          |
| **Markers contiguous, no gaps**                 | ✅ yes (`3 → 38,719,999`)                               |
| **Remaining to activation**                     | **11,737,424** ledgers                                  |
| **Measured rate**                               | **~145,000 ledgers/hr** (download-bound; slowly easing) |
| **Last write**                                  | ~1 h ago → **run is alive and healthy**                 |
| **Cleanup rule**                                | ✅ **DISABLED** (correct)                               |
| `soroban_amm` leg                               | dead / stale 235 h — **ignore, not your job**           |

### Estimated finish of Pass 1

At ~145k ledgers/hr: `11,737,424 ÷ 145,000 ≈ 81 h ≈ 3.4 days`.

> **ETA: ~2026-07-27 evening → 2026-07-28 (UTC).**
> The rate keeps easing slightly as partitions grow, so treat 07-28 as the realistic
> target and **recompute from live samples** (§4.2). Do not act on the date alone —
> act on the frontier reaching activation.

---

## 3. Access prerequisites (sort out BEFORE pass 1 finishes)

You need three things. **Confirm all three with the primary operator now** — you don't
want to discover a gap the moment pass 1 completes.

| #   | Thing                                                  | Used for                | How to verify                                                                                                                                  |
| --- | ------------------------------------------------------ | ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | SSH key `~/.ssh/sorban-prod_ed25519`                   | Talk to prod ClickHouse | `ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 'echo ok'` prints `ok`                                                                |
| 2   | AWS profile `soroban-explorer` (region `eu-central-1`) | Toggle the cleanup rule | `aws events describe-rule --name prices-production-cleanup --region eu-central-1 --profile soroban-explorer --query State` prints `"DISABLED"` |
| 3   | **Access to `fishuser-hero` (`192.168.1.106`)**        | **Launch pass 2**       | `ssh fishuser-hero 'echo ok'` prints `ok`                                                                                                      |

> 🚧 **Network caveat — read this.** `fishuser-hero` is on a **private LAN**.
> From a different network you **cannot reach it** without being on the same LAN or
> on a VPN/Tailscale into it. **Pass 2 can only be _launched_ on fishuser-hero.**
> Everything else (monitoring, clearing markers, pre-roll, cleanup toggle) works
> from your laptop against the public CH host. **If you can't get onto fishuser-hero,
> tell the primary operator before pass 1 ends — pass 2 will otherwise be blocked.**

**Never** read, print, or copy any private key or secret into chat, tickets, or this
doc. Reference them by name only.

### Handy shell shortcut (define in every new [LAPTOP] terminal)

```bash
CHQ() { ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client"; }
```

Then you can pipe SQL to it with a heredoc (`CHQ <<'SQL' … SQL`).

---

## 4. Phase 1 — Monitor Pass 1 until it finishes

Do this **once or twice a day**. Nothing to change; you are watching for two things:
progress climbing, and the cleanup rule staying off.

### 4.1 [LAPTOP] Progress + health check

```bash
CHQ <<'SQL'
SELECT
  max(sequence)                       AS frontier,
  count()                             AS markers,
  count() = (max(sequence) - min(sequence) + 1) AS contiguous_no_gaps,
  50457423 - max(sequence)            AS remaining_to_activation
FROM prices.backfill_sdex_ledgers
WHERE sequence < 50457424;
SQL
```

> The `WHERE sequence < 50457424` filter is **mandatory**. Without it the query
> returns the _other_ (completed) run's ceiling `63,352,611` and gives a nonsense
> reading over 100%.

**✅ Checkpoints:**

- `frontier` is **higher** than last time you checked (and higher than 38,719,999).
- `contiguous_no_gaps` = **1**.
- `remaining_to_activation` is **shrinking**.

### 4.2 [LAPTOP] Rate + ETA (optional, do it if you want a fresh estimate)

Run §4.1 twice, **at least 25 minutes apart** (the run is download-bound; samples
closer than ~22 min can look frozen even when healthy — this is normal, not a stall).

```
rate  = (frontier_2 − frontier_1) / hours_between      # expect ~130k–150k/hr
ETA   = remaining_to_activation / rate  (hours from now)
```

### 4.3 [LAPTOP] Cleanup rule must stay DISABLED

```bash
aws events describe-rule --name prices-production-cleanup --region eu-central-1 \
  --profile soroban-explorer --query State --output text
```

**✅ Checkpoint:** prints **`DISABLED`**. If it EVER prints `ENABLED`, **stop and
call the primary operator immediately** — do not try to fix data, just get it
disabled again:

```bash
aws events disable-rule --name prices-production-cleanup --region eu-central-1 \
  --profile soroban-explorer
```

### 4.4 [FISHUSER-HERO] Liveness (only if you can reach the LAN — optional)

```bash
ssh fishuser-hero 'pgrep -af sdex-backfill; echo "---"; tail -n 15 ~/sdex-tail.log'
```

**✅ Checkpoint:** a `sdex-backfill …` process is listed, and the log's last lines
are recent (partition numbers climbing). If §4.1 shows progress, the run is fine
even if you can't reach fishuser-hero.

---

## 5. Phase 2 — Detect that Pass 1 is done

Pass 1 is **done** when the frontier reaches activation. Run this query — it prints
a plain-English verdict in the `status` column so you don't have to eyeball numbers:

```bash
# [LAPTOP]
CHQ <<'SQL'
SELECT
  max(sequence)                                 AS frontier,
  50457423 - max(sequence)                      AS remaining_to_activation,
  count() = (max(sequence) - min(sequence) + 1) AS contiguous_no_gaps,
  multiIf(
    max(sequence) >= 50457423 AND count() = (max(sequence) - min(sequence) + 1),
      'DONE AND READY TO START PHASE 2',
    max(sequence) >= 50457423,
      'AT ACTIVATION BUT GAPS PRESENT — do NOT proceed, escalate',
    'IN PROGRESS'
  )                                             AS status
FROM prices.backfill_sdex_ledgers
WHERE sequence < 50457424;
SQL
```

**✅ Read the `status` column:**

- **`IN PROGRESS`** → pass 1 is still running. Keep monitoring (Phase 1); do not
  start Phase 3.
- **`DONE AND READY TO START PHASE 2`** → the database side is complete. Do the one
  remaining confirmation below, then proceed to Phase 3.
- **`AT ACTIVATION BUT GAPS PRESENT …`** → frontier reached activation but the marker
  span has holes. Do **not** proceed — see §9 _Troubleshooting_ / escalate.

**One more confirmation before Phase 3** (only when `status` says DONE):

- [FISHUSER-HERO, if reachable] `pgrep -af sdex-backfill` prints **nothing**, and
  `~/sdex-tail.log` shows a completion line — i.e. the process has actually exited,
  not just paused.

If the frontier has plateaued **well below** 50,457,423 for hours _and_ no process
is running, the run died early — see §9 _Troubleshooting_ (it is resume-safe).

**Do not start Phase 3 until `status` reads `DONE AND READY TO START PHASE 2`.**

---

## 6. Phase 3 — Pass 2: re-walk the bottom slice `[1, 23,423,999]`

This fills the 2015 → 2018 hole that cleanup deleted.

### 6.1 [LAPTOP] Re-confirm cleanup is DISABLED

Run §4.3. Must be `DISABLED`. If not, stop and fix before continuing.

### 6.2 [LAPTOP] Clear the old markers for the bottom slice — **CRITICAL**

The backfill skips ledgers it thinks are already done. The bottom slice's markers
still exist (the _data_ was deleted, the _markers_ survived). If you don't clear
them, pass 2 will **silently skip the whole span** and you'll fill nothing.

```bash
CHQ <<'SQL'
DELETE FROM prices.backfill_sdex_ledgers WHERE sequence <= 23423999;
SQL
```

This is an async mutation. Wait for it to finish, then verify:

```bash
CHQ <<'SQL'
SELECT count() AS leftover_low_markers
FROM prices.backfill_sdex_ledgers
WHERE sequence <= 23423999;
SQL
```

**✅ Checkpoint:** `leftover_low_markers` = **0**. Do not launch pass 2 until it is 0.

### 6.3 [FISHUSER-HERO] Launch pass 2 in tmux

> Requires being on fishuser-hero (see _Access_, §3). All commands run **on
> fishuser-hero**.

```bash
# 1) certs + CH endpoint (the write path)
export CH_DOMAIN=ch.sorobanscan.rumblefish.dev
export MTLS_CERT_PATH=$HOME/prices-mtls/prices_writer.crt
export MTLS_KEY_PATH=$HOME/prices-mtls/prices_writer.key
export MTLS_CA_PATH=$HOME/prices-mtls/ca.crt

# 2) binary (build it if missing)
BIN=$HOME/stellar-prices-api/target/release/sdex-backfill
ls "$BIN" || ( cd ~/stellar-prices-api && \
  cargo build --release -p sdex-backfill --features aws-mtls )

# 3) live chain tip (upper bound; only moves forward, so any current value is safe)
TIP=$(curl -s 'https://horizon.stellar.org/' \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["core_latest_ledger"])')
echo "tip = $TIP"

# 4) run it inside tmux so it survives disconnects; log OUTSIDE /tmp
tmux new -s sdex-pass2       # detach with:  Ctrl-b  then  d
"$BIN" --mode sdex-only --start 1 --end 23423999 --tip "$TIP" \
  --transport hetzner --verbose 2>&1 | tee -a "$HOME/sdex-pass2.log"
```

A healthy start prints pre-flight checks passing and begins downloading. Detach
with **Ctrl-b then d**. Reattach any time with `tmux attach -t sdex-pass2`.

**✅ Checkpoint (right after launch):** the log shows pre-flight passed and
partition downloads starting — no immediate error/exit.

### 6.4 Monitor pass 2 (same as Phase 1)

Pass 2 rebuilds markers from ledger 1 upward again, so use the **same** query but
watch it climb toward the pass-2 target `23,423,999`:

```bash
# [LAPTOP]
CHQ <<'SQL'
SELECT min(sequence) AS low, max(sequence) AS high, count() AS markers
FROM prices.backfill_sdex_ledgers
WHERE sequence <= 23423999;
SQL
```

**✅ Checkpoint:** `high` climbing toward **23,423,999**; `low` back down near **1–3**.
Expect **~4–5 days** at the same rate. Keep §4.3 (cleanup DISABLED) checked daily
throughout.

**✅ Pass 2 done when:** `high` ≈ 23,423,999 and [FISHUSER-HERO] `pgrep -af
sdex-backfill` prints nothing / the log shows completion.

---

## 7. Phase 4 — Pre-roll the history into the permanent tables

This copies the newly-filled 1-minute data up into the permanent coarse tables.
**Use the INCREMENTAL script** — it _appends_ the pre-Soroban years and leaves the
existing Soroban-era history untouched. **Do NOT use `preroll.sql`** (the full
rebuild) — that would wipe the Soroban-era coarse.

This phase runs from **wherever the repo is checked out** — fishuser-hero has it at
`~/stellar-prices-api`; your laptop works too if you have the repo cloned. The SQL
is piped over SSH to prod CH either way.

### 7.1 Pre-flight

```bash
# [LAPTOP or FISHUSER-HERO] confirm the tail reached activation
CHQ <<'SQL'
SELECT max(sequence) AS frontier FROM prices.backfill_sdex_ledgers WHERE sequence < 50457424;
SQL
# expect ~50,457,423

# [LAPTOP] cleanup STILL disabled (must be)
aws events describe-rule --name prices-production-cleanup --region eu-central-1 \
  --profile soroban-explorer --query State --output text        # expect DISABLED

# get the EXACT activation boundary timestamp (do NOT assume midnight)
CHQ <<'SQL'
SELECT min(timestamp) AS boundary
FROM prices.price_ohlcv_1m
WHERE source IN ('aquarius','phoenix','soroswap');
SQL
```

Copy that `boundary` timestamp exactly — you pass it into the pre-roll.

### 7.2 Run the incremental pre-roll

Run from the directory that contains the repo (has
`packages/prices-clickhouse/schema/preroll-incremental.sql`):

```bash
BOUNDARY="<paste the exact boundary timestamp from 7.1>"

ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client \
     --param_boundary='$BOUNDARY' --multiquery" \
  < packages/prices-clickhouse/schema/preroll-incremental.sql \
  && echo "PREROLL OK"
```

**✅ Checkpoint:** prints **`PREROLL OK`** with no ClickHouse error above it. It is
idempotent — if it errors partway, fix the cause and re-run the same command.

### 7.3 Verify coarse now covers the old years

```bash
CHQ <<'SQL'
SELECT toYear(timestamp) AS yr, count() AS candles
FROM prices.price_ohlcv_1d
WHERE source='sdex' AND timestamp < '2024-02-20'
GROUP BY yr ORDER BY yr;
SQL
```

**✅ Checkpoint:** a **smooth ramp from ~2016 through 2023**, no missing years.
If years are missing or the table is empty, the pre-roll didn't complete — re-run
§7.2. **Do NOT proceed to Phase 5 until this passes.**

---

## 8. Phase 5 — Re-enable cleanup (the LAST step)

Only after §7.3 passes. This restores normal nightly retention.

```bash
# [LAPTOP]
aws events enable-rule --name prices-production-cleanup --region eu-central-1 \
  --profile soroban-explorer

aws events describe-rule --name prices-production-cleanup --region eu-central-1 \
  --profile soroban-explorer --query State --output text
```

**✅ Checkpoint:** prints **`ENABLED`**. Done — the history is now permanently stored.

---

## 9. Troubleshooting & golden rules

**Golden rules**

- ✅ Cleanup rule stays **DISABLED** from now until Phase 5. Check it every phase.
- ✅ Never sample the progress query < 25 min apart — download-bound cadence makes
  a healthy run look frozen.
- ✅ Always keep the `WHERE sequence < 50457424` filter on the frontier query.
- ✅ Clear the low markers (§6.2) **before** launching pass 2, or it skips the span.
- ❌ Never use `preroll.sql` for Phase 4 — only `preroll-incremental.sql`.
- ❌ Don't touch the `soroban_amm` leg — separate issue, the primary operator's.
- ❌ Never `rm` files, never print secrets/keys.

**"The run looks stalled."** If two samples < 25 min apart show no movement, that's
expected. Confirm with samples > 25 min apart and check `contiguous_no_gaps = 1`.

**"A pass died / the process is gone before reaching target."** It is resume-safe.
Re-launch the **same command** (§6.3 for pass 2; for pass 1 the command is the same
but with `--end 50457423`). Markers already written are skipped automatically.

**"`ENABLED` appeared on the cleanup rule."** Disable it immediately (§4.3) and call
the primary operator — some backfilled partitions may have been deleted and may need
re-running.

**When to escalate:** cleanup turned itself on; you can't reach fishuser-hero and
pass 2 is due; the pre-roll won't complete after two tries; or anything not covered
here. Better to pause than to guess against production.

---

## 10. Reference values

| Thing                          | Value                                                                                                     |
| ------------------------------ | --------------------------------------------------------------------------------------------------------- |
| ClickHouse host (prod)         | `ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161`                                                 |
| Backfill machine               | `fishuser-hero` = `192.168.1.106` (private LAN)                                                           |
| Soroban activation ledger      | `50,457,424` (pass-1 target = `50,457,423`)                                                               |
| Pass-2 range                   | `[1, 23,423,999]`                                                                                         |
| Backfill top (never exceed)    | `63,352,611`                                                                                              |
| Cleanup rule                   | `prices-production-cleanup` (region `eu-central-1`, profile `soroban-explorer`)                           |
| Pre-roll script (use this one) | `packages/prices-clickhouse/schema/preroll-incremental.sql`                                               |
| Pass-2 log (on fishuser-hero)  | `~/sdex-pass2.log`                                                                                        |
| Runbooks                       | `docs/runbooks/preroll-incremental-presoroban.md`, `docs/runbooks/fix-backfill-history-loss-and-rerun.md` |

</content>
