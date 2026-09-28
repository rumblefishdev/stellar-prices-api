# Runbook — changing a rollup MV body on a provisioned target (task 0142)

How to land an edit to `packages/prices-clickhouse/schema/rollups.sql` on a
cluster that already holds the six refreshable rollup MVs, without
reintroducing the task 0090/0095 data loss.

**Applies to:** `mv_ohlcv_1m_to_15m`, `_15m_to_1h`, `_1h_to_4h`, `_4h_to_1d`,
`_1d_to_1w`, `_1d_to_1M` on ch-prod-01, and (since tasks 0143 + 0203) the six
hourly reconciliation MVs `mv_reconcile_1m_to_15m` … `mv_reconcile_1d_to_1M`.

> **Landing tasks 0143 + 0203 (the `DEPENDS ON` chain and the reconcile MVs)?**
> Go to [Tasks 0143 + 0203](#tasks-0143--0203--dependency-order-and-reconciliation).
> The five fast dependents change in place with `MODIFY REFRESH`; nothing is
> dropped.

> ⚠️ **If the re-CREATE you are here for is task 0286's**, follow
> [`0286-candle-definitions-rollout.md`](0286-candle-definitions-rollout.md)
> instead of this file alone. That rollout changes all six bodies at once,
> renames the month's MV (`mv_ohlcv_1w_to_1M` → `mv_ohlcv_1d_to_1M`, because the
> month now rolls from the DAY) and rebuilds `price_ohlcv_1M` — none of which
> this runbook covers. It uses the checklist below for each individual MV, and
> wraps it in the FREEZE, the deploy order and the rollback the change needs.

## Why you cannot just re-apply the file

Every statement in `rollups.sql` is `CREATE MATERIALIZED VIEW IF NOT EXISTS`,
and `IF NOT EXISTS` does not redefine an object that already exists. On a target
that holds the MV, **re-applying an edited file changes nothing and reports
success.** Verified on 26.3.10.60 and pinned by
`tests/rollup_drift_it.rs::an_edited_body_is_reported_as_drift_because_the_reapply_silently_no_ops`.

Task 0134 removed this footgun from `views.sql` by converting those to `CREATE
OR REPLACE VIEW`. That escape does not exist here: a refreshable `TO`-table MV
has no `OR REPLACE` form. The only route is `DROP` + re-`CREATE`, which is why
this is an operator procedure rather than something the apply path does.

> ⚠️ **Requires a privileged user.** `DROP VIEW` / `CREATE MATERIALIZED VIEW`
> need DDL grants the scoped production users (`prices_writer`,
> `prices_reader`) do not have and cannot be granted by us — they are
> XML-managed. On ch-prod-01 this runs as the container's `default` user over
> the loopback native port, the same path `views.sql` uses.
>
> The drift check in step 1 is the exception: it is read-only and runs as any
> account that can read `system.tables`.

## Where these commands run

⚠️ **Read this before step 1.** `Config::from_env()` defaults `CLICKHOUSE_URL` to
`http://localhost:8123`, so a bare `cargo run` on a local machine checks the
**local dev ClickHouse** and exits 0 — a clean result that says nothing whatever
about ch-prod-01. Every command below is written for the Hetzner host.

Prod's HTTP endpoint (`ch.sorobanscan.rumblefish.dev`) is mTLS-only behind Caddy
and `prices_clickhouse::client()` builds a plaintext client, so there is no path
from a local machine to prod for this binary. Run it **on the host, against the
loopback port** — the same pattern as
[`events-sourced-amm-reprice.md`](events-sourced-amm-reprice.md):

```bash
# On the Hetzner host (connection details: the prod SSH access note).
# Build locally for the host target and copy the binary up, or build on the host.
read -rs CH_PW
CLICKHOUSE_URL=http://localhost:8123 \
CLICKHOUSE_USER=default \
CLICKHOUSE_PASSWORD="$CH_PW" \
CLICKHOUSE_DATABASE=prices \
  ./prices-clickhouse-drift
```

The password goes in the environment, never in `argv` — `/proc/<pid>/cmdline` is
world-readable, so a flag would expose it to any `ps` on the box.

The tool logs `url`, `user` and `database` at startup. **Read that line before
you read the result**; it is the only thing standing between a local all-clear
and a statement about production.

