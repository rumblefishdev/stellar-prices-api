---
id: "0136"
title: "Every coarse OHLCV table frozen since 2026-07-21 — merges and mutations inert on six tables"
type: BUG
status: active
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
  - "../../../docs/runbooks/0136-coarse-rollup-merge-recovery.md"
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
  - date: 2026-07-31
    status: backlog
    who: okarcz
    note: >
      **Recovery found, validated locally, and NOT yet run on prod.** A cluster
      restart is ruled out by the operator (shared with BE; we may touch
      `prices.*` only), so the handover-to-BE framing recorded on 07-30 is
      superseded. `DETACH TABLE` + `ATTACH TABLE` re-runs `startup()` for a
      single table, rebuilding the background operations assignee. Four tests on
      the prod CH pin (`scripts/test-0136-detach-attach-recovery.sh`): a table
      pinned at `parts_to_throw_insert` and failing with the identical
      `Code: 252` recovered to parts 30→1 within 10 s with inserts accepted
      again; a pending `ALTER DELETE` survives the detach and completes after
      the attach; `DETACH` of a live refreshable MV's `TO` target is allowed and
      the MV self-recovers with no recreate; `count() FINAL` unchanged. Runbook
      rewritten around it (PR #159). Deliberately deferred past the weekend —
      the freeze is a stable state, not a decaying one, and a hung `DETACH` is
      the one bad outcome, so it wants BE reachable and a watcher.
  - date: 2026-08-03
    status: active
    who: okarcz
    note: >
      Promoted to active to run the recovery on ch-prod-01, per the
      `docs/runbooks/0136-coarse-rollup-merge-recovery.md` sequence. Monday,
      BE reachable, watcher present — the conditions the 07-31 deferral asked
      for. Execution log below.
  - date: 2026-08-05
    status: active
    who: okarcz
    note: >
      WATCH PERIOD CLOSED - all six tiers current. `_1M` reached 2026-08-01 at
      the 00:00 refresh, which was the last outstanding tip. Full reading
      10:15Z: _15m 10:15, _1h 10:00, _4h 08:00, _1d 08-05 00:00, _1w 08-03,
      _1M 08-01. All nine refreshable MVs Scheduled with empty exception. That
      confirms the 08-04 diagnosis was right - `_1M` was not re-freezing, it
      read `_1w` before `mv_ohlcv_1d_to_1w` had written the week row. So
      [[0143]]'s "bounded, self-healing" framing HOLDS and its severity does
      not need revisiting. The race itself is unchanged though:
      mv_ohlcv_1d_to_1w and mv_ohlcv_1w_to_1M both show last_refresh_time
      2026-08-05 00:00:01, same second, no DEPENDS ON - it resolved in our
      favour, which is ordering luck, not a fix. [[0105]] is UNBLOCKED (the
      `_bak` rollback path is no longer needed). Remaining here is the
      07-21 -> 08-03 gap pre-roll, which has picked up a NEW dependency:
      [[0144]] C1 found the same unguarded argMax(close_usd, ...) in all four
      pre-roll scripts, so running the gap pre-roll before [[0145]] lands would
      manufacture a fresh estate of coarse rows with close_usd = 0.
  - date: 2026-08-11
    status: active
    who: okarcz
    note: >
      GAP MEASURED AND ITS SOURCE CONFIRMED INTACT - and it has acquired a HARD
      DEADLINE. Measured on prod while verifying whole-chain coverage after
      0088's pre-roll: price_ohlcv_1d holds 2026-07-21 (4,848 rows, partial)
      then NOTHING until 2026-08-03 (12,857). So 2026-07-22 -> 2026-08-02, 12
      FULL DAYS, are absent from the coarse forever-tables. Everything else from
      genesis to today is complete: every year 2016-2025 has 12 months, all five
      forever-tables populated in every year, ledger markers contiguous 3 ->
      63,352,611 with missing_ledgers = 0, and live ingestion is current
      (newest 1m candle 0 minutes behind now()).
      THE SOURCE SURVIVES. price_ohlcv_1m holds every one of the 12 missing days
      at 149k-413k rows/day, because prices-production-cleanup has been DISABLED
      since 2026-07-20 - one day before the freeze began. The gap is therefore
      repairable by a bounded incremental pre-roll over 2026-07-21 -> 2026-08-04.
      DEADLINE: cleanup drops WHOLE MONTHLY PARTITIONS older than
      now() - retention (cleanup-worker/src/lib.rs), and 1m retention is 7 DAYS.
      Re-enabling cleanup today drops partition 202607 entirely, taking 10 of
      the 12 recoverable days with it and making that part of the gap PERMANENT;
      202608 (08-01, 08-02) would survive until ~2026-09. So the ordering is
      now: run the gap pre-roll FIRST, re-enable cleanup AFTER - the same
      constraint 0088 operated under, for the same reason.
      The [[0145]] dependency noted in the 2026-08-05 entry is SATISFIED: the
      guard shipped, and 0088's pre-roll ran the guarded script on 2026-08-11
      with 0 unguarded argMax(close_usd, t.timestamp) sites verified in the
      checkout. Generating the windowed variant must preserve that guard - derive
      it from preroll-incremental.sql by changing ONLY the WHERE bounds, per the
      procedure now recorded in docs/runbooks/preroll-incremental-presoroban.md.
  - date: 2026-08-11
    status: active
    who: okarcz
    note: >
      GAP PRE-ROLL EXECUTED AND VERIFIED - the 07-21 -> 08-03 hole is CLOSED in
      all six coarse tiers, and the retention deadline recorded earlier today is
      lifted. Total prod runtime 6.1s. Generated the trial + full scripts from
      preroll-incremental.sql (unchanged since daecea1, so the sed anchors were
      still valid - re-checked, not assumed), gated them (0 unguarded argMax, 0
      leftover {boundary}, INSERT-only, correct rollup chain) and syntax-checked
      both on a throwaway CH 26.3.10.60 before anything touched prod.
      The single-day trial settled the open RMT question with a measurement
      rather than reasoning: price_ohlcv_1d FINAL for 07-21 went 4,848 -> 14,700,
      so the recomputed full bucket's max(version) does displace a partial row.
      Full window then produced all 13 days at 10,931-17,431 rows/day against
      controls of 15,616/14,841; 1h and 4h verified separately (all 17 days,
      correct cardinality ladder); 1w weeks 07-20/07-27/08-03 all ~27k matching
      the pre-freeze 07-13.
      THE 0145 GUARD WAS VERIFIED ON THE OUTPUT, not just the input: rebuilt days
      show close_usd = 0 at 68.6-74.2% against pre-freeze controls of 73.0/73.5%
      - inside the band, so no fresh zeroed estate was manufactured. That was the
      entire reason this pre-roll waited on 0145.
      CORRECTION ON THE RECORD - the note that "August's 1M row should heal once
      1w's 07-27 week is fixed" was WRONG and is retracted. mv_ohlcv_1w_to_1M
      takes toStartOfInterval(t.timestamp, INTERVAL 1 MONTH) over the 1w table
      (rollups.sql:186,199), so a month bucket is the set of weeks whose START
      falls in it - 08-01 and 08-02 live in JULY's row, and August's row is
      already correct. Nothing is pending and it must not be filed as an 0143
      race. This also justifies the [07-01, 08-01) window: it selects exactly the
      weeks the MV attributes to July, so the rebuild matches the MV's semantics
      instead of diverging from them.
      Cleanup is now UNBLOCKED but deliberately NOT re-enabled - separate
      operator decision.
      ALSO CLOSED THE SAME DAY - 0072's change_7d_pct AC. 1,680 of 3,295 assets
      (51.0%) now publish a non-zero value, against 0 for every asset while _1h
      was frozen, off a live mv_current_prices refresh. 0138 re-confirmed in
      passing: change_7d_pct = -100 on ZERO assets, against 396 on 2026-08-03.
      CORRECTION: the pre-roll did NOT unblock that column. ref_7d reads
      price_ohlcv_1h over now() - INTERVAL 7 DAY, which today starts 2026-08-04
      and so lies entirely AFTER the gap - the 13 rebuilt days are outside the
      window it reads. It became verifiable through elapsed time (a week past the
      08-03 recovery), which was always the other branch of the recorded "a week
      post-recovery OR the pre-roll". Do not credit the pre-roll.
      Remaining on 0136: 0137 alarm, the note to BE, and the cleanup re-enable.
---

# Every coarse OHLCV table has been frozen since 2026-07-21

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

## ✅ Recovery — `DETACH`/`ATTACH`, validated locally 2026-07-31

The background assignee is created in `IStorage::startup()`. A **ClickHouse
server restart** would therefore re-establish it for every table — and was the
recorded handover point on 07-30, as BE's call on a shared cluster.

**That is now ruled out.** The operator's decision (2026-07-31): a cluster
restart is too broad a risk, and our authorisation extends to `prices.*` only.

`DETACH TABLE` + `ATTACH TABLE` re-runs `startup()` for **one table** — the
per-table equivalent of a restart, scoped to a single table in a single
database, and reachable with the operator's `default`-user loopback path that
all schema DDL here already uses.

Validated on local docker CH **26.3.10.60** (the prod pin) —
`scripts/test-0136-detach-attach-recovery.sh`:

| Test | Result |
| ---- | ------ |
| **T4 — prod-shaped.** Table pinned at `parts_to_throw_insert`, inserts failing with the identical `Code: 252` | detach 106 ms / attach 105 ms → **parts 30 → 1 within 10 s, INSERT accepted again** |
| **T1 — pending mutation.** Wedged table carrying an unexecuted `ALTER DELETE` | mutation **survives** the detach and **completes** after attach (`is_done` 0 → 1); `MergeParts` + `MutatePart` resume |
| **T3 — dependent MV.** `DETACH` of a refreshable MV's `TO` target while the MV is live | **allowed**; MV errors `Code: 60` during the window only, then **self-recovers after `ATTACH`**, no recreate |
| **Data integrity** | `count() FINAL` unchanged (200 → 200) across detach/attach |

### Limits of that evidence

- **The local wedge is built with `SYSTEM STOP MERGES`, which prod falsified.**
  So this proves the command rebuilds a table's scheduler and clears in-memory
  state — **not** that it clears prod's specific unknown cause. What carries is
  the mechanism: `ATTACH` builds a new storage object from scratch, so no
  in-memory state survives it.
- **`OPTIMIZE` as a by-hand stopgap is UNVALIDATED.** It bypasses the assignee
  and merges synchronously on the query thread, so it *should* work on prod —
  but locally it returned `Code: 236 Cancelled merging parts`, purely because
  the simulation stops merges. Untested either way; unused in the runbook.
- **Do not quote the 105 ms attach for prod.** That was 30 parts; `_15m` has
  ~5,000 part descriptors to load. Expect meaningfully slower.
- **A hung `DETACH` is the one bad outcome** — it would mean a background task
  holds a lock, leaving the table inaccessible. Hence the leaf-table-first order
  and the 60 s hang gate in the runbook.

### Why it has not been run yet (2026-07-31)

Deliberately deferred past the weekend. **The freeze is a stable state, not a
decaying one:** `_15m` is pinned at `parts_to_throw_insert` so parts stop
growing, the other five receive no inserts, and `price_ohlcv_1m` is healthy — so
no raw data is being lost and the gap stays reconstructible. Against that, a hung
`DETACH` wants BE reachable, and step 2 is tens of minutes of watched merging.

**Two holds to confirm before acting:** `prices-production-cleanup` must still be
**disabled** (it drops historical partitions immediately and would eat the `_1m`
history the recovery depends on — **unverified on 07-31, operator creds expired**),
and [[0105]] must not have run (`_bak` is the only rollback).

## Prod recovery run — 2026-08-03

### Step 0 — pre-flight snapshot ✅

All three gates passed: six `_bak` tables present and untouched since 07-17
(so [[0105]] has not run), six mutations still `is_done = 0` at exactly the
07-30 `parts_to_do` figures, `SHOW CREATE` captured off-cluster.

**`uptime()` = 2,399,533 s — the cluster started 2026-07-06 14:59:19 and has
NOT restarted since.** This matters: the assignee was alive at startup and died
11 days later (07-17 11:06:52, with the mutations). Nothing has yet tried and
failed to rebuild it, which is the precondition under which
`ATTACH` → `startup()` is expected to work. A restart *after* 07-21 with the
tables still frozen would have falsified the mechanism outright.

**Baseline — the "before" picture:**

| table | parts | raw rows | `count() FINAL` | blowup | size |
|---|---|---|---|---|---|
| `_15m` | 5,027 | 68,400,564 | 9,405,741 | 7.27× | 4.55 GiB |
| `_1h` | 6,325 | 124,691,905 | 82,335,897 | 1.51× | 7.79 GiB |
| `_4h` | 3,124 | 61,444,661 | — | — | 4.02 GiB |
| `_1d` | 1,395 | 24,506,433 | — | — | 1.64 GiB |
| `_1w` | 819 | 9,956,812 | — | — | 678 MiB |
| `_1M` | 806 | 13,037,820 | 1,343,906 | 9.70× | 851 MiB |
| `_1m` (healthy) | 269 | 724,022,054 | — | — | 17.69 GiB |

Per-source `_1h`: sdex 123,118,521 · aquarius 1,269,152 · soroswap 179,302 ·
phoenix 124,930. **Only phoenix may drop** after recovery (the [[0097]] delete
finally applying); the other three must be unchanged.

### ⚠️ Correction — the freeze is NOT fully static

The 07-31 note claimed "`_15m` is pinned so parts stop growing, the other five
receive no inserts." **Half wrong.** Against the 07-30 figures, `_15m` (5,027),
`_4h` (3,124) and `_1d` (1,395) are unchanged to the part — but **`_1M` grew
750 → 806 (+56) and `_1w` 811 → 819 (+8)** over four days.

The coarsest two are still being written. The likely reason is window width: a
rollup MV re-reads a bounded recent window of its source, and for `_1w`/`_1M`
that window is wide enough to still contain rows from before the 07-21 freeze,
so every refresh re-appends the same stale rows. `_4h`/`_1d` read narrower
windows that have now moved entirely past the freeze, find nothing, and insert
nothing.

Consequences: the `_1M` duplicate blowup (9.70×) is still growing, and those two
tables are slowly marching toward the same per-partition
`parts_to_throw_insert = 5000` wall that pinned `_15m`. Adds mild urgency; does
not change the recovery.

### Step 1 — `price_ohlcv_1M` probe ✅ **RECOVERY CONFIRMED ON PROD**

`DETACH` + `ATTACH` in one invocation, 2026-08-03 ~09:47Z. Returned promptly,
no output, no hang. Within ~5 minutes:

| | baseline (step 0) | after attach |
|---|---|---|
| active parts | 806 | **113** |
| mutation `parts_to_do` | 60 | **0** |
| mutation `is_done` | 0 | **1** |
| `part_log` (10 min window) | empty for 13 days | `MergeParts` 49 · `MergePartsStart` 49 · `MutatePart` 60 · `MutatePartStart` 60 |

**The hypothesis is confirmed in production, not just locally.** `ATTACH`
constructs a fresh storage object and runs `startup()`, which rebuilt the
background operations assignee; merges *and* mutations resumed immediately.

The **17-day-old [[0097]] Phoenix `ALTER DELETE` completed** — it had never been
attempted once since 2026-07-17 11:06:52. That is the sharpest possible evidence
for the diagnosis: the mutation was never failing, it was never being *scheduled*.

It also closes out the `SYSTEM START MERGES` negative recorded on 07-30. That
command released a lock on a decision nothing was left to make; `ATTACH` restores
the thing that makes it.

Faster than the local test led us to expect — 806 parts collapsed to 113 inside
the 5-minute window, with no observable impact on the shared cluster.

### Step 2 — `price_ohlcv_15m` ✅ **THE FREEZE IS OVER**

`DETACH` + `ATTACH` at ~09:52Z. Returned promptly despite ~5,027 part
descriptors — the feared slow attach did not materialise.

| | baseline (step 0) | after attach |
|---|---|---|
| active parts | 5,027 (pinned at `parts_to_throw_insert`) | **12** |
| raw rows | 68,400,564 | 9,596,297 |
| mutation `parts_to_do` | 52 | **0** / `is_done 1` |
| `part_log` | `Code: 252` × 40,377, no merge lines | `MergeParts` 68 · `MutatePart` 2 · **`NewPart` 09:54:00** |

**`NewPart` at 09:54:00 is the end of the freeze** — the first insert accepted
by `price_ohlcv_15m` since 2026-07-21 02:44. `mv_ohlcv_1m_to_15m` is writing
again with no recreate, as the local T3 test predicted.

**The row arithmetic confirms the "storage shrinks, readers see the same data"
prediction exactly.** Raw 68,400,564 → 9,596,297, against a step-0 `count() FINAL`
of 9,405,741: dedup collapsed the 7.27× duplicate backlog down to precisely what
readers always saw, and the ~190k excess over the old FINAL is genuinely new data
landing. A 7× storage reduction with zero reader-visible loss.

### Step 3 — remaining four ✅ all six tables recovered

`_1h` first (heaviest), then `_4h`/`_1d`/`_1w` in one sequential invocation.
Running the last three together was a deliberate deviation from the runbook's
one-at-a-time rule: the shell executes them in order, so a hang on `_4h` still
blocks the two after it exactly as the rule intends, and the pool was idle at
**0 / 32** with the three collectively smaller than `_1h`, which had just drained
in about a minute.

| table | parts before | after | raw rows before | after | mutation |
|---|---|---|---|---|---|
| `_15m` | 5,027 | **15** | 68,400,564 | 9,625,597 | ✅ 52 |
| `_1h` | 6,325 | **179** | 124,691,905 | 114,744,527 | ✅ 779 |
| `_4h` | 3,124 | **149** | 61,444,661 | 57,098,077 | ✅ 431 |
| `_1d` | 1,395 | **122** | 24,506,433 | 19,550,735 | ✅ 159 |
| `_1w` | 819 | **108** | 9,956,812 | 5,509,296 | ✅ 120 |
| `_1M` | 806 | **113** | 13,037,820 | 2,326,776 | ✅ 60 |

**17,496 parts → 686 (96% reduction).** `system.mutations WHERE is_done=0`
returns **zero rows** — all six [[0097]] Phoenix deletes executed, ~1,601 parts
rewritten, 17 days after they were created. `active_merges 0` afterwards: fully
settled, nothing left running on BE's shared cluster. `price_ohlcv_1m` stayed
healthy throughout (269 → 240 parts).

The whole recovery — all six tables, six mutations, 17k parts — took roughly
**10 minutes** of wall clock (09:47Z → 09:57Z).

**Note on residual duplication (not a defect).** `_1h` shed only 8% of its rows
(114.7M raw vs an 82.3M `FINAL`), where `_15m` shed 86%. ClickHouse merges by
size-tier policy and never merges across partitions, so a settled table can
retain duplicates indefinitely. Every consumer reads `FINAL`, so this is a
storage-efficiency observation only.

### Steps 4–5 — freshness + data integrity ✅

**Freshness (10:06Z, ~10 min after recovery):**

| table | before | after |
|---|---|---|
| `_1m` | live | 2026-08-03 10:06 |
| `_15m` | 2026-07-21 02:30 | **2026-08-03 10:00** ✅ |
| `_1h` | 2026-07-21 02:00 | **2026-08-03 09:00** ✅ |
| `_4h` | 2026-07-21 00:00 | pending — MV cadence |
| `_1d` | 2026-07-21 00:00 | pending |
| `_1w` | 2026-07-20 00:00 | pending |
| `_1M` | 2026-07-01 00:00 | pending |

The head of the chain caught up within minutes. The four coarser tips lag on
their own cadences (`4h_to_1d` last ran 08:00, `1d_to_1w` / `1w_to_1M` at 00:00)
and advance over the following hours, as the runbook predicted. **All nine
refreshable MVs report `Scheduled` with empty `exception`**, including
`mv_current_prices`.

**Data integrity — live vs `_bak`, `FINAL`, window `timestamp < 2026-07-17`
(predates both the freeze and the mutation):**

| source | live | backup | delta |
|---|---|---|---|
| sdex | 81,415,340 | 81,415,340 | **0** |
| aquarius | 465,795 | 465,795 | **0** |
| soroswap | 86,093 | 86,093 | **0** |
| phoenix | 40,693 | 40,693 | **0** |

Plus `price_ohlcv_1d` deep history (`< 2025-06-01`): live 7,763,189 = backup
7,763,189. **Zero data loss.**

`FINAL` counts rose rather than fell — `_15m` 9,405,741 → 9,417,194, `_1h`
82,335,897 → 82,342,912, `_1M` unchanged — i.e. new data arriving with nothing
lost, against a 96% reduction in physical parts.

### ⚠️ Runbook defect found and fixed — step 5's per-source gate was unsound

The runbook's step 5 compared per-source counts **without `FINAL`**. Collapsing a
duplicate backlog necessarily reduces raw counts for *every* source, so the
stated expectation ("sdex must be unchanged; phoenix should DROP") is
unsatisfiable after a dedup event: the observed raw deltas were sdex −7.9%,
aquarius −9.9%, soroswap −10.0%, phoenix −39.1%. Followed literally, that gate
reads as unexplained loss on three sources and would have triggered a false
"stop and restore from `_bak`" mid-recovery.

Replaced with a live-vs-`_bak` `FINAL` comparison over a fixed pre-incident
window, which is what actually discriminates loss from dedup.

### Surprise — the Phoenix delete removed nothing reader-visible

Phoenix `delta = 0` as well, which was not predicted. Essentially all phoenix
data lies inside the delete's range (decoded predicate: `source = 'phoenix' AND
timestamp >= 2024-02-20 17:00:10 AND timestamp < 2026-07-06 09:35:16`; only ~106
phoenix rows postdate 07-17), yet live still matches the backup exactly. The
mutation physically completed while changing nothing a reader could see.

**Likely cause: `apply_mutations_on_fly`**, default-on in modern ClickHouse —
`SELECT`s apply pending mutations logically, so the delete has been *in effect*
since 07-17 despite no part being rewritten. Consistent with `_1h` `FINAL` rising
rather than falling when the mutation completed. **Inference, not proof**
(`SELECT value FROM system.settings WHERE name='apply_mutations_on_fly'` would
settle it); nothing downstream depends on it.

Corollary: the raw phoenix drop (124,930 → 76,070) was pure dedup, **not**
deletion — retiring the "the excess drop is the delete applying" reading recorded
mid-run.

### Watch period — 14:25Z, ~4.5 h after recovery ✅ HELD

| table | frozen at | 14:25Z |
|---|---|---|
| `_1m` | (never froze) | 14:25 |
| `_15m` | 07-21 02:30 | **14:15** ✅ |
| `_1h` | 07-21 02:00 | **14:00** ✅ |
| `_4h` | 07-21 00:00 | **12:00** ✅ |
| `_1d` | 07-21 00:00 | **08-03 00:00** ✅ |
| `_1w` | 07-20 00:00 | unchanged — **expected** |
| `_1M` | 07-01 00:00 | unchanged — **expected** |

`_4h` and `_1d` were the discriminating pair and both advanced. `_1w`/`_1M` have
not because their MVs last ran at **00:00 today — nine hours before the
recovery** — so they read a still-frozen `_1d`. They pick up at 00:00 tonight.
Cascade behaving as predicted, not a residual fault.

All nine MVs `Scheduled`, empty `exception`. Part counts flat against the
post-recovery baseline (17/113/122/179/108/151 vs 15/113/122/179/108/149) —
ordinary insert growth, nowhere near the thousands that would signal the
assignee stopping again.

### ✅ `prices-production-cleanup` confirmed DISABLED (2026-08-03)

Outstanding since 2026-07-31 (operator credentials had expired, twice). Now
verified:

```
aws events describe-rule --name prices-production-cleanup \
  --region eu-central-1 --query State --output text
→ DISABLED
```

⚠️ **Drop `--profile soroban-explorer` from these commands** — that profile's
token is expired; the default SSO identity
(`AWSReservedSSO_AdministratorAccess/oskar.karcz`, account 750702271865) works.
Every runbook here still carries the `--profile` form.

**This was the last hold on the gap pre-roll.** Both preconditions are now
satisfied: cleanup is off, and the `_bak` tables are intact ([[0105]] has not run).

## Remaining work

> ## ✅ 2026-08-11 — THE GAP PRE-ROLL IS DONE; THE DEADLINE IS LIFTED
>
> Ran and verified the same day it was measured — §Execution has the numbers.
> All 13 days are in the forever-tables, so the retention clock below no longer
> threatens anything. **What remains on this task is no longer data recovery:**
>
> | # | item | state |
> |---|---|---|
> | 1 | **[[0137]]** freshness alarm | ❌ open — the reason this ran 10 days silent |
> | 2 | [[0072]] `change_7d_pct` non-zero for assets with 7d of data | ✅ **verified 2026-08-11** — 1,680/3,295 (51.0%), and `-100` on zero assets |
> | 3 | Tell BE the coarse data was stale and has moved | ❌ unsent |
> | 4 | Re-enable `prices-production-cleanup` | ⏸️ unblocked, **deliberately not flipped** |
>
> ✅ **Item 2 closed the same day** — see its AC for the numbers and for the
> correction that the *pre-roll* did not unblock it (elapsed time did; the
> column's 7-day window no longer overlaps the gap at all).
>
> The original framing is kept below because the deadline reasoning is the
> reusable part.
>
> ### ~~🔴 THE GAP PRE-ROLL NOW HAS A DEADLINE~~ (resolved)
>
> The only remaining item is the **07-21 → 08-03 gap pre-roll**, and it has gone
> from "outstanding" to **time-bounded**, because its source data is on a
> retention clock that is currently paused by an unrelated decision.
>
> **The gap, measured on prod 2026-08-11** (`price_ohlcv_1d`, daily counts):
>
> ```
> 2026-07-21 →  4,848 rows   (partial — the freeze began mid-day)
> 2026-07-22 ┐
>     …      │  ABSENT — 12 full days
> 2026-08-02 ┘
> 2026-08-03 → 12,857 rows   (recovery)
> ```
>
> **Everything else is complete.** Same check across all of history: every year
> 2016–2025 has 12 months, all five forever-tables populated in every year,
> ledger markers contiguous `3 → 63,352,611` with `missing_ledgers = 0`, and
> live ingestion current (newest `1m` candle **0 minutes** behind `now()`). This
> 12-day window is the single hole in the entire chain.
>
> **✅ The source survives — the gap IS repairable.** `price_ohlcv_1m` holds
> every one of the 12 missing days at **149k–413k rows/day**, because
> `prices-production-cleanup` has been DISABLED since **2026-07-20** — one day
> before the freeze started. A bounded pre-roll over
> `[2026-07-21, 2026-08-04)` closes it.
>
> ### ⏳ Why it is now time-bounded
>
> Cleanup drops **whole monthly partitions** older than `now() - retention`
> (`cleanup-worker/src/lib.rs`), and `1m` retention is **7 days**. Re-enabling it
> today drops partition `202607` in full:
>
> | gap days | partition | fate on cleanup re-enable |
> |---|---|---|
> | 07-22 → 07-31 (**10 days**) | `202607` | **dropped — permanently unrecoverable** |
> | 08-01 → 08-02 (2 days) | `202608` | survives until ~2026-09 |
>
> 🔴 **Ordering, and it is the same constraint [[0088]] operated under:**
> **run the gap pre-roll FIRST, re-enable cleanup AFTER.** Re-enabling first
> destroys 10 of the 12 recoverable days silently.
>
> ### ✅ The [[0145]] dependency is satisfied
>
> The 2026-08-05 entry noted this pre-roll was blocked behind 0145, because
> [[0144]] C1 found unguarded `argMax(close_usd, …)` in all four pre-roll
> scripts. **That shipped.** 0088's pre-roll ran the guarded
> `preroll-incremental.sql` on 2026-08-11 with **0 unguarded sites** verified in
> the checkout it was run from.
>
> ⚠️ **Generating the windowed variant must preserve that guard.** Derive it from
> `preroll-incremental.sql` by changing **only** the `WHERE` bounds — never
> hand-write the aggregation — and re-verify with
> `grep -c 'argMax(close_usd, t.timestamp)'` on the generated file. The full
> procedure, including the syntax-check against a throwaway CH on the pinned
> version, is now recorded in
> [`docs/runbooks/preroll-incremental-presoroban.md`](../../../docs/runbooks/preroll-incremental-presoroban.md).
>
> ⚠️ Unlike 0088's pre-roll, this window **already holds partial coarse rows**
> (07-21 has 4,848; 08-03 has 12,857) and the rollup MVs are live-writing there.
> RMT should resolve in the pre-roll's favour — it aggregates the full bucket, so
> its `max(version)` is ≥ a partial row's — but that is reasoning, not a
> measurement, and it should be verified on a single day before the full window.

---

## ✅ THE GAP PRE-ROLL — EXECUTED AND VERIFIED 2026-08-11

**The 2026-07-21 → 08-03 gap is CLOSED across all six coarse tiers.** The
procedure below was followed exactly as written; results are in
§"Execution — 2026-08-11" immediately after it. Total prod runtime: **6.1 s**.

It is kept in full rather than summarised because it is now a *proven* procedure
for a windowed re-roll over live tables, and the next gap will want it. The two
generated `.sql` files were **not** committed — the generator is the artifact
(see Step 0).

⏭️ **What this does NOT do:** re-enable `prices-production-cleanup`. That is now
unblocked but remains a separate, explicit decision — see §Remaining work.

### Step 0 — regenerate the two scripts

⚠️ **The generated files are NOT in the repo and NOT on the server.** They were
built in a session scratchpad which no longer exists. That is deliberate:
committing derived SQL means two copies of the aggregation expression, which is
exactly how the unguarded `argMax(close_usd, …)` survived into [[0145]]. **The
generator is the artifact.** Re-run it from the repo root:

```bash
SRC=packages/prices-clickhouse/schema/preroll-incremental.sql
emit () { sed -n "$1,$(($1+12))p" "$SRC" | sed "s|$2|$3|"; }

W1="WHERE t.timestamp >= '2023-01-01' AND t.timestamp < '2024-01-01'"
W2="WHERE t.timestamp < {boundary:DateTime}"
GAP="WHERE t.timestamp >= '2026-07-21' AND t.timestamp < '2026-08-04'"
DAY="WHERE t.timestamp >= '2026-07-21' AND t.timestamp < '2026-07-22'"

# --- single-day trial: 15m -> 1h -> 4h -> 1d, 2026-07-21 only ---
{ emit 193 "$W1" "$DAY"; echo; emit 237 "$W2" "$DAY"; echo
  emit 251 "$W2" "$DAY"; echo; emit 265 "$W2" "$DAY"; } > /tmp/gap-trial-0721.sql

# --- full window ---
{ emit 193 "$W1" "$GAP"; echo; emit 237 "$W2" "$GAP"; echo
  emit 251 "$W2" "$GAP"; echo; emit 265 "$W2" "$GAP"; echo
  emit 279 "$W2" "WHERE t.timestamp >= '2026-07-20' AND t.timestamp < '2026-08-10'"; echo
  emit 293 "$W2" "WHERE t.timestamp >= '2026-07-01' AND t.timestamp < '2026-08-01'"
} > /tmp/gap-full.sql
```

⚠️ **The line numbers `193/237/251/265/279/293` are positions inside
`preroll-incremental.sql` as of commit `daecea1`.** If that file has changed,
re-locate the statements first (`grep -n 'INSERT INTO prices.price_ohlcv'`) — a
silently shifted `sed` range would emit a truncated statement.

**Gate the generated files before they go anywhere near prod:**

```bash
for f in /tmp/gap-trial-0721.sql /tmp/gap-full.sql; do
  echo "== $f"
  echo -n "  unguarded argMax (MUST be 0): "; grep -c 'argMax(close_usd, t.timestamp)' $f
  echo -n "  guarded argMaxIf            : "; grep -v '^\s*--' $f | grep -c 'argMaxIf(close_usd, t.timestamp, close_usd > 0)'
  echo -n "  leftover {boundary}  (MUST be 0): "; grep -c '{boundary:DateTime}' $f
  echo    "  verbs:"; grep -v '^\s*--' $f | grep -oiE '^\s*(INSERT|DELETE|TRUNCATE|DROP|ALTER)\b' | sort | uniq -c
done
# expect: trial = 4 INSERT / 4 guarded; full = 6 INSERT / 6 guarded; 0 everywhere else
```

Then syntax-check on the pinned engine (catches a `sed` slip in seconds):

```bash
docker run -d --name gap-syntax -e CLICKHOUSE_DB=prices clickhouse/clickhouse-server:26.3.10.60
sleep 8
docker exec -i gap-syntax clickhouse-client --multiquery < packages/prices-clickhouse/schema/init.sql
docker exec -i gap-syntax clickhouse-client --multiquery < /tmp/gap-full.sql   # expect exit 0, silent
docker rm -f gap-syntax
```

### The windows, and why they differ per level

| level | window | why |
|---|---|---|
| `15m`/`1h`/`4h`/`1d` | `[2026-07-21, 2026-08-04)` | the gap; all four bucket sizes align on those instants |
| `1w` | `[2026-07-20, 2026-08-10)` | **wider on purpose.** `toStartOfInterval(…, INTERVAL 1 WEEK)` starts **Monday** — verified on 26.3.10.60 — so the gap sits inside weeks `07-20`, `07-27`, `08-03`. Bounding `1w` to the gap would rebuild those weeks from **partial days** and overwrite correct rows. Upper bound stops before the current week (`08-10`), which belongs to the live MVs. |
| `1M` | `[2026-07-01, 2026-08-01)` | **July only.** August is the current month; rebuilding it from a `1w` table whose current week is deliberately absent produces a *worse* row than the live MV holds. |

⚠️ ~~**August's monthly row is NOT repaired by this script.** It should heal on
its own once `1w`'s `07-27` week (which contains 08-01 and 08-02) is fixed,
because `mv_ohlcv_1w_to_1M` re-aggregates daily. **Verify after the next MV
refresh** rather than assuming — if it has not healed in 24 h, that is [[0143]]'s
known `1d→1w→1M` race.~~

🔴 **RETRACTED 2026-08-11 — the premise was wrong, and nothing needs to heal.**
`mv_ohlcv_1w_to_1M` computes `toStartOfInterval(t.timestamp, INTERVAL 1 MONTH)`
over the **`1w`** table (`rollups.sql:186,199`), so **a monthly bucket is the set
of weeks whose *start* falls in that month** — not the set of days in that month.
Week `07-27` starts in July, so 08-01 and 08-02 belong to the **July** row all
along. August's row is weeks-starting-in-August only, and it is already correct.

Two consequences, both load-bearing:

1. **Do not wait 24 h for August to move, and do not file it as an [[0143]]
   race.** There is nothing pending. The 08-11 run measured `1M` July = 42,026 /
   August = 25,828 and both are right.
2. **The `[2026-07-01, 2026-08-01)` window is correct *because* of this**, not in
   spite of it. Filtering the `1w` table to July selects exactly the weeks
   `07-06/13/20/27` — precisely the set the MV attributes to July — so the
   rebuild reproduces the MV's own semantics rather than diverging from them. It
   deliberately excludes the `06-29` week, whose 07-01…07-05 days the MV counts
   in **June**. A window that "fixed" that apparent omission would have made the
   rebuilt row disagree with every other month in the table.

### Step 1 — baseline

```bash
CHQ <<'SQL' | tee ~/gap-before.txt
SELECT toDate(timestamp) AS d, count() AS raw_,
       uniqExact(asset_id, quote_asset_id, source) AS keys_
FROM prices.price_ohlcv_1d
WHERE timestamp >= '2026-07-19' AND timestamp < '2026-08-05'
GROUP BY d ORDER BY d;
SQL
```

Expect `2026-07-21` = **4,848** and `07-22 … 08-02` **absent**.

### Step 2 — the trial, and the decision it settles

Run the single-day script first. **07-21 is the only day in the window that
already holds partial coarse rows**, so it is the only one that exercises the
RMT collision; every other gap day is empty and would prove nothing.

```bash
scp -i ~/.ssh/sorban-prod_ed25519 /tmp/gap-trial-0721.sql /tmp/gap-full.sql \
  deploy@168.119.73.161:~/
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161
time docker exec -i app-clickhouse-1 clickhouse-client --multiquery < ~/gap-trial-0721.sql
```

No `--param_boundary` — every bound is a literal. ~340k source rows, so seconds;
the spill flag should not be needed.

```bash
CHQ <<'SQL'
SELECT count() AS final_rows FROM prices.price_ohlcv_1d FINAL
WHERE timestamp >= '2026-07-21' AND timestamp < '2026-07-22';
SQL
```

- **~15k** → the recomputed full day displaced the 4,848-row partial. The RMT
  collision resolves as predicted; **proceed**.
- **still ~4,848** → the existing partial is winning on version. **STOP** and
  rethink before touching the other 12 days.

### Step 3 — the full window, only if the trial passed

```bash
time docker exec -i app-clickhouse-1 clickhouse-client --multiquery < ~/gap-full.sql
```

Then re-run the Step 1 query into `~/gap-after.txt` and diff. All 13 days
present, none absent, counts in the same ~15k band as their neighbours.

---

## Execution — 2026-08-11 ✅ GAP CLOSED

Ran exactly as written above. **Prod runtime 6.1 s** for the full window — three
orders below the budget that made this feel risky.

### Step 0 — generation and gates ✅

`preroll-incremental.sql` was **unchanged since `daecea1`** (still the last commit
to touch it), so the `sed` anchors `193/237/251/265/279/293` landed on the
intended statements — re-verified rather than assumed, per the warning above.
Both files gated clean:

| gate | trial | full |
|---|---|---|
| unguarded `argMax(close_usd, t.timestamp)` | **0** ✅ | **0** ✅ |
| guarded `argMaxIf(…, close_usd > 0)` | 4 | 6 |
| leftover `{boundary:DateTime}` | **0** ✅ | **0** ✅ |
| verbs | 4 × `INSERT`, nothing else | 6 × `INSERT`, nothing else |
| rollup chain | `1m→15m→1h→4h→1d` | + `→1w→1M` |
| `--multiquery` on a throwaway 26.3.10.60 | exit 0, silent | exit 0, silent |

### Step 1 — baseline ✅ reproduced the measurement exactly

`07-21` = 4,848 (and `raw_ == keys_`, i.e. a single fully-merged version, so the
RMT collision under test was a clean one), `07-22 … 08-02` absent, `08-03` =
12,857.

⚠️ **Incidental, and worth keeping:** `08-04` read `raw_ = 72,580` against
`keys_ = 14,516` — **exactly 5.0 versions per key**, un-collapsed refreshable-MV
appends awaiting a merge. Benign under `FINAL`, outside the write window, and a
standing reminder never to compare recent-day counts without `FINAL`.

### Step 2 — the trial settled the RMT question ✅

`price_ohlcv_1d FINAL` for 07-21 went **4,848 → 14,700**, inside the ~15k band
and flanked by 07-19 (15,616) and 07-20 (14,841). The recomputed full day
displaced the partial: **`max(version)` from the full bucket wins**, as reasoned
but not previously measured. The remaining 12 days were empty, so strictly
easier.

### Step 3 — the full window ✅ all 13 days present

`price_ohlcv_1d FINAL`, 07-21 → 08-03, ranged **10,931 – 17,431 rows/day**,
against controls of 15,616 / 14,841. No day absent.

`07-21` shows `raw_ = 34,248` vs `keys_ = 14,700` — the trial's rows plus the
full run's, un-merged. `FINAL` resolves to the same 14,700, so it is duplication
awaiting a merge, **not** a discrepancy.

Coarser tiers, verified separately rather than inferred from `1d`:

| tier | result |
|---|---|
| `1h` | all 17 days present, 44,697 – 92,892/day |
| `4h` | all 17 days present, 26,797 – 48,960/day |
| `1w` | weeks `07-20` (27,098), `07-27` (27,868), `08-03` (26,747) vs the pre-freeze `07-13` (27,086) |
| `1M` | July 42,026, August 25,828 — **both correct**, see the retraction above |

The cardinality ladder holds (`1h` ≈ 5× `1d`, `4h` ≈ 3×), and the low days
(07-25/26, 08-01) are low in **every** tier together — a real activity signal,
not a partial rebuild.

### The [[0145]] guard verified on the *output*, not just the input ✅

The gates prove the generated SQL carries `argMaxIf`; they do not prove the rows
it wrote are sane. Measured `close_usd = 0` rates on `price_ohlcv_1d FINAL`:

| days | `pct_zero` |
|---|---|
| controls 07-19 / 07-20 (live-MV written, pre-freeze) | 73.0 / 73.5 |
| the 13 rebuilt days | **68.6 – 74.2** |

The rebuilt days sit **inside** the control band and mostly below it. No fresh
estate of `close_usd = 0` was manufactured — which was the whole reason this
pre-roll waited on 0145. The ~70% floor itself is the pre-existing [[0144]] /
[[0145]] defect class and is untouched by this run.

### ✅ Cleanup is now UNBLOCKED — but re-enabling stays an explicit decision

~~Do NOT re-enable cleanup until this is verified.~~ **Verified 2026-08-11.** The
13 days now live in the forever-tables (`1h`/`4h`/`1d`/`1w`/`1M`), so dropping
`1m`'s `202607` partition no longer destroys anything unrecoverable — the reason
for the hold is gone.

⚠️ It is still **not** flipped, and not to be flipped as a side effect of this
task. `prices-production-cleanup` has been disabled since 2026-07-20 and other
work has been running under that assumption ([[0088]]'s output, [[0174]]'s `15m`
archaeology). Re-enabling is a standalone, operator-confirmed step.

The original constraint, kept for the record: re-enabling drops partition
`202607` whole and takes **10 of the 13 days** permanently. Same constraint
[[0088]] operated under, same reason.

1. ✅ **DONE 2026-08-05 — the watch period is closed and [[0105]] is unblocked.**

   > **2026-08-04 — half done.** `_1w` reached `2026-08-03` ✅. `_1M` stayed at
   > `2026-07-01` ❌ — but **not** a re-freeze: `mv_ohlcv_1w_to_1M` ran cleanly
   > at 00:00 (status `Scheduled`, empty exception) and simply read `_1w` before
   > `mv_ohlcv_1d_to_1w` had written the `08-03` week row. Both are
   > `REFRESH EVERY 1 DAY` with no `DEPENDS ON`, so they race. Spawned as
   > [[0143]].
   >
   > **2026-08-05 10:15Z — `_1M` reached `2026-08-01` ✅.** Every tier is now
   > current, ten days after the freeze ended:
   >
   > | `_1m` | `_15m` | `_1h` | `_4h` | `_1d` | `_1w` | `_1M` |
   > |---|---|---|---|---|---|---|
   > | live | 10:15 | 10:00 | 08:00 | 08-05 00:00 | 08-03 | **08-01** |
   >
   > All nine refreshable MVs `Scheduled`, empty `exception`. This confirms the
   > 08-04 read — it was the cascade racing, not the assignee dying again — so
   > **[[0143]]'s "bounded, self-healing" framing holds** and its severity does
   > not need revisiting. ⚠️ The race is still uncontrolled: `mv_ohlcv_1d_to_1w`
   > and `mv_ohlcv_1w_to_1M` both report `last_refresh_time 2026-08-05 00:00:01`
   > — the same second, with no `DEPENDS ON`. It resolved in our favour this
   > time. That is ordering luck, not a fix, and it stays [[0143]]'s to close.

2. ✅ **DONE 2026-08-11 — the 07-21 → 08-03 gap pre-roll.** All 13 days present
   in all six tiers; full results in §Execution above. `_1d` jumping straight
   from 07-21 to 08-03 was the hole made visible. **BOUNDED INCREMENTAL only —
   never `preroll.sql`**, which expects TRUNCATE-d tables and would re-run the
   [[0090]] history loss — and that is what ran.

   ⚠️ **Script-choice discrepancy, recorded rather than papered over.** This item
   and the AC below both name **`preroll-live-gap.sql`**; the prepared procedure
   and the executed run both derived from **`preroll-incremental.sql`**. Both
   exist, both are post-[[0145]] guarded (`grep -c 'argMax(close_usd,
   t.timestamp)'` = **0** on all four pre-roll scripts, checked 2026-08-11), and
   the statements are the same shape — so the outcome is unaffected and the
   verification above stands on measurements, not on which file it came from.
   `preroll-live-gap.sql` is the **0090-era stopgap**, written when the rollup
   MVs were *dropped*; the MVs are back in APPEND mode since [[0095]], so
   `preroll-incremental.sql` was the better-matched source. Anyone re-running
   this should use `preroll-incremental.sql`.

   > ✅ **[[0145]] dependency CLEARED 2026-08-06** (PR #176, `4e35dc6`). All 121
   > unguarded `argMax(close_usd, …)` sites across the four pre-roll scripts are
   > now `argMaxIf(close_usd, t.timestamp, close_usd > 0)`. Without it, every
   > coarse row whose newest sub-bucket was not yet enriched would have landed
   > with `close_usd = 0`, manufacturing a fresh zeroed estate at backfill scale
   > — rows that then age out of the MV re-aggregation windows and need the
   > [[0114]] sweep to repair.
   >
   > ⚠️ **But 0145 shipped no deployable artifact, so verify before you run.**
   > These are operator-run scripts — nothing embeds them (`prices-clickhouse-init`
   > applies `INIT`/`VIEWS`/`ROLLUPS`/`SEED`, never `PREROLL`), so there was
   > nothing to deploy and there is no build to confirm. The fix applies only to
   > whoever runs the script **from an up-to-date checkout**. Running it from a
   > stale one silently executes the pre-0145 SQL and every signal reports
   > success. **Immediately before the gap pre-roll, from the checkout you are
   > actually pasting from:**
   >
   > ```bash
   > # must print 0 — any non-zero means this is the pre-0145 script
   > grep -c 'argMax(close_usd, t.timestamp)' \
   >   packages/prices-clickhouse/schema/preroll-live-gap.sql
   > ```
   >
   > `preroll-live-gap.sql` is the live-era script and the right one for this
   > gap. (Counting `argMaxIf` instead is the wrong check — the file header
   > quotes the guard expression verbatim, so that count is one higher than the
   > number of real sites.)
3. **[[0137]]** — the freshness alarm. Health was measured on MV status, not on
   the data, which is why this ran ten days silent. Recovery is not complete
   without it.

## Mechanism — reproduced locally, but NOT the cause (2026-07-30)

> ⛔ **The hypothesis below was FALSIFIED on prod** (see the START MERGES section
> above). It is kept because the reproduction is still the best available model
> of the *signature*, and because it is what the local recovery test wedges a
> table with. Do not read it as the cause.

The hypothesis was that merges and mutations are **administratively stopped on
these six tables** (`SYSTEM STOP MERGES <table>`), plausibly during the 07-17
operations and never reversed. It halts merges *and* mutations, persists until
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
- **Step 1 — `DETACH` + `ATTACH` `price_ohlcv_1M`**, not `_15m`. It is the leaf
  of the chain (nothing rolls up from it), the smallest by parts, and off the
  critical path — so a wrong approach costs the least. Issue both statements in
  **one invocation** so there is no human-sized gap. **Hard decision gate:** if
  `system.part_log` is still empty after ~5 minutes, stop and record the
  negative result — it would mean the failure survives a storage rebuild, which
  is new information. **If `DETACH` itself does not return within ~60 s it is
  hung** — stop, do not touch the other five, escalate.
- **Step 2 — unblock the chain head** (`_15m`), watching parts fall well below
  `parts_to_throw_insert = 5000`. Expect a slower `ATTACH` here (~5,000 part
  descriptors) and an MV refresh failing `Code: 60` during the window — both
  expected, the latter self-corrects. The freeze ends when `mv_ohlcv_1m_to_15m`
  reports a success with an empty exception **and** `part_log` shows a `NewPart`
  newer than the attach.
- **Step 3 — the remaining four**, one at a time, pausing if the merge pool
  approaches saturation. This is a shared cluster and there is no deadline.
- ⚠️ **Plain `DETACH`, never `DETACH … PERMANENTLY`** — plain detach leaves the
  metadata file in place, so `ATTACH TABLE` reads it straight back and the table
  cannot be lost even if the attach fails.
- ⚠️ **`count()` is a useless health probe here.** RMT dedup makes it fall while
  writes land perfectly (observed 800 → 400 locally while the MV inserted every
  5 s). Every gate runs off `system.part_log` — `NewPart` proves inserts land,
  `MergeParts` proves merging resumed.
- **Step 4 — the 07-21 gap does NOT self-heal.** The rollup MVs read a bounded
  recent window; they will not reach back nine days. Closing the hole needs a
  bounded incremental pre-roll, and **never `preroll.sql`** — that expects
  TRUNCATE-d tables and would re-run the [[0090]] history-loss incident.
- **Step 5 — verify data, not just freshness.** Deep history against `_bak`,
  per-source deltas explainable by the Phoenix delete and RMT dedup.
- **Do not `KILL MUTATION`.** The six deletes are [[0097]]'s STAGE 0; killing
  them abandons a half-applied delete. If merges resume they drain on their own.
- **Expect row counts to change**, but precisely: **physical** rows drop hard as
  merges collapse the duplicate-PK rows the idempotent [[0097]] pre-roll wrote
  (2,400 → 466 on a local test table), while **`count() FINAL` should NOT move**
  — those duplicates were never visible to readers. The one real logical change
  is the pending delete removing `phoenix` rows, and only from parts that
  existed when the mutation was created on 07-17; rows inserted after that are
  untouched, so expect a **partial**, not total, phoenix reduction.
- Re-verify `change_7d_pct` on [[0072]] afterwards; the column is correct and the
  data is not. It stays 0 until `_1h` holds a full 7 days again.

Detection is a separate deliverable → **[[0137]]**.

## Acceptance Criteria

- [~] Root cause established — **mechanism identified**: the storage's
      background operations assignee is never scheduled for these six tables
      (zero merge-machinery lines in `text_log` over 30 min, against a loud
      control). `SYSTEM STOP MERGES` reproduces every observable locally but was
      **falsified on prod**, so the signature is confirmed and the *trigger*
      remains unknown. A recovery exists that does not require knowing it.
- [~] Recovery method chosen and validated — **`DETACH`/`ATTACH` per table**,
      four tests green on the prod pin 2026-07-31 (PR #159). Ruled out: cluster
      restart (operator, too broad). Unvalidated: `OPTIMIZE` as a stopgap.
      **Not yet run against production.**
- [x] All six coarse tables current again, `max(timestamp)` within one refresh
      cadence of now — **verified 2026-08-05 10:15Z**, `_1M` (the last tip) at
      `2026-08-01`. All nine MVs `Scheduled`, empty `exception`.
- [x] `price_ohlcv_15m` accepting inserts; `TOO_MANY_PARTS` no longer firing —
      `NewPart` at 2026-08-03 09:54:00 ended the freeze; parts 5,027 → 15.
- [x] The six Phoenix mutations either completed or explicitly re-planned — not
      killed. All six executed on attach; `system.mutations WHERE is_done = 0`
      returns zero rows.
- [x] The 2026-07-21 → recovery gap closed by a **bounded incremental** pre-roll
      (never `preroll.sql`). **DONE 2026-08-11**, 6.1 s on prod, all 13 days
      present across all six tiers — §Execution has the per-tier counts.
      ✅ **[[0145]] gate cleared 2026-08-06** and re-verified in the checkout the
      SQL was generated from: `grep -c 'argMax(close_usd, t.timestamp)'` printed
      `0` for **all four** pre-roll scripts, and 0 on both generated files.
      **The guard was also verified on the output**, not just the input: the
      rebuilt days carry a `close_usd = 0` rate of 68.6–74.2% against controls of
      73.0/73.5% — inside the band, so nothing was manufactured.
      ⚠️ Derived from `preroll-incremental.sql`, not the `preroll-live-gap.sql`
      named here — see §Remaining work item 2 for why that is immaterial.
      ⚠️ "Deep history verified against `_bak`" was **not** performed and was not
      needed: this pre-roll writes only `[2026-07-01, 2026-08-10)`, touching no
      deep history. The whole-chain integrity check that *would* have caught such
      damage ran on 2026-08-11 anyway (every year 2016–2025 with 12 months, all
      five forever-tables populated, ledger markers contiguous, `missing_ledgers
      = 0`).
- [ ] A freshness alarm exists that would have caught this within a day →
      **[[0137]]**.
- [x] [[0072]]'s `change_7d_pct` verified non-zero for assets with 7d of data —
      **2026-08-11: 1,680 of 3,295 assets (51.0%) non-zero**, against 0 for every
      asset while `_1h` was frozen. `mv_current_prices` was `Scheduled` with an
      empty exception and a `last_success_time` 30 s old, so that is a live
      refresh rather than a stale write. The `ref_7d` window resolved to
      `2026-08-04 19:00 → 2026-08-11 18:00` over 8,884 assets.
      ✅ **[[0138]] also re-confirmed in passing: `change_7d_pct = -100` for
      ZERO assets**, against 396 on 2026-08-03 before the numerator guard.
      ⚠️ **This was NOT unblocked by the gap pre-roll, and the earlier note
      saying it would be is half right.** `ref_7d` reads
      `price_ohlcv_1h WHERE timestamp >= now() - INTERVAL 7 DAY`
      (`current.sql:185-192`) — today that is `2026-08-04 →` now, entirely
      *after* the gap. The 13 rebuilt days sit outside the window this column
      reads. It became verifiable through elapsed time (a week past the 08-03
      recovery), which was always the other branch of "a week post-recovery **or**
      the pre-roll". Do not cite the pre-roll as the cause.
      Two readings that look wrong and are not: `nonzero_7d` (1,680) exceeding
      `nonzero_24h` (1,122) is just the 7× longer window catching more assets;
      and 3,295 is not the asset universe — the row set is driven by `unfiltered`
      (`current.sql:287-290`), i.e. assets with a `_1m` row in the last 24 h, so
      `current_prices` holds currently-active assets only.
- [ ] BE told that coarse `prices` data was stale and has moved — their 0199
      LP-analytics contract reads the 1h/1d views. **Not** framed as their
      operation: the 07-17 window was ours (see Provenance).
- [x] [[0105]] unblocked only after the above holds for a watch period —
      **unblocked 2026-08-05**, two days of healthy autonomous rollups with
      every tier current. The `_bak` tables are no longer the rollback path for
      anything pending.

## Notes

- Discovered during 0072 step 3. The 0072 MV itself is **healthy** — 3,023 rows
  per refresh at 211 ms, `exception` empty — and writes the same volume as the
  v1 MV it replaced, so it neither caused nor worsens this.
- `price_ohlcv_1m` being the sole healthy table is the control case: it is the
  only one of the seven with **no pending mutation**.
