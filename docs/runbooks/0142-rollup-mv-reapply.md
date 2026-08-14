# Runbook — changing a rollup MV body on a provisioned target (task 0142)

How to land an edit to `packages/prices-clickhouse/schema/rollups.sql` on a
cluster that already holds the six refreshable rollup MVs, without
reintroducing the task 0090/0095 data loss.

**Applies to:** `mv_ohlcv_1m_to_15m`, `_15m_to_1h`, `_1h_to_4h`, `_4h_to_1d`,
`_1d_to_1w`, `_1w_to_1M` on ch-prod-01.

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

## Step 1 — see what the target actually holds

```bash
cargo run -p prices-clickhouse --bin prices-clickhouse-drift
cargo run -p prices-clickhouse --bin prices-clickhouse-drift -- --verbose
```

Read-only: it issues `SELECT`s against `system.tables` and
`formatQuerySingleLine`, and creates, alters and drops nothing. Exit 0 means all
six are present, match the file and are in `APPEND` mode; exit 1 means at least
one needs attention.

Run this **before** you edit anything. A target that has already drifted is a
different job from landing a new edit, and doing both in one pass makes the
verification in step 5 unreadable.

Three findings it reports, in descending severity:

| Report                          | Meaning                                                                                                                                                                                             |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CRITICAL … NOT in APPEND mode` | Replace mode. Every refresh is atomically replacing the whole target table with just its bounded window — the task 0090 data loss, **happening now**. Fix this first and independently of any edit. |
| `MISSING`                       | Declared in the file, absent on the target. That tier is not rolling up (the task 0136 shape). A plain re-apply fixes it — `IF NOT EXISTS` creates what is not there.                               |
| `DRIFT`                         | Live definition and file disagree. Re-applying will NOT fix it; that is what the rest of this runbook is for.                                                                                       |

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
the cluster:

```bash
docker compose up -d clickhouse           # 26.3.10.60, the prod pin
cargo test -p prices-clickhouse --lib
cargo test -p prices-clickhouse --test rollup_drift_it   -- --ignored
cargo test -p prices-clickhouse --test rollup_append_it  -- --ignored
cargo test -p prices-clickhouse --test rollup_chain_it   -- --ignored
```

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
| `mv_ohlcv_1w_to_1M`  | 1 day           | 400 days | ample                                     |

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

For each MV, in **coarse-to-fine** order (`_1w_to_1M` first, `_1m_to_15m` last)
so that a tier is never fed by a source that is itself mid-change:

```sql
-- 1. drop
DROP VIEW prices.mv_ohlcv_1w_to_1M;

-- 2. re-create — paste the edited statement from schema/rollups.sql verbatim,
--    including `IF NOT EXISTS` (harmless: you just dropped it, and keeping the
--    file and the executed statement identical is what makes step 5 clean).
CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1w_to_1M
REFRESH EVERY 1 DAY APPEND
TO prices.price_ohlcv_1M AS
SELECT …;
```

Then confirm this one before touching the next:

```sql
SELECT view, status, last_success_time, next_refresh_time, exception
FROM system.view_refreshes
WHERE database = 'prices' AND view = 'mv_ohlcv_1w_to_1M';
```

`status = Scheduled`, a `last_success_time` at or after your `CREATE`, and an
empty `exception`. A non-empty `exception` means the MV exists but is failing on
every tick — visible here and nowhere else.

> **Applying the whole file is a valid way to re-create**, once the MV is
> dropped: `IF NOT EXISTS` creates what is absent. `cargo run -p
prices-clickhouse --bin prices-clickhouse-init -- --rollups` re-creates every
> _missing_ MV and leaves every existing one untouched. Convenient when several
> tiers are `MISSING`; it is **not** a way to land an edit on an MV that still
> exists.

## Step 5 — verify

```bash
cargo run -p prices-clickhouse --bin prices-clickhouse-drift
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

## Related

- `docs/runbooks/0136-coarse-rollup-merge-recovery.md` — per-table surgery on
  these same objects, and the precedent for how long a starved rollup goes
  unnoticed.
- `packages/prices-clickhouse/schema/rollups.sql` — the invariants in step 2 are
  documented at length in its header.
- `packages/prices-clickhouse/src/drift.rs` — why the check compares a
  fingerprint rather than the DDL text.
