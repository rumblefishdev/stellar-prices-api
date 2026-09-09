# Incremental pre-roll — pre-Soroban SDEX tail

**Run this once, at the END of the pre-Soroban SDEX tail backfill**, to roll the
`[genesis, activation)` `1m` candles up into the coarse forever-tables
(`_15m … _1M`) **without** disturbing the already-pre-rolled Soroban-era coarse.

- **Script:** [`packages/prices-clickhouse/schema/preroll-incremental.sql`](../../packages/prices-clickhouse/schema/preroll-incremental.sql)
- **Task:** 0088 · **Related:** 0090 (the full-rebuild pre-roll + cleanup coordination), 0095 (rollup MVs → APPEND)
- **Status:** ✅ **EXECUTED against prod 2026-08-11.** 718,619,989 pre-Soroban `1m`
  candles rolled into all six coarse tables, boundary `2024-02-20 17:01:00`.
  Coverage is now 2015–2024 at every granularity. The **script needed no edits**;
  this runbook did — three of its steps were wrong and are corrected below,
  each marked **CORRECTED 2026-08-11**.

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
Soroban-era row for the same key → on merge RMT keeps the Soroban row. Confirmed
locally: 2020 buckets appended at all six granularities, and a second run did not
double-count. **Confirmed on prod 2026-08-11 by arithmetic** — per table,
`final = written + pre-existing` exactly, so no pre-existing row was displaced.

⚠️ **One qualification, measured 2026-08-11 and NOT true as originally written.**
This section used to say the pre-Soroban partial at the activation-boundary
bucket "simply loses". That holds only where the competing version is strictly
lower. Because each coarse level carries `max(version)` upward, a boundary bucket
can inherit a Soroban-era version and **tie**, in which case the surviving row is
the _recomputed_ one. It still loses no data — the recomputed row incorporates the
prior values rather than discarding them — but which row survives is
non-deterministic under a tie. See §Accepted residual.

## Pre-flight (confirm before running)

0. ⚠️ **The script carries the `close_usd` guard (task 0145). Verify it — there
   is no deploy to check.**

   ```bash
   # run in the checkout you are actually pasting the SQL from
   grep -c 'argMax(close_usd, t.timestamp)' \
     packages/prices-clickhouse/schema/preroll-incremental.sql
   # MUST print 0. Non-zero = this is the pre-0145 script. STOP and `git pull`.
   ```

   **Why this step exists.** Until 2026-08-06 this script had 14 unguarded
   `argMax(close_usd, t.timestamp)` sites (0144 scope correction C1 found 121
   across the four pre-roll scripts). `close_usd` is baked by a separate,
   _lagging_ enrichment pass onto a non-nullable `Decimal(38,14) DEFAULT 0`
   column, so "not yet enriched" and "no USD price exists" are the same value —
   zero. Unguarded, a coarse bucket inherits that zero whenever its **newest**
   sub-bucket is un-enriched, discarding every priced sub-bucket beneath it.
   Here that happens at _backfill_ scale, over a span where enrichment is
   incomplete by definition, and the zeroed rows then age out of the MV
   re-aggregation windows where only the 0114 sweep can reach them (task 0148).

   **Why a check rather than a build gate.** 0145 shipped **no deployable
   artifact**. These are operator-run plain SQL scripts — nothing embeds them
   (`prices-clickhouse-init` applies `INIT`/`VIEWS`/`ROLLUPS`/`SEED`, never
   `PREROLL`), so there was nothing to deploy and there is no release to confirm
   against. The fix applies _only_ to whoever runs the script from an up-to-date
   checkout. A stale checkout silently executes the old SQL and every signal
   reports success. **fishuser-hero keeps its own `~/stellar-prices-api` at
   whatever commit it was left on** — `git pull` there before copying anything.

   Do **not** invert this by counting `argMaxIf` instead: the file header quotes
   the guard expression verbatim, so that count is one higher than the number of
   real sites. (This broke 0145's own guard test on first write — 7 not 6 — which
   is why the shipped test asserts over comment-stripped statements.)

1. **Tail is done.** `pgrep -af sdex-backfill` is gone / the run reports complete,
   and the floor reached activation-1:
   ```sql
   SELECT max(sequence) FROM prices.backfill_sdex_ledgers WHERE sequence < 50457424;
   -- expect ~50,457,423
   ```