Any account that can read `system.tables` will do — but note that view is
grant-filtered, so an account without rights on the MV objects reports all six
as `MISSING` rather than erroring. The tool flags that shape explicitly when
every declared MV comes back missing.

## Step 1 — see what the target actually holds

```bash
prices-clickhouse-drift
prices-clickhouse-drift --verbose
```

Read-only: it issues `SELECT`s against `system.tables` and
`formatQuerySingleLine`, and creates, alters and drops nothing. Exit 0 means
every declared MV (twelve since task 0203: six fast, six reconcile) is present,
matches the file and is in `APPEND` mode; exit 1 means at least one needs
attention. A missing or wrong `DEPENDS ON` is reported as `DRIFT` on the
`refresh` field.

Run this **before** you edit anything. A target that has already drifted is a
different job from landing a new edit, and doing both in one pass makes the
verification in step 5 unreadable.

Five findings it reports, in descending severity:

| Report                          | Meaning                                                                                                                                                                                             |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CRITICAL … NOT in APPEND mode` | Replace mode. Every refresh is atomically replacing the whole target table with just its bounded window — the task 0090 data loss, **happening now**. Fix this first and independently of any edit. |
| `MISSING`                       | Declared in the file, absent on the target. That tier is not rolling up (the task 0136 shape). A plain re-apply fixes it — `IF NOT EXISTS` creates what is not there.                               |
| `DRIFT`                         | Live definition and file disagree. Re-applying will NOT fix it; that is what the rest of this runbook is for.                                                                                       |
| `EXTRA`                         | An MV not in the file writes into a table the declared MVs own — two writers into one `ReplacingMergeTree`. Find out what created it before changing anything else.                                 |
| `UNKNOWN`                       | The object exists but its definition is not a shape the check can read — most likely re-created without `REFRESH`, which makes it an insert-trigger MV with entirely different semantics.           |

## Step 2 — pre-flight the new definition

Check the edited statement against all four invariants **before** dropping
anything. A re-`CREATE` is the moment these get silently re-decided, and three
of the four have already caused production incidents.

- [ ] **`REFRESH … APPEND`** — the keyword is present. Without it the MV
      atomically replaces its entire target table on every refresh; paired with
      the bounded `WHERE` below, that deletes all pre-rolled history. This is
      task 0090's data loss and task 0095's fix. Non-negotiable.
- [ ] **`sum(version)`, not `max(version)`** — the target is a
      `ReplacingMergeTree(version)`, so the projected version decides which
      re-inserted row wins. `max` ties when an early row in a bucket is
      corrected, and RMT's tie-break is not contractual (task 0059 #5).
- [ ] **Window lower bound aligned to the coarse bucket** —
      `toStartOfInterval(now() - <window>, INTERVAL <coarse-grain>)`, never a
      raw `now() - <window>`. A raw bound falls mid-bucket, so the oldest bucket
      in the window is rebuilt from only its in-window slice and a **partial**
      bucket gets appended over complete pre-rolled history (task 0095).
- [ ] **`t.`-qualified source columns** — the bucket key must be aliased `AS
timestamp` for the `TO`-table insert routing to work, and that alias
      shadows the source column inside the SELECT. A bare `timestamp` in
      `argMin`/`argMax`/`WHERE` resolves to the constant bucket start, not the
      per-row time (task 0071). Renaming the bucket is not an option — see the
      header of `rollups.sql`.

Then prove the edit locally against the prod-pinned server before it goes near
the cluster. CI runs these three — and every other ClickHouse integration test —
on the PR (task 0275), but a schema edit is worth seeing green before it is
pushed:

```bash
docker compose up -d --wait clickhouse    # 26.3.10.60, the prod pin
tools/scripts/ignored-tests.sh preflight  # --wait is not readiness: see below
cargo run -q -p prices-clickhouse --bin prices-clickhouse-init -- --rollups
cargo test -p prices-clickhouse --lib
cargo test -p prices-clickhouse --test rollup_drift_it   -- --ignored --test-threads=1
cargo test -p prices-clickhouse --test rollup_append_it  -- --ignored --test-threads=1
cargo test -p prices-clickhouse --test rollup_chain_it   -- --ignored --test-threads=1
```

`preflight` is not optional on a fresh `clickhouse-data` volume. `--wait`
reports healthy while the image's temporary initdb server — listening only
inside the container — is still up; it is then killed, and for a moment nothing
serves the host port. `preflight` retries from the host until it is served, then
checks `version()` against the pin and `timezone()` is UTC. CI does the same.

Or all of them, exactly as CI does, after the three setup lines above:
`scripts/ch-proxy-0281.sh up`, then
`CLICKHOUSE_PROXY_URL=http://localhost:8124 tools/scripts/ignored-tests.sh`.

