---
id: "0136"
title: "Every coarse OHLCV table frozen since 2026-07-21 — merges and mutations inert on six tables"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0072", "0097", "0104", "0109", "0114", "0127"]
tags:
  [
    "priority-high",
    "effort-medium",
    "clickhouse",
    "production-incident",
    "milestone-M2",
    "data-freshness",
  ]
milestone: 2
links:
  - "../../../docs/runbooks/0072-current-prices-mv-rollout.md"
history:
  - date: 2026-07-30
    status: backlog
    who: okarcz
    note: >
      Found while verifying the 0072 MV rollout on ch-prod-01. `change_7d_pct`
      came back 0 for every asset, which traced to `price_ohlcv_1h` having no
      rows newer than 2026-07-21. All six coarse tables are frozen; only
      `price_ohlcv_1m` is live. Diagnosis is read-only and incomplete by
      choice — the remaining discriminating test is a state change on a shared
      production cluster and was deliberately not run.
  - date: 2026-07-30
    status: backlog
    who: okarcz
    note: >
      **Mechanism reproduced locally on CH 26.3.10.60.** `SYSTEM STOP MERGES` +
      `ALTER … DELETE` reproduces every prod observable — mutation `is_done=0`,
      `parts_to_do` non-zero, `latest_fail_time` 1970, empty `fail_reason`,
      parts accumulating while inserts still succeed, and `part_log` showing
      `NewPart` only. The control (same mutation, merges normal) completed in
      seconds with `MutatePart` events, so a pending mutation alone does NOT
      freeze a table — that alternative is eliminated. Also corrected a wrong
      reading recorded earlier: a fresh INSERT into a ReplacingMergeTree yields
      a **level-1** part (a plain MergeTree yields level 0), so prod's uniform
      level 1 across 5,000 parts means nothing has EVER been merged, not
      "merged exactly once". Proving prod is actually in the stopped state
      still requires `SYSTEM START MERGES` there — not run, by standing rule.
---

# Every coarse OHLCV table has been frozen for nine days

## Summary

`price_ohlcv_15m`, `_1h`, `_4h`, `_1d`, `_1w` and `_1M` have accepted no new
data since **2026-07-21 02:44**. `price_ohlcv_1m` is unaffected and current.

Nothing alarmed, because eight of the nine refreshable MVs report `status =
Scheduled` with an empty `exception` every cycle — rolling up stale input is not
an error. Only `mv_ohlcv_1m_to_15m` carries the failure, and everything
downstream of it dutifully reports success while starving.

## Impact

| Surface | Effect |
|---|---|
| `GET /assets/{id}/ohlcv` | every granularity above `1m` serves data ≥ 9 days stale |
| BE LP-analytics (0199 contract) | consumes the 1h/1d views — same staleness |
| `current_prices.change_7d_pct` ([[0072]]) | permanently 0; the column reads `_1h` and there is nothing in the 7-day window |
| [[0127]] M2 backfill-depth gate | reads `_1d`; any depth/coverage figure drawn from coarse tables is wrong |
| [[0088]] backfill | writes `_1m` (fine), but its coarse rollups are not landing |

## Evidence (all read-only, ch-prod-01, 2026-07-30 ~11:00Z)

**Freshness — one live table, six dead:**

```
price_ohlcv_1m   2026-07-30 11:02:00   <- live
price_ohlcv_15m  2026-07-21 02:30:00
price_ohlcv_1h   2026-07-21 02:00:00
price_ohlcv_4h   2026-07-21 00:00:00
price_ohlcv_1d   2026-07-21 00:00:00
price_ohlcv_1w   2026-07-20 00:00:00
price_ohlcv_1M   2026-07-01 00:00:00
```

**The one failing MV:**

```
mv_ohlcv_1m_to_15m
  last_success_time: 2026-07-21 02:44:00
  exception: Code: 252. Too many parts (5000 with average size of 867.73 KiB)
             in table 'prices.price_ohlcv_15m'. Merges are processing
             significantly slower than inserts
```