2. 🔴 **Get the exact activation boundary — CORRECTED 2026-08-11.**

   > ⚠️ **The query this step used to specify is BROKEN and returns a dangerously
   > wrong answer.** It said to take the first Soroban-only-source candle:
   >
   > ```sql
   > -- DO NOT USE — returned 2026-07-01 on 2026-08-11
   > SELECT min(timestamp) FROM prices.price_ohlcv_1m
   > WHERE source IN ('aquarius','phoenix','soroswap');
   > ```
   >
   > That is the **`1m` retention horizon**, not activation. After 0090 the
   > Soroban-era `1m` partitions were dropped — which this very runbook states in
   > §Why a separate script — so the earliest surviving Soroban-source row is
   > wherever retention now begins. The step is invalidated by the partition drop
   > it describes.
   >
   > Deriving it from `price_ohlcv_15m` instead returns `2026-06-01` — _that_
   > table's horizon (see task 0174). **No edge-probe on any table can find
   > activation: every one reports its own retention state.**

   **Anchor on the activation LEDGER instead** — `50,457,424`, BE-measured and
   authoritative. The boundary is one minute past the last pre-activation candle:

   ```sql
   -- version = ledger_sequence*1000 + operation_index (bucket.rs:48)
   SELECT max(timestamp) AS last_pre_activation
   FROM prices.price_ohlcv_1m
   WHERE intDiv(toUInt64(version), 1000) < 50457424;
   -- 2026-08-11: 2024-02-20 17:00:00  ->  BOUNDARY = '2024-02-20 17:01:00'
   ```

   Cross-check that `1m` holds nothing between that value and the live tail; if
   so, any boundary in the gap is equivalent for STAGE 1, and the value above is
   the safe choice for STAGE 2.

   ⚠️ **Do not round to midnight.** `2024-02-20 00:00:00` would silently discard
   the activation day's 17 hours of pre-activation SDEX.

   **Why a wrong boundary is dangerous, not merely inaccurate.** STAGE 2 is
   bounded _only_ by `t.timestamp < {boundary}` — **no lower bound** — and reads
   the coarse tables, which **do** retain the Soroban era. A boundary of
   `2026-07-01` would re-aggregate ~2.3 years of already-pre-rolled coarse. The
   script's safety proof (_every inserted row has a version strictly lower than
   any Soroban-era row_) holds **only below activation**; past it, re-derived rows
   carry essentially the same versions — an **RMT tie**, the case 0097 found needs
   DELETE-first to resolve predictably. And `close_usd` would not re-derive
   identically anyway, since `argMaxIf(…, close_usd > 0)` reads enrichment state
   that has moved on since 0090.

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
run prod DDL from an agent). Use the **exact** activation timestamp from
pre-flight step 2 — do NOT assume midnight; activation is intraday, and a
midnight bound would drop the activation day's pre-activation SDEX slice:

Copy the script to the server and run it under `tmux` — at 718M rows this takes
minutes per stage and an SSH drop would kill a mid-flight statement:

```bash
scp -i ~/.ssh/sorban-prod_ed25519 \
  packages/prices-clickhouse/schema/preroll-incremental.sql \
  deploy@168.119.73.161:~/preroll-incremental.sql

# verify the copy — a stale/edited script is the 0145 regression risk
md5sum packages/prices-clickhouse/schema/preroll-incremental.sql
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 'md5sum ~/preroll-incremental.sql'
```

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161
tmux new -s preroll

time docker exec -i app-clickhouse-1 clickhouse-client \
  --param_boundary='2024-02-20 17:01:00' \
  --max_bytes_before_external_group_by=2000000000 \
  --multiquery \
  < ~/preroll-incremental.sql 2>&1 | tee -a ~/preroll.log
```

Success is **silent** — `INSERT … SELECT` prints nothing. `--multiquery` aborts on
the first error, so nothing after a failure runs; the script is idempotent, so
re-running simply redoes the cheap completed years and resumes.

### 🔴 The ~5.59 GiB quota — CORRECTED 2026-08-11

> ⚠️ **This section previously said "split that year into halves".** That advice
> was written against a much smaller dataset and **does not survive contact with
> 718M rows.** Measured failures, in order:

| statement            | rows scanned | outcome                                               |
| -------------------- | ------------ | ----------------------------------------------------- |
| STAGE 1 · 2022       | 211M         | OOM (`AggregatingTransform`)                          |
| STAGE 1 · 2023       | 435M         | OOM **even with spilling** (`SourceFromNativeStream`) |
| STAGE 2 · `1h ← 15m` | 159M         | OOM — unchunked, no lower bound                       |

**Enable disk spilling first** — it is one flag and needs no edit to the script:

```
--max_bytes_before_external_group_by=2000000000
```

⚠️ `max_bytes_ratio_before_external_group_by` is `0.5` on this cluster but
**never fires**: it is computed against available _server_ memory, which far
exceeds the 5.59 GiB per-query cap, so the threshold is never reached before the
query limit kills it. **Only the absolute setting has any effect.**

⚠️ **If it still OOMs in `SourceFromNativeStream`, do NOT lower the threshold.**
That failure is in the _readback_ of spilled data; a lower threshold produces
more spill files and more concurrent streams, making it worse. Chunk instead.

**Chunking that worked (2026-08-11):**

- **STAGE 1 · 2023 → one statement per month** (~36M rows each, 10–15 s each).
  The 12 chunks summed to exactly 82,640,286 = the observed `15m` 2023 total.
- **STAGE 2 → year-chunk `1h`, `4h`, `1d` only.**

> 🔴 **NEVER year-chunk `1w` or `1M`.** Their buckets straddle calendar years, so
> chunking splits a week or month across two statements and RMT keeps whichever
> landed with the higher version — **silent corruption**, unlike an OOM which
> merely stops. They read the small `1d`/`1w` tables, so memory is not a concern
> there anyway.

**Generate chunked variants — never hand-write them.** Transform the original
text so the 0145 `argMaxIf(close_usd, …)` guard is copied verbatim and only the
`WHERE` bounds change:

```bash
SRC=packages/prices-clickhouse/schema/preroll-incremental.sql