`rollup_append_it` is the one that matters most here: it places data **outside**
the refresh window and proves a refresh preserves it. An edit that reintroduces
replace mode passes `rollup_chain_it` and fails this.

## Step 3 — the exposure while an MV is dropped

**One MV at a time, and never leave one dropped.** While an MV is gone its tier
receives nothing, which looks exactly like a quiet market — task 0136 is the
precedent, where starved rollups went nine days unnoticed.

The gap is **self-healing if you are quick, because each MV re-aggregates a
bounded window rather than only the newest bucket.** Once re-created, the first
refresh rebuilds everything in its window, back-filling what was missed. So the
requirement is simply that the outage stays well inside the window:

| MV                   | refresh cadence | window   | practical budget                          |
| -------------------- | --------------- | -------- | ----------------------------------------- |
| `mv_ohlcv_1m_to_15m` | 1 min           | 2 h      | tightest — but a DROP + CREATE is seconds |
| `mv_ohlcv_15m_to_1h` | 15 min          | 8 h      | ample                                     |
| `mv_ohlcv_1h_to_4h`  | 1 h             | 1 day    | ample                                     |
| `mv_ohlcv_4h_to_1d`  | 4 h             | 7 days   | ample                                     |
| `mv_ohlcv_1d_to_1w`  | 1 day           | 60 days  | ample                                     |
| `mv_ohlcv_1d_to_1M`  | 1 day           | 400 days | ample                                     |

**Measured 2026-08-14 on the 26.3.10.60 pin:** a freshly created refreshable MV
runs its initial refresh **immediately at creation**, not at the next scheduled
boundary — `last_success_time` equals the `CREATE` time — and `next_refresh_time`
then realigns to the normal clock boundary. So the catch-up is automatic; no
manual `SYSTEM REFRESH VIEW` is required, though it is available if you want to
force one.

The real risk here is therefore **not** the length of the gap. It is a botched
re-`CREATE` — which is what step 2 exists for, and why the statement you are
about to run should be in your clipboard before you drop anything.

⚠️ **Pair this with the task 0137 rollup freshness alarm.** If the alarm is
routed to Slack, expect it to fire for the dropped tier if the outage outlasts
its bound (bucket width + feeding-MV refresh). Do not silence it; let it prove
it works, and confirm it returns to OK in step 5.

## Step 4 — drop and re-create, one MV at a time

For each MV, **fine-to-coarse** — `mv_ohlcv_1m_to_15m` first, the two dailies
(`mv_ohlcv_1d_to_1w`, `mv_ohlcv_1d_to_1M`) last — so that each MV is re-created
only once its own source has already been corrected.

⚠️ **The order is load-bearing, and the intuitive one is backwards.** Every MV
reads the tier below it, and (per step 3) a freshly created MV refreshes
_immediately_. Re-create `mv_ohlcv_1d_to_1M` first and its one and only refresh
for the next 24 hours re-aggregates `price_ohlcv_1d` rows that the old
`mv_ohlcv_4h_to_1d` wrote — so a correction propagates upward not at all. Going
fine-to-coarse, each re-created MV reads a source its predecessor has already
fixed.