`TOO_MANY_PARTS` has fired **40,377** times and was still firing at 11:03Z.

**Parts are wedged in one partition, at exactly the throw limit:**

```
price_ohlcv_15m  partition 202607: 5000 parts  (parts_to_throw_insert = 5000)
                 partition 202606:   27 parts
```

Every active part is **level 1**, and partition `202606` was last touched
**2026-07-17 11:07:07**.

> ⚠️ Level 1 does **not** mean "merged once" here. A fresh `INSERT` into a
> **ReplacingMergeTree** produces a level-**1** part (insert-time dedup);
> a plain `MergeTree` produces level 0. Verified locally on 26.3.10.60 — see the
> reproduction below. So `min_level = max_level = 1` across all 5,000 parts means
> **not one of them has ever been merged**, which is stronger evidence for the
> freeze than the "merged exactly once" reading first recorded here.

**Six mutations created 2026-07-17 11:06:52, never executed:**

```
price_ohlcv_15m  parts_to_do  52   is_done 0   latest_fail_time 1970-01-01  fail_reason ''
price_ohlcv_1h              779   is_done 0   (same)
price_ohlcv_4h              431
price_ohlcv_1d              159
price_ohlcv_1w              120
price_ohlcv_1M               60
DELETE WHERE source = 'phoenix' AND timestamp >= …    (the 0097 Phoenix rework)
```

They are not failing. They have never been attempted in 13 days. Partition
`202606` stopped being touched **15 seconds after these were created**.

## Second-order effect — unmerged duplicate backlog (measured 2026-07-30)

Frozen merges mean `ReplacingMergeTree` has collapsed nothing since 07-17, so the
coarse tables hold heavy same-key duplication:

| scope | raw rows | distinct keys | blowup |
|---|---|---|---|
| `sdex`, partition **202607 only** | 11,654,728 | 1,786,458 | **6.52×** |
| `phoenix`, whole table | 124,930 | 40,799 | 3.06× |
| `aquarius`, whole table | 1,269,152 | 469,597 | 2.70× |
| `soroswap`, whole table | 179,302 | 86,442 | 2.07× |

**This is NOT a correctness problem, and it is NOT [[0097]]-specific.** A working
hypothesis that phoenix was double-counted because STAGE 0's delete never applied
was **tested and rejected**: `sdex` — untouched by 0097 — is the most duplicated
of all, and the AMM figures are table-wide (diluted by older merged partitions)
while sdex's is the frozen partition alone.

Every consumer reads with `FINAL` — the OHLCV API query
(`prices-api/src/assets/queries_ch.rs:578`, `FROM {table} FINAL`) and the
`views.sql` read surfaces BE uses in-cluster — so duplicates are collapsed at read
time and no wrong number has been served.

The cost is real though: ~9.9 M redundant rows in one sdex partition, and nine
days of `FINAL` running over 5,000 unmerged parts on every `/ohlcv` request and
every BE view query. Recovery reclaims both the space and the read latency.

## Hypotheses tested and rejected

Recording these so they are not re-run — each looked likely and each is dead:

1. **`15m` was dropped by the 0114/0130 work.** No — the table exists and held
   data until 07-21.
2. **Disk full.** No — 496 GiB free, 28.2% of 1.72 TiB, on both disks.
3. **Merge pool saturated by the [[0088]] backfill.** No — the cluster performs
   68k–285k merges/day (5–16 billion rows). Merges run constantly.
4. **Merges administratively stopped cluster-wide.** No — `price_ohlcv_1m` did
   5,220 merges in the last 24h. Whatever this is, it is scoped to these six
   tables.