# Example: STAGE 1's 2023 block (lines 193-206) as 12 monthly statements
for m in 01 02 03 04 05 06 07 08 09 10 11 12; do
  if [ "$m" = "12" ]; then nxt="2024-01-01"; else nxt="2023-$(printf '%02d' $((10#$m + 1)))-01"; fi
  echo "-- 2023-$m"
  sed -n '193,206p' "$SRC" \
    | sed "s|WHERE t.timestamp >= '2023-01-01' AND t.timestamp < '2024-01-01'|WHERE t.timestamp >= '2023-$m-01' AND t.timestamp < '$nxt'|"
done > /tmp/preroll-2023-monthly.sql

# ALWAYS re-verify the guard on the generated file before running it
grep -c 'argMax(close_usd, t.timestamp)' /tmp/preroll-2023-monthly.sql   # MUST be 0
grep -v '^\s*--' /tmp/preroll-2023-monthly.sql \
  | grep -oiE '^\s*(INSERT|DELETE|TRUNCATE|DROP|ALTER)\b' | sort | uniq -c  # INSERT only
```

Then syntax-check it against a throwaway CH **on the pinned version** before it
goes near prod — this catches a `sed` slip in seconds:

```bash
docker run -d --name preroll-syntax -e CLICKHOUSE_DB=prices \
  clickhouse/clickhouse-server:26.3.10.60
docker exec -i preroll-syntax clickhouse-client --multiquery \
  < packages/prices-clickhouse/schema/init.sql
docker exec -i preroll-syntax clickhouse-client \
  --param_boundary='2024-02-20 17:01:00' --multiquery < /tmp/preroll-2023-monthly.sql
docker rm -f preroll-syntax
```

Derived chunk files are **deliberately not committed** — two copies of the
aggregation expression is exactly how the 0145 unguarded `argMax` survived. The
generator above is the artifact; regenerate from the script rather than reusing
a stale file.

- **STAGE 2** must run **after** STAGE 1 completes. If STAGE 1 dies partway, do
  not hand-run STAGE 2 — it reads what STAGE 1 wrote, so a partial `15m` would
  bake an incomplete rollup into every level above it.
- Add earlier-year blocks only if a count shows `1m` rows before 2015 (earliest
  SDEX candle is ~2016-03).
- **Killing the run is safe.** It is INSERT-only, so there is nothing to roll
  back; a killed `INSERT … SELECT` leaves a subset of _complete_ buckets, and
  re-running fills the rest and collapses duplicates by primary key.

## Post-run verification

Run these with the **same** `--param_boundary='$BOUNDARY'` as the pre-roll (the
`{boundary:DateTime}` placeholder needs the type annotation and the param, or CH
errors on an unbound query parameter):

```sql
-- Coarse now covers the pre-Soroban years (spot years):
SELECT '1d' t, toYear(timestamp) y, count() FROM prices.price_ohlcv_1d
WHERE timestamp < {boundary:DateTime} GROUP BY y ORDER BY y;