Since task 0143 the five dependents carry `DEPENDS ON` — see
[DROP + CREATE with dependencies](#3-fallback--drop--create-with-dependencies)
for what happens to them while their dependency is dropped.

```sql
-- 1. drop
DROP VIEW prices.mv_ohlcv_1m_to_15m;

-- 2. re-create — paste the edited statement from schema/rollups.sql verbatim,
--    including `IF NOT EXISTS` (harmless: you just dropped it, and keeping the
--    file and the executed statement identical is what makes step 5 clean).
CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1m_to_15m
REFRESH EVERY 1 MINUTE APPEND
TO prices.price_ohlcv_15m AS
SELECT …;
```

Then confirm this one before touching the next:

```sql
SELECT view, status, last_success_time, next_refresh_time, exception
FROM system.view_refreshes
WHERE database = 'prices' AND view = 'mv_ohlcv_1m_to_15m';
```

`status = Scheduled`, a `last_success_time` at or after your `CREATE`, and an
empty `exception`. A non-empty `exception` means the MV exists but is failing on
every tick — visible here and nowhere else.

> **Re-creating by re-applying the file — know what else that touches.** Once an
> MV is dropped, `IF NOT EXISTS` will create it, so applying `rollups.sql`
> re-creates every _missing_ MV and leaves every existing one untouched. It is
> **not** a way to land an edit on an MV that still exists.
>
> ⚠️ But `prices-clickhouse-init --rollups` does **not** apply only that file.
> Before it reaches `ROLLUPS_SQL` it unconditionally applies `init.sql`, seeds
> `backfill_progress`, and applies all six `CREATE OR REPLACE VIEW` statements in
> `views.sql` — **from your working tree**, over whatever ch-prod-01 currently
> holds. That is a much larger blast radius than "re-create one missing MV", and
> it needs the `DROP VIEW` grant those replaces require.
>
> For a single missing tier, prefer the explicit `CREATE` above. Reach for the
> binary only when you intend to re-land the whole schema, and only from a
> checkout you have verified against the cluster.

## Step 5 — verify

On the host, same environment as step 1 (see "Where these commands run" — a
local run here verifies your dev machine, not the cluster you just changed):

```bash
prices-clickhouse-drift
```

Exit 0, and every line `ok`. That is the only check that confirms the edit
actually landed — the `CREATE` reporting success does not, which is the whole
premise of task 0142.

Then verify on the **data**, not on the DDL:

```sql
-- the tier is advancing again
SELECT max(timestamp) FROM prices.price_ohlcv_1M;

-- and history was not truncated by a replace-mode mistake
SELECT min(timestamp), count() FROM prices.price_ohlcv_1M;
```

⚠️ **Compare `min(timestamp)` and `count()` against what you recorded before
step 4.** A replace-mode re-`CREATE` shows up here as history collapsing to the
window, and it will already have happened by the first refresh. Take the
readings before you drop.

Finally, confirm the task 0137 freshness alarm has returned to OK — and note the
lesson task 0204 records: an alarm returning to OK is not by itself proof of
recovery. The `min`/`count` readings above are.

## Rollback

There is no snapshot to restore — an MV is a definition, not data. Rollback is
`DROP VIEW` + re-`CREATE` from the **previous** statement, which is why you
should have it to hand (`git show HEAD:packages/prices-clickhouse/schema/rollups.sql`)
before starting.

The data is a different question. If a botched re-`CREATE` ran in replace mode
even once, the target table has lost everything outside its window, and recovery
is a pre-roll — see `docs/runbooks/0136-coarse-rollup-merge-recovery.md` and the
`preroll-live-gap.sql` path, not this runbook.

## Tasks 0143 + 0203 — dependency order and reconciliation

What changed in `schema/rollups.sql` (decision record:
`lore/2-adrs/XXXX_rollup-mvs-dependency-order-and-reconciliation.md`):

- **0143.** The five fast MVs above `mv_ohlcv_1m_to_15m` wait for the MV that
  writes their source (`REFRESH … DEPENDS ON prices.<mv> APPEND`). At 00:00 the
  two dailies no longer read `price_ohlcv_1d` before `mv_ohlcv_4h_to_1d` has
  written the day that just closed. Bodies, windows and cadences are unchanged.
- **0203.** Six new MVs, `mv_reconcile_1m_to_15m` … `mv_reconcile_1d_to_1M`,
  run `EVERY 1 HOUR` over a bucket-aligned 7-day window. Each writes into the
  same table as its fast MV, and only the buckets whose `trade_count` or
  `volume_base` disagree with the tier below (missing buckets included). They
  are chained among themselves the same way, so one hourly pass repairs
  15m → 1h → 4h → 1d → {1w, 1M}. No fast MV depends on a reconcile MV, and
  `mv_reconcile_1m_to_15m` depends on nothing.

**When.** Only after task 0286 phase 3 has finished on prod: the re-ingest
rewrites `price_ohlcv_1m` under the reconcile window (see
[0286-reingest-history §1a](0286-reingest-history.md#1a-stop-the-reconcile-mvs-tasks-0143--0203)).
Everything below runs as the ClickHouse admin, as in
[Where these commands run](#where-these-commands-run).

**Before you start.**

- `SHOW GRANTS FOR prices_writer` includes `SELECT ON system.view_refreshes`.
  The probe reads it with that identity, and the table is DENIED
  (`Code: 497 ACCESS_DENIED`), not grant-filtered, to a user holding only
  `SELECT ON prices.*`. Without the grant `prices-production-mv-refresh-unreadable`
  fires by design. The users are XML-managed on BE's side (BE task 0477).
- Size the first reconcile pass. Its memory scales with the number of
  (series, 15-minute bucket) groups over 7 days, not with rows. Read-only:

  ```sql
  SELECT uniq(asset_id, quote_asset_id, source) AS series,
         uniq(asset_id, quote_asset_id, source,
              toStartOfInterval(timestamp, INTERVAL 15 MINUTE)) AS groups_15m
  FROM prices.price_ohlcv_1m
  WHERE timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 15 MINUTE);
  ```

  Measured locally on 26.3.10.60 (~3 M `_1m` rows, full emit into an empty
  target): 200 k groups → 238 MiB, 1.2 M → 1.19 GiB, 3.0 M (one row per group,
  the worst case) → 2.26 GiB, i.e. 40 % of the 5.59 GiB per-query quota. If
  `groups_15m` is well above 3 M, stop and re-measure before creating anything.

- Run `prices-clickhouse-drift` built from this change. Expected before the
  rollout: five `DRIFT` (the `refresh` field of the five fast dependents) and
  six `MISSING` (the reconcile MVs). Anything else is a different job — see
  Step 1.
- **Order, and one session.** The MVs change first (§1, §2), the probe Lambda
  and ObservabilityStack from this change are deployed after them (§2b), and
  §1 → §2 → §2b run in ONE session. `prices-production-mv-drift` fires for
  the whole of that window from the probe deployed today, whichever order you
  choose — see §2b for why, and for when it clears.

### 1. Preferred rollout — `MODIFY REFRESH` on the five fast dependents

The fast bodies do not change, so nothing is dropped and there is no exposure
window. Apply bottom-up, one at a time:

```sql
ALTER TABLE prices.mv_ohlcv_15m_to_1h MODIFY REFRESH EVERY 15 MINUTE DEPENDS ON prices.mv_ohlcv_1m_to_15m APPEND;
ALTER TABLE prices.mv_ohlcv_1h_to_4h MODIFY REFRESH EVERY 1 HOUR DEPENDS ON prices.mv_ohlcv_15m_to_1h APPEND;
ALTER TABLE prices.mv_ohlcv_4h_to_1d MODIFY REFRESH EVERY 4 HOUR DEPENDS ON prices.mv_ohlcv_1h_to_4h APPEND;
ALTER TABLE prices.mv_ohlcv_1d_to_1w MODIFY REFRESH EVERY 1 DAY DEPENDS ON prices.mv_ohlcv_4h_to_1d APPEND;
ALTER TABLE prices.mv_ohlcv_1d_to_1M MODIFY REFRESH EVERY 1 DAY DEPENDS ON prices.mv_ohlcv_4h_to_1d APPEND;
```

These are the generator's output (`rollup_sql::mv_modify_refresh`), and the
unit test `the_reapply_runbook_quotes_every_generated_modify_refresh_statement`
fails if this block and the generator disagree. Do not hand-edit them.

- ⚠️ **`MODIFY REFRESH` replaces ALL refresh parameters.** The whole clause is
  repeated on purpose: a statement without `DEPENDS ON` removes it, and one
  without `APPEND` is refused (`Code: 48 … Adding or removing APPEND is not
supported`). Replace mode cannot be reintroduced this way.
- `mv_ohlcv_1m_to_15m` has no dependency, so its clause does not change and
  there is no statement for it.
- After each statement, confirm it landed before the next:

  ```sql
  SELECT name, create_table_query LIKE '%DEPENDS ON prices.%APPEND TO %' AS chained
  FROM system.tables WHERE database = 'prices' AND name = 'mv_ohlcv_15m_to_1h';
  ```

  `chained = 1`, and `system.view_refreshes` shows the view `Scheduled` with an
  empty `exception`.

### 2. Create the six reconcile MVs

Paste each `CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_reconcile_…`
statement from `schema/rollups.sql` verbatim, **fine to coarse** (the file
order: `1m_to_15m`, `15m_to_1h`, `1h_to_4h`, `4h_to_1d`, `1d_to_1w`, `1d_to_1M`),
**one at a time, off-peak**.

A new refreshable MV refreshes immediately at `CREATE`, even with `DEPENDS ON`
(measured on 26.3.10.60). So each `CREATE` runs a full 7-day comparison of its
tier right away. Wait for it before the next:

```sql
SYSTEM WAIT VIEW prices.mv_reconcile_1m_to_15m;

SELECT view, status, last_success_time, last_success_duration_ms, exception
FROM system.view_refreshes
WHERE database = 'prices' AND view = 'mv_reconcile_1m_to_15m';

-- the reconcile passes' peak memory, per target table
SYSTEM FLUSH LOGS;
SELECT extract(query, '^INSERT INTO prices\\.(\\w+)') AS target,
       max(event_time) AS last_pass,
       argMax(query_duration_ms, memory_usage) AS duration_ms,
       formatReadableSize(max(memory_usage)) AS peak
FROM system.query_log
WHERE type = 'QueryFinish' AND query_kind = 'Insert'
  AND event_time >= now() - INTERVAL 15 MINUTE
  AND query LIKE 'INSERT INTO prices.price_ohlcv_%'
  AND query LIKE '%NOT IN (%'   -- only a reconcile body has it
GROUP BY target
ORDER BY max(memory_usage) DESC;
```

`status = Scheduled`, `last_success_time` at or after the `CREATE`, an empty
`exception`, and a peak well under the 5.59 GiB quota. The `15m` pass is the
heaviest: it is the only one that reads `price_ohlcv_1m`.

⚠️ **Keep the `NOT IN (` filter.** A refresh pass is logged as
`INSERT INTO prices.<target> (<columns>) SELECT …`, and each fast MV writes the
same target with the same prefix — `mv_ohlcv_1m_to_15m` every minute, so about
15 fast inserts for every reconcile pass in a 15-minute window. Without the
filter the query reports a small, healthy-looking fast pass instead of the
heaviest one. Only a reconcile body contains `NOT IN (` (the unit test
`only_the_reconcile_bodies_contain_not_in` pins that). Read the maximum
(`ORDER BY max(memory_usage)`), not the latest.

`system.view_refreshes.written_rows` reads **0 when a pass wrote nothing**. A
non-zero value is not a row count on 26.3.10.60 (it was measured at 256 × the
buckets appended), so read it as "something was written", never quote it as a
number of rows. A pass rewrites only closed buckets that ended at least 2 h ago
(`MISMATCH_GRACE`), never the open bucket, which stays the fast MV's. So on a
live system `0` is the normal steady state after the first pass, and a non-zero
value means the pass repaired something (a back-fill, a missed bucket).

### 2b. Deploy the probe and ObservabilityStack — after §1 and §2

Only once the twelve MVs are in place, and in the same session as §1–§2.

**Why after, and why in one session.** The probe deployed on production
today embeds the OLD `rollups.sql` (six MVs, no `DEPENDS ON`). Its drift check
counts refresh drift and undeclared writers in `MvDriftCount`, not in the
critical count (the reconcile MVs keep `APPEND`). So:

- from the first §1 statement it reports up to five refresh drifts, and from
  the first §2 `CREATE` up to six undeclared writers into the coarse tables:
  **`prices-production-mv-drift` fires** (up to 11), and stays in ALARM until
  the new probe runs. That is expected; `-mv-drift-critical` stays OK;
- deploying the new probe FIRST does not avoid it: the new probe declares
  twelve MVs, so against the old chain it reports five `DRIFT` + six
  `MISSING` and `prices-production-mv-drift` fires just the same (and the
  mismatch and `mv-refresh-*` alarms would judge a chain that does not exist
  yet).

Either order has a `mv-drift` window; MVs first keeps it to the length of this
session. Do not leave §1–§2 applied overnight with the old probe.

**Steps.**

1. `prices-clickhouse-drift` built from this change: exit 0, twelve lines
   `ok`. `system.view_refreshes` (query in §4) shows twelve views, none
   `WaitingForDependencies` or `Disabled`. Do not deploy on anything else.
2. **The grant, again, before the stack:** `SHOW GRANTS FOR prices_writer`
   includes `SELECT ON system.view_refreshes`. Without it the new probe
   publishes `MvRefreshUnreadable = 1` and `prices-production-mv-refresh-unreadable`
   fires as soon as the stack lands (by design: the waiting/disabled counts are
   suppressed, not zero). If it is missing, deploy only when you accept that
   alarm, and chase the grant with BE (task 0477).
3. Deploy, reading the diff for removals first (`--require-approval
broadening` prompts on IAM/security-group widening only, and the
   EventBridge stack carries the CleanupRule hazard of task 0200, guarded at
   synth by `assertCleanupRuleStaysDisabled` — still read it):

   ```bash
   make diff-production
   make deploy-production-eventbridge     # the probe Lambda (rollup-freshness-probe)
   make deploy-production-observability   # rollup-mismatch-*, mv-refresh-* alarms
   ```

   The probe first: that is what ends the `mv-drift` window.

4. Expected afterwards: `prices-production-mv-drift` returns to OK within two
   probe runs (≤ 30 min: the alarm is 1 of 2 fifteen-minute periods). The six
   `prices-production-rollup-mismatch-*` and three `mv-refresh-*` alarms leave
   `INSUFFICIENT_DATA` once the new probe has published, and read OK on a
   healthy chain. Then continue with §4.

### 3. Fallback — DROP + CREATE with dependencies

If `MODIFY REFRESH` is refused, or a body has to change as well, use Steps 2–5
above, bottom-up, with these differences:

- **Create bottom-up.** A dependent created before its dependency waits
  (`WaitingForDependencies`) from its first scheduled slot on.
- **Dropping a dependency does not touch its dependents.** They keep their
  `DEPENDS ON` in the stored DDL. They stay `Scheduled` until their next slot,
  then wait (`WaitingForDependencies`) **with no error** for as long as the
  dependency is absent. When it is re-created under the same name they re-link
  by name. So a DROP + CREATE of `mv_ohlcv_4h_to_1d` also stalls both dailies
  for as long as it is gone.
- A stopped (`SYSTEM STOP VIEW`), failing or misspelled dependency blocks its
  dependents the same way, silently. `prices-production-mv-refresh-waiting`
  and `prices-production-mv-refresh-disabled` exist for that.
- `SYSTEM REFRESH VIEW` ignores dependencies. Use it to force one view, never to
  "unstick" a chain whose dependency is still missing.
- Reconcile MVs are dropped and re-created the same way, from the file, fine to
  coarse. Each re-`CREATE` runs its 7-day comparison again (Step 2).

### 4. Verify

```sql
SELECT view, status, last_success_time, next_refresh_time, exception
FROM system.view_refreshes
WHERE database = 'prices'
  AND (view LIKE 'mv_ohlcv_%' OR view LIKE 'mv_reconcile_%')
ORDER BY view;
```

- Twelve rows. None `WaitingForDependencies` or `Disabled` for longer than one
  of its own periods (a dependent waits a few seconds at every shared slot;
  that is the ordering working). Every `exception` empty.
- `prices-clickhouse-drift` (built from this change): exit 0, twelve lines `ok`.
- Once the probe is deployed (§2b): `RollupMismatchBuckets` reads 0 for every coarse
  table within one to two hourly cycles. It counts only closed buckets that
  ended at least 2 h ago. A `_1m` hole shows on `price_ohlcv_15m` first: each
  tier is compared only with the tier directly below, so the coarser tiers agree
  with their (equally holed) source until the reconcile pass propagates the
  repair upward. A coarser table mismatching on its own means the chain broke
  part-way.
- `SHOW GRANTS FOR prices_writer` includes `SELECT ON system.view_refreshes`;
  otherwise `prices-production-mv-refresh-unreadable` stays in ALARM.
- **The probe's time budget.** Over the first day after §2b, read the
  `prices-production-rollup-freshness-probe` Lambda's `Duration` (p95 and
  maximum) against its 1-minute timeout, and check that
  `prices-production-rollup-freshness-probe-duration-near-timeout` stays OK.
  The six mismatch reads run last, 15m first, each bounded by what is left of
  the invocation. A tier that cannot get 2 s is skipped: the invocation log
  shows `rollup_mismatch=… price_ohlcv_1M=skipped` and the probe's errors alarm
  fires with `rollup-mismatch … skipped`. If that happens, or Duration sits near
  the timeout, raise the timeout of the probe in `eventbridge-stack.ts` as its
  own change (that stack's deploy carries the CleanupRule hazard, task 0200).

### 5. An outage longer than 7 days

The reconcile window is 7 days (`RECONCILE_WINDOW` in `src/rollup_sql.rs`), so
buckets older than that are never compared or repaired. `price_ohlcv_1m`
retention sets the ceiling of what any repair can reach (task 0200). For a hole
older than the window, pre-roll the range with the bounded, generated
`schema/preroll-live-gap.sql` and its `{start_ts}` (inclusive) / `{end_ts}`
(exclusive) parameters, fine to coarse, exactly as
[0286-reingest-history §4g](0286-reingest-history.md#4g-drop-the-overlapping-coarse-partitions-and-pre-roll-the-month)
does. Never `preroll.sql` on populated tables (the task 0090 history loss). If
the range overlaps the last 7 days, stop the reconcile MVs for the duration
([0286-reingest-history §1a](0286-reingest-history.md#1a-stop-the-reconcile-mvs-tasks-0143--0203)).

### 6. Rollback

If the probe Lambda and ObservabilityStack from this change are deployed, roll
them back first (`make deploy-production-eventbridge` and
`make deploy-production-observability` from the commit before this change),
then run the steps below in the same session. What that order buys, and what it
does not:

- **It removes** the `prices-production-rollup-mismatch-*` and
  `mv-refresh-*` alarms, which would otherwise judge a chain that is being
  taken apart (a dropped reconcile MV's dependents wait, and the probe cannot
  tell a rollback from a stall).
- **It does NOT avoid `prices-production-mv-drift`.** The previous probe
  embeds the six-MV `rollups.sql`, so while steps 1–2 run it sees the six
  reconcile MVs as undeclared writers into the coarse tables and the five
  `DEPENDS ON` clauses as refresh drift (up to 11 in `MvDriftCount`, none
  critical). The alarm fires until step 2 finishes and clears within two probe
  runs after. That is expected. Rolling the probe back LAST instead fires it
  too (the new probe reads the rolled-back chain as five `DRIFT` + six
  `MISSING`), so neither order avoids it — keep the window short.

1. Drop the reconcile MVs **coarse to fine**, so no remaining reconcile MV is
   left waiting on a dropped one:

   ```sql
   DROP VIEW prices.mv_reconcile_1d_to_1M;
   DROP VIEW prices.mv_reconcile_1d_to_1w;
   DROP VIEW prices.mv_reconcile_4h_to_1d;
   DROP VIEW prices.mv_reconcile_1h_to_4h;
   DROP VIEW prices.mv_reconcile_15m_to_1h;
   DROP VIEW prices.mv_reconcile_1m_to_15m;
   ```

   The rows they wrote stay. They are ordinary rollup rows over the same source,
   and the fast MVs keep re-rolling their own windows.

2. Take the dependencies off the five fast MVs. `APPEND` is repeated, because
   `MODIFY REFRESH` replaces every parameter and refuses to remove it:

   ```sql
   ALTER TABLE prices.mv_ohlcv_1d_to_1M MODIFY REFRESH EVERY 1 DAY APPEND;
   ALTER TABLE prices.mv_ohlcv_1d_to_1w MODIFY REFRESH EVERY 1 DAY APPEND;
   ALTER TABLE prices.mv_ohlcv_4h_to_1d MODIFY REFRESH EVERY 4 HOUR APPEND;
   ALTER TABLE prices.mv_ohlcv_1h_to_4h MODIFY REFRESH EVERY 1 HOUR APPEND;
   ALTER TABLE prices.mv_ohlcv_15m_to_1h MODIFY REFRESH EVERY 15 MINUTE APPEND;
   ```

3. Verify with `prices-clickhouse-drift` built from the commit before this
   change: six lines `ok`. The binary from this change reports the rolled-back
   chain as five `DRIFT` and six `MISSING`, which is the expected reading of a
   rollback, not a failure.

## Related

- `docs/runbooks/0136-coarse-rollup-merge-recovery.md` — per-table surgery on
  these same objects, and the precedent for how long a starved rollup goes
  unnoticed.
- `packages/prices-clickhouse/schema/rollups.sql` — the invariants in step 2 are
  documented at length in its header.
- `packages/prices-clickhouse/src/drift.rs` — why the check compares a
  fingerprint rather than the DDL text.
- `packages/prices-clickhouse/src/rollup_sql.rs` — the generator of every
  statement in `rollups.sql`, including `mv_modify_refresh` and the reconcile
  window, cadence and mismatch grace.
- [`0286-reingest-history.md`](0286-reingest-history.md) §1a — stopping the
  reconcile MVs during a re-ingest.