5. **`number_of_free_entries_in_pool_to_execute_mutation` unsatisfiable.**
   No. It is 20, and the effective pool is `background_pool_size (16) ×
   background_merges_mutations_concurrency_ratio (2)` = **32**, so the config is
   legal. Sampled pool occupancy 20× over 60s: **18/20 samples showed 0 busy
   slots**, 2 showed 1. Mutations have been eligible essentially continuously.
6. **Write amplification across partitions** (the [[0132]] shape). No — the
   5,000 parts are in a single partition, not spread.

Also ruled out along the way: no detached parts, no merge errors in
`system.part_log` (`error != 0` returns nothing), and **no `part_log` activity of
any kind on `price_ohlcv_15m` in 24h** — ClickHouse is not touching the table.

## ⛔ TESTED ON PROD 2026-07-30 — `SYSTEM START MERGES` had NO EFFECT

The stopped-merges hypothesis was **falsified in production**. `SYSTEM START
MERGES prices.price_ohlcv_1M` was run (the leaf table, chosen as the smallest
blast radius). Six minutes later: parts still 750, `parts_to_do` still 60, and
`system.part_log` **completely empty** for that table. No state changed — the
command is a no-op where merges were never stopped — so nothing needed undoing.

The local reproduction remains valid: `SYSTEM STOP MERGES` *does* produce this
exact signature. It simply is not what is happening here. A matching signature
was never proof of a matching cause.

### What the trace log then showed — the actual finding

`text_log` is enabled and `logger.level = trace`, so scheduling decisions are
recorded. Over 30 minutes:

- **`price_ohlcv_15m` emits ZERO merge-machinery lines.** The only entries are
  `Create MergeTreeSink` (the MV's insert attempt) and `Code: 252 TOO_MANY_PARTS`
  (that insert failing), once a minute. No merge selection, no reservation
  attempt, nothing.
- **`price_ohlcv_1m` (the control) is loud** — block allocation, reservations,
  part renames, continuously.
- **The background pool is working**: `Background process (mutate/merge) peak
  memory usage` fires dozens of times a minute for other tables.
- Even **`price_ohlcv_15m_bak` has a live `CleanupThread`** scheduling itself.
  The backup copy has healthier background threads than the live table.

**The selector is not declining these tables — it is never being asked about
them.** A selector that ran and found nothing would log that at `trace`. Silence
means the storage's background operations assignee is not being scheduled at all.

That is a level *below* merge policy, which is why every knob checked came back
innocent: disk, pool, thresholds, table settings and the merge-stop flag are all
inputs to a decision that is never made. It is also why `SYSTEM START MERGES`
did nothing — it releases a lock on the decision, it does not restart a task that
is not running.

### Handover point

The background assignee is initialized at **table startup**, so the action that
reliably re-establishes it for every table is a **ClickHouse server restart** —
which would also clear in-memory locks of every kind in one step.

That is **BE's decision, not ours**: shared production cluster, their tables on
it, and precisely the class of action our standing no-risk-on-prod rule exists to
prevent drifting into. Uptime is 24 days; the freeze began 11 days in.

An `OPTIMIZE TABLE … PARTITION …` would further discriminate (forcing a merge
rather than waiting to be selected) but is a state change and was **not** run.

## Mechanism — reproduced locally, but NOT the cause (2026-07-30)

Merges and mutations are **administratively stopped on these six tables**
(`SYSTEM STOP MERGES <table>`), plausibly during the 07-17 operations and never
reversed. It halts merges *and* mutations, persists until
explicitly started or the server restarts, raises no error, logs nothing, and —
for non-replicated MergeTree — **is exposed in no system table**, which is why
every read-only diagnostic came back silent.

Reproduced on local docker ClickHouse **26.3.10.60** (the prod pin), on a
`ReplacingMergeTree` partitioned by month, seeded one part per `INSERT` to mimic
the 1-minute rollup MV appending forever.

**Test A — merges normal, then `ALTER … DELETE WHERE source = 'phoenix'`:**

```
mutation_id  is_done  parts_to_do  latest_fail_time      fail_reason
mutation_13        1            0  1970-01-01 00:00:00   (empty)
part_log: NewPart 12 | MutatePart 4 | MutatePartStart 4 | MergeParts 2
rows 800 -> 532 ; active_parts 12 -> 4
```

The mutation completes in seconds. **A pending mutation on its own does not
freeze anything** — this rules out the alternative mechanism.

**Test B — `SYSTEM STOP MERGES`, then the same `ALTER … DELETE`, then more
inserts:**

```
mutation_id  is_done  parts_to_do  latest_fail_time      fail_reason
mutation_13        0            4  1970-01-01 00:00:00   (empty)
part_log after the stop: NewPart only — no MergeParts, no MutatePart
active_parts 4 -> 16 (inserts keep working, nothing merges)
```

| Signal | ch-prod-01 | Test B |
|---|---|---|
| `is_done` | 0 | 0 |
| `latest_fail_time` | 1970-01-01 | 1970-01-01 |
| `latest_fail_reason` | empty | empty |
| mutation ever attempted | no | no |
| parts accumulate unboundedly | yes (5,000) | yes (4 → 16) |
| `part_log` merges/mutations after the freeze | none | none |
| inserts still succeed | yes, until the throw limit | yes |

Every observable matches, and Test A rules out the only competing explanation.

**What this does and does not establish.** It establishes that
`SYSTEM STOP MERGES` produces exactly ch-prod-01's signature, and that a pending
mutation alone does not. It does **not** prove that someone ran that command on
prod — that state is unreadable, so it can only be shown by
`SYSTEM START MERGES` taking effect. That is a state change on a **shared
production cluster** and was deliberately **not run**: the operator's standing
rule is that we do not take risks on production. It is recorded as the decision
for whoever owns the cluster, not as a pending action.

The timestamps remain the strongest circumstantial support: the mutations were
created at 11:06:52 and the last merge on `202606` landed at 11:07:07, fifteen
seconds apart, after which both stopped permanently.

### Provenance of the 07-17 window — it was OURS, and the trigger is unrecorded

Worth stating plainly, because the `_bak` tables look like evidence of an
unexplained third-party operation and are not:

- The six `price_ohlcv_*_bak` tables were created by **[[0095]]** on 2026-07-17
  as the restore path before the APPEND rollup-MV recreate. Already tracked for
  deletion by **[[0105]]**, which is now **blocked until this task is verified** —
  they are the only rollback for the recovery.
- The six Phoenix `ALTER … DELETE` mutations are **[[0097]]**'s
  `schema/preroll-amm-reprice.sql` **STAGE 0**, run on prod the same day.

So two of our own heavy operations ran minutes apart in that window. **But
`SYSTEM STOP MERGES` appears nowhere in the repository** — not in
`preroll-amm-reprice.sql`, not in 0095's steps, not in any runbook or script. The
operator does not recall running it. Most likely it was typed at the console
before a bulk delete (a reasonable thing to do) and never written down.

**The trigger is unresolved and does not gate recovery** — the fix is identical
either way. Do not spend time on attribution first. The lesson worth keeping is
narrower and actionable: an ad-hoc `SYSTEM STOP MERGES` is invisible afterwards,
so if one is ever issued it must be paired with its `START` in the same runbook
step.

Reproduction script: `scripts/repro-0136-merge-freeze.sh` (local docker CH only;
the `SYSTEM STOP MERGES` in it must never be pointed at ch-prod-01). Core of it:

```sql
CREATE TABLE t (timestamp DateTime, asset_id UInt64, source String,
                close Decimal(38,14), version UInt64)
ENGINE = ReplacingMergeTree(version)
PARTITION BY toYYYYMM(timestamp) ORDER BY (asset_id, timestamp, source);
-- 12 separate INSERTs == 12 parts, then:
SYSTEM STOP MERGES t;
ALTER TABLE t DELETE WHERE source = 'phoenix';
-- mutation never runs; parts accumulate; part_log shows NewPart only.
```

## Implementation

Full recovery plan: **`docs/runbooks/0136-coarse-rollup-merge-recovery.md`**.
Shape of it:

- **Step 0 — pre-flight snapshot** (read-only). Per-source row counts, `_bak`
  tables confirmed present, mutations still pending, uptime unchanged. This is
  the baseline that later distinguishes "the numbers moved" from "the numbers
  moved wrongly".
- **Step 1 — probe on `price_ohlcv_1M`**, not `_15m`. It is the leaf of the
  chain (nothing rolls up from it), the smallest by parts, and off the critical
  path — so a wrong hypothesis costs one small table's merges. **Hard decision
  gate:** if nothing happens within ~5 minutes the hypothesis is dead, stop, and
  record the negative result.
- **Step 2 — unblock the chain head** (`_15m`), watching parts fall well below
  `parts_to_throw_insert = 5000`. The freeze ends when `mv_ohlcv_1m_to_15m`
  reports a success with an empty exception.
- **Step 3 — the remaining four**, one at a time, pausing if the merge pool
  approaches saturation. This is a shared cluster and there is no deadline.
- **Step 4 — the 07-21 gap does NOT self-heal.** The rollup MVs read a bounded
  recent window; they will not reach back nine days. Closing the hole needs a
  bounded incremental pre-roll, and **never `preroll.sql`** — that expects
  TRUNCATE-d tables and would re-run the [[0090]] history-loss incident.
- **Step 5 — verify data, not just freshness.** Deep history against `_bak`,
  per-source deltas explainable by the Phoenix delete and RMT dedup.
- **Do not `KILL MUTATION`.** The six deletes are [[0097]]'s STAGE 0; killing
  them abandons a half-applied delete. If merges resume they drain on their own.
- **Expect row counts to change**, in both directions of surprise: merges
  collapse duplicate-PK rows the idempotent [[0097]] pre-roll wrote, and the
  pending delete removes `phoenix` rows in range. Both are intended.
- Re-verify `change_7d_pct` on [[0072]] afterwards; the column is correct and the
  data is not. It stays 0 until `_1h` holds a full 7 days again.

Detection is a separate deliverable → **[[0137]]**.

## Acceptance Criteria

- [~] Root cause established — **mechanism confirmed by local reproduction on
      26.3.10.60** (`SYSTEM STOP MERGES` reproduces every observable; a pending
      mutation alone does not). Still open: proving prod is in that state, which
      is only observable by starting merges there.
- [ ] All six coarse tables current again, `max(timestamp)` within one refresh
      cadence of now.
- [ ] `price_ohlcv_15m` accepting inserts; `TOO_MANY_PARTS` no longer firing.
- [ ] The six Phoenix mutations either completed or explicitly re-planned — not
      killed.
- [ ] The 2026-07-21 → recovery gap closed by a **bounded incremental** pre-roll
      (never `preroll.sql`), with deep history verified against `_bak`.
- [ ] A freshness alarm exists that would have caught this within a day →
      **[[0137]]**.
- [ ] [[0072]]'s `change_7d_pct` verified non-zero for assets with 7d of data.
- [ ] BE told that coarse `prices` data was stale and has moved — their 0199
      LP-analytics contract reads the 1h/1d views. **Not** framed as their
      operation: the 07-17 window was ours (see Provenance).
- [ ] [[0105]] unblocked only after the above holds for a watch period.

## Notes

- Discovered during 0072 step 3. The 0072 MV itself is **healthy** — 3,023 rows
  per refresh at 211 ms, `exception` empty — and writes the same volume as the
  v1 MV it replaced, so it neither caused nor worsens this.
- `price_ohlcv_1m` being the sole healthy table is the control case: it is the
  only one of the seven with **no pending mutation**.