-- Boundary month preserved the Soroban value (version-wins), not clobbered:
SELECT timestamp, close, version FROM prices.price_ohlcv_1M FINAL
WHERE timestamp = toStartOfMonth({boundary:DateTime}) AND source='sdex' LIMIT 5;
```

Expect non-zero coarse rows for 2017–2023 and the boundary-month row unchanged
from its pre-run (Soroban) value.

Also check the chain is coherent — counts must **decrease** at every step
(`15m` > `1h` > `4h` > `1d` > `1w` > `1M`) within each year, at all ten years:

```sql
SELECT tbl, yr, rows_ FROM (
    SELECT '1_15m' AS tbl, toYear(timestamp) AS yr, count() AS rows_
      FROM prices.price_ohlcv_15m WHERE timestamp < {boundary:DateTime} GROUP BY yr
    UNION ALL
    SELECT '2_1h' AS tbl, toYear(timestamp) AS yr, count() AS rows_
      FROM prices.price_ohlcv_1h  WHERE timestamp < {boundary:DateTime} GROUP BY yr
    -- …repeat per table; ⚠️ aliases do NOT carry past the first UNION branch,
    --   so every branch needs its own `AS tbl` / `AS yr` / `AS rows_`
)
ORDER BY tbl, yr;
```

### Proving nothing was deleted

⚠️ **Do not use raw row counts as the guard.** They drift under background merges
on a live cluster — during the 2026-08-11 run `1h` 2024 fell 576 rows purely from
RMT collapsing duplicates, in a slice holding **26.1M** un-merged duplicates. That
produced a false alarm.

Two instruments that do work:

1. **Content checksums** over the untouched slice, captured _before_ the run:
   ```sql
   SELECT sum(sipHash64(toString(timestamp), asset_id, quote_asset_id, source,
            toString(open), toString(high), toString(low), toString(close),
            toString(volume_base), toString(volume_quote), toString(volume_quote_usd),
            toString(close_usd), toString(vwap), trade_count, version)) AS chk
   FROM prices.price_ohlcv_1d
   WHERE timestamp >= {boundary:DateTime} AND timestamp < '2026-01-01';
   ```
   Stop the upper bound short of the current year — live ingestion writes there
   and will produce spurious differences.
2. **The arithmetic, which is strongest:** capture pre-existing sub-boundary row
   counts before the run, then confirm `final = written + pre-existing` exactly
   per table, taking `written` from `system.query_log`. On 2026-08-11 all five
   tables reconciled to the row (`1h` 5,665,906+4,734 = 5,670,640, etc.).

⚠️ When reading `query_log`, match with a **leading wildcard** —
`query LIKE '%INSERT INTO prices.price_ohlcv%'`. The logged query text begins with
the statement's leading `--` comment, so an anchored prefix silently hides most of
the run (it showed 5 of 32 statements and looked like a failure).

## After it succeeds — re-enabling cleanup is a separate, operator-owned decision

The pre-roll's _precondition_ for re-enabling is satisfied the moment the coarse
tables are verified durable — they no longer depend on `1m`. Whether to actually
re-enable is the operator's call:

```bash
aws events enable-rule --name prices-production-cleanup --region eu-central-1 \
  --profile soroban-explorer
```

⚠️ **As of 2026-08-11 this was deliberately NOT done** — the operator chose to
keep every row on disk after the pre-roll. Consequence: the pre-Soroban `1m`
(718.6M rows) stays resident _in addition to_ the new coarse rows, so disk climbs
rather than falls. Confirm the current state before assuming either way:

```bash
aws events describe-rule --name prices-production-cleanup --region eu-central-1 \
  --profile soroban-explorer --query 'State'
```

⚠️ An `UnrecognizedClientException` here is an **expired SSO session**, not a
wrong profile name — `aws sso login` and retry. `--profile soroban-admin` also
works where the documented `soroban-explorer` fails.

## Accepted residual — ⚠️ the "optional repair" is NO LONGER POSSIBLE

The activation month/week's coarse buckets reflect only their **post-activation**
slice (the pre-activation SDEX partial loses on version). This drops at most the
`[2024-01-01 … activation)` SDEX sliver from the boundary calendar buckets.

> 🔴 **CORRECTED 2026-08-11.** This section used to offer a repair: _"recompute
> just those boundary buckets from the full `1m` (both sides) — possible only
> while the boundary-month Soroban `1m` is still resident."_
>
> **That window has already closed.** `price_ohlcv_1m` partition `202402` holds
> nothing after `2024-02-20 17:00` — the post-activation side was dropped after
> 0090's pre-roll, long before this runbook ran. The repair requires both sides
> of the boundary month and only one exists.
>
> **The activation month/week residual is therefore permanent** (barring a
> re-download of that span from ledger archives). Do not plan around the repair;
> it cannot be performed.

Measured behaviour at the boundary, from a local fixture on the pinned engine
(26.3.10.60): the pre-existing boundary row is **not deleted**. Where the
competing pre-Soroban row carries a strictly lower version the existing row wins
outright; where the chain propagates the same version upward the two **tie**, and
the surviving row is the recomputed one — which _incorporates_ the old values
rather than discarding them (its `volume_base` was the sum of both). Either way
no data is lost, but the tie makes which row survives non-deterministic, so treat
the boundary bucket as approximate rather than authoritative.
