---
id: "0176"
title: "GET /backfill/status publishes an impossible state — 'completed, 0%, 63.8M remaining' for SDEX and a 28-day-dead soroban_amm still reading 'running'"
type: BUG
status: completed
related_adr: []
related_tasks: ["0127", "0263", "0088", "0072", "0136"]
tags: [layer-api, priority-medium, effort-small, backfill, observability, consumer-facing]
links: []
history:
  - date: 2026-08-11
    status: backlog
    who: okarcz
    note: >
      Spawned from 0088's last open AC ("GET /backfill/status monotonic"). The
      AC could not be closed: measured on prod 2026-08-11, the endpoint reports
      sdex_archive as completed AND 0.0% AND 63,795,748 ledgers remaining, and
      soroban_amm as running with a last push 28 days old. Two independent root
      causes, one surface. 0088's own notes had already predicted the second.
  - date: 2026-09-04
    status: backlog
    who: okarcz
    note: >
      ⚠️ **Defect 1 is FIXED — but by [[0127]], not by this task, which nobody
      read first.** PR #283 (`d9de725`) corrected `progress_pct` and
      `ledgers_remaining` for the backward stream exactly as this task
      prescribes, including the instruction not to repair the data: the reader
      was changed, the stored `current_ledger = 1` was left alone. Found
      independently while closing Tranche 2's backfill gate, so the two were
      solved twice up to the point of discovery. **Defect 2 is untouched.**
      Re-scope before starting: what remains here is the dead-run status and
      the timestamp inconsistency, not the arithmetic.
  - date: 2026-09-08
    status: active
    who: okarcz
    note: >
      Promoted to active for the Milestone 2 pre-submission pass ([[0128]]).
      ⚠️ **Scope is Defect 2 only** — Defect 1 was fixed by [[0127]]/PR #283.
      What remains: a dead run still advertising `running`, and `completed_at`
      predating `last_push_at`. Operator decision 2026-09-08: **derive the
      stalled state in the reader and wire the missing alarm**, not a reaper
      that mutates the table (0176's own note forbids hand-patching the row).
      🔑 Found while scoping: `backfill-freshness-probe` publishes
      `PushAgeSeconds` for `soroban_amm` and has done for eight weeks, but the
      alarm at `observability-stack.ts:678` is pinned to
      `Stream: 'sdex_archive'` — nothing was watching. The probe's own
      exemption comment (`lib.rs:47-53`, "completes in a single push then
      transitions to `completed`") is the falsified assumption that created
      the gap.
  - date: 2026-09-08
    status: completed
    who: okarcz
    note: >
      **CLOSED — merged (PRs #296, #295, #297) and verified on production
      2026-09-08.** All five criteria met. 🔑 The diagnosis changed during the
      work: the AMM run did not die mid-flight, it FINISHED — `reached_tip` is
      unreachable because the combined run stops at the live handoff floor, so
      `running` was structural. Fixed at both ends: the writer rests a finished
      run at `paused`, the reader republishes a long-stale `running` as
      `stalled`. `realtime_tip_ledger` was 534,222 ledgers behind and now reads
      the live cursor. AMM freshness alarm wired, `OK` at 13:33:20 UTC.
      ⚠️ Two production incidents en route, both in Issues Encountered: a 46 s
      outage from a `Nullable` scalar subquery that `FORMAT TSVWithNames` could
      not reveal, and a near-rollback of the ledger-processor caught by
      timestamping artifacts. Both folded into the deploy runbook. ⚠️ This
      task's "series that has never existed" claim was wrong and is corrected
      here and in the M2 package.
---

# `/backfill/status` publishes a self-contradictory state

> ## 🗄️ Defect 1 is FIXED — 2026-09-04, PR #283 (`d9de725`)
>
> Corrected in `packages/prices-api/src/backfill/handlers.rs` exactly as this
> task prescribes. `progress_pct` is now `(target − current) / (target − start)`
> and `ledgers_remaining` is `current − start`; the completed archive reads
> **100% with 0 remaining** instead of 0% with 63,795,748. 🔑 This task's
> instruction was followed to the letter: **the reader was fixed and the stored
> `current_ledger = 1` was left untouched.** Five unit tests plus the amended
> integration test cover it.
>
> Two things this task did not anticipate, both now guarded:
> a non-`completed` stream is held under 100% (a genesis-anchored chunk writes
> `current_ledger = 1` while still `running`, which the corrected arithmetic
> would otherwise read as finished — that residue is [[0263]]), and
> `ledgers_remaining` is clamped to the span because `current > target` is
> reachable.
>
> ⚠️ **Defect 2 below is untouched and is the whole remaining scope**: a dead
> run that still advertises `running`, and `completed_at` predating
> `last_push_at`. AC 1 and AC 4 are met; AC 2, 3 and 5 are not.


## Summary

Two independent defects make the public `GET /v1/backfill/status` endpoint
report things that cannot all be true at once. Measured on prod 2026-08-11:

```
task_name     status     current_ledger  start_ledger  target_ledger  last_push_at         completed_at
sdex_archive  completed  1               1             63795749       2026-08-11 02:43:32  2026-07-27 21:24:26
soroban_amm   running    63352611        50457424      63475475       2026-07-14 17:54:24  NULL
```

## Defect 1 — writer and reader disagree on what `current_ledger` means

🔴 **The stored value is CORRECT. The endpoint's arithmetic is wrong.** Do not
"repair" the data; that would corrupt a right answer to hide a reader bug.

`resolve_current` (`packages/sdex-backfill/src/sink.rs:365-373`) has two modes:

```rust
Current::SetForward(v)  => existing.map_or(v, |e| e.max(v)),   // furthest FORWARD
Current::SetBackward(v) => …e.min(v)…                          // furthest BACK
```

The pre-Soroban tail walks **downward** toward genesis, so for that stream
`current_ledger` means *"furthest back reached"*. Pass 2 reached ledger 1, so
`current_ledger = 1` is exactly right and monotonic **in its own direction**.

But `progress_pct` (`packages/prices-api/src/backfill/handlers.rs:68-75`) and
`ledgers_remaining` (`handlers.rs:41`) both assume a forward position:

```
done              = current_ledger − start_ledger = 1 − 1          = 0  -> 0.0%
ledgers_remaining = target_ledger  − current_ledger = 63,795,749−1  = 63,795,748
```

So a fully completed backfill publishes **`completed`, 0.0%, 63,795,748
remaining**. A consumer cannot reconcile those, and `progress_pct` is the field
most likely to be rendered on a status page.

**This is a semantics bug, not a data bug.** The fix belongs in the read model —
either the DTO distinguishes forward from backward streams, or the row carries
its direction so the reader can compute the right thing. Note that
`sdex_archive` is a *single* stream that has been walked in **both** directions
(forward to the tip, backward to genesis), so "which direction is this stream"
may not be answerable from one field — that is the design question this task has
to settle.

## Defect 2 — nothing writes a terminal state when a run dies

`soroban_amm` has read `status: running` since **2026-07-14** with
`completed_at: NULL`. The run is long dead. `resolve_status`
(`sink.rs:381-388`) only transitions on a push from a live run, so a crashed or
killed run leaves its row asserting it is still working — **indefinitely**.

0088 already flagged this class:

> Same class of lying health signal as the sweep that reported nothing and the
> [[0136]] freeze.

Consumer impact: the API advertises an in-flight AMM backfill that stopped 28
days ago. Anything gating on `status != 'running'` waits forever.

Also inconsistent on `sdex_archive`: `completed_at` (2026-07-27, pass 1) is
**earlier** than `last_push_at` (2026-08-11, pass 2). A completion timestamp
that predates the last write is another signal a consumer cannot interpret.

## Implementation

- Decide the read-model contract for a stream walked in both directions, then
  fix `progress_pct` / `ledgers_remaining` to match. Consider publishing the
  covered *range* rather than a scalar position — `earliest_data_available` is
  already on the row and already correct (`2015-11-18` for `sdex_archive`).
- Give a stalled stream a way to stop claiming it is running. Options: a
  freshness rule derived from `last_push_at` (the freshness probe in
  `packages/backfill-freshness-probe/` already reads this table), an explicit
  terminal write on shutdown, or a reaper. ⚠️ A shutdown hook alone does not
  cover a hard kill or a power cut — 0088 saw both.
- Reconcile `completed_at` vs `last_push_at` so completion cannot predate the
  last write.

## Acceptance Criteria

- [x] `/backfill/status` cannot publish `completed` together with a non-100%
      `progress_pct`, in either walk direction. Assert it as a test, not by
      inspection.
- [x] A stream whose last push is far in the past does not report `running`.
      Fixed at BOTH ends: the writer now rests a finished run at `paused`
      (the root cause), and the reader republishes a long-stale `running` as
      `stalled` (the guard, for a run hard-killed before it can write a
      terminal state).
- [x] `completed_at >= last_push_at` holds, or the two fields are given
      meanings that make the ordering irrelevant and that is documented.
      **Second branch taken, documented in `dto.rs`:** `completed_at` belongs to
      the run that completed, `last_push_at` to the last write by *any* run, so
      on a stream walked in more than one pass the earlier completion beside the
      later push is correct, not a defect. Forcing the ordering would have
      destroyed true information.
- [x] `sdex_archive`'s stored `current_ledger = 1` is **preserved**, not
      overwritten — the fix is in the read path. A test should pin this so a
      later "cleanup" does not silently reintroduce the bug by mutating data.
- [x] Prod re-checked after the fix and the actual output recorded.

      ```json
      {
        "realtime_tip_ledger": 64332242,
        "sdex": { "status": "completed", "current_ledger": 1,
                  "progress_pct": 100.0, "ledgers_remaining": 0,
                  "earliest_data_available": "2015-11-18T03:47:00Z" },
        "soroban_amm": { "status": "paused", "completed_at": null,
                         "earliest_data_available": "2024-03-08T19:00:00Z" }
      }
      ```

      Was, before: `realtime_tip_ledger` 63,795,749 (28 days stale),
      `progress_pct` 0.0, `ledgers_remaining` 63,795,748,
      `soroban_amm.status` `running`, watermark `2024-02-20T17:00:00Z`.



## Implementation Notes

Shipped across three PRs, all merged and deployed 2026-09-08: **#296** (reader,
tip, alarm, ceiling doc), **#295** (the `paused` writer fix, riding with 0264),
**#297** (the hotfix below).

- `handlers.rs` — `effective_status()` republishes a long-stale `running` as
  `stalled` at 604,800 s, deliberately the same value as
  `opsAlarms.sdexPushFreshnessSeconds` so endpoint and alarm cannot disagree.
  `realtime_tip()` reads the chain tip from `ingest_cursor` instead of the
  backfill's frozen `target_ledger`.
- `queries_ch.rs` — push age computed **server-side** (clock-skew immune), live
  tip carried as a scalar subquery rather than a second round trip.
- `progress.rs` — a finished run short of the tip writes `paused`, gated on
  having indexed something.
- `observability-stack.ts` — `prices-{env}-amm-push-freshness` with its own
  tunable; reached `OK` at 13:33:20 UTC on deploy.

194 prices-api tests, 30 sdex-backfill tests, clippy clean.

## Issues Encountered

- 🔴 **The reader fix took `/backfill/status` down for 46 seconds** (15:17:47 →
  15:18:33 UTC, 2 failed requests). A ClickHouse **scalar subquery is `Nullable`
  regardless of what it selects**, so `(SELECT ifNull(max(ledger), 0) …)` typed
  as `Nullable(UInt64)` against a bare `u64` field; RowBinary drifted one byte
  and the *next* row failed on an unrelated column. Fixed with `assumeNotNull`
  (PR #297) — the pattern `backfill-freshness-probe` already documents.
  ⚠️ **The pre-deploy check could not have caught it:** it used
  `FORMAT TSVWithNames`, and text formats carry no null-flag byte. See
  [[clickhouse-scalar-subquery-is-always-nullable]].
- 🔴 **A Compute deploy nearly rolled the ledger-processor back two days.** That
  stack holds both Lambdas; only `prices-api` had been rebuilt, and the
  ledger-processor artifact on disk predated the deployed one. Its `S3Key`
  changed in the diff and read as an upgrade. Would have silently removed
  `ClickHouseWriteLatencyMs`, the metric behind a dashboard tile whose
  screenshots ship in the M2 package. Caught by timestamping the artifacts
  against the live functions; both traps folded into the runbook above.
- ⚠️ **A claim in this task and in the M2 package was wrong.** "The SDEX
  push-freshness alarm watches a series that has never existed" — CloudWatch
  shows 23 daily datapoints, 2026-07-05 to 2026-07-27, stopping when the stream
  completed. Corrected in `observability-stack.ts` and evidence §7.2/§8. The
  real gap is worse and now disclosed: `resolve_status` never downgrades a
  stored `completed`, so that alarm can never fire again and a **future** SDEX
  backfill would run uncovered.

## Design Decisions

### From Plan

1. **Derive in the reader, wire the alarm** — chosen over a reaper that mutates
   `backfill_progress`. This task forbids hand-patching the table: it repairs one
   row and leaves the mechanism intact.

### Emerged

2. **The diagnosis was wrong and was replaced.** The AMM run did not die
   mid-flight — it *finished*. `reached_tip` is unreachable in the documented
   operating model, because the combined run's `--end` is the live handoff floor,
   not the tip. Production's `current_ledger` of 63,352,611 matches the runbook's
   floor to the ledger, and the 122,864-ledger difference is the deliberate
   margin handed to live ingestion. So `running` was structural, not a crash.
3. **`paused` added at the writer as the root-cause fix**, mirroring what the
   `sdex_archive` row in the same function already did. The reader's `stalled`
   derivation was kept as the guard for a genuinely hard-killed run — the two
   compose, and `stalled` never touches `paused`.
4. **`paused` gated on having indexed something.** A completed run that advanced
   nothing is failing, not resting, and `paused` would silence the alarm on it.
5. **The alarm deploy gated on the data correction.** Shipping it against a
   `running` row would have paged permanently. Recorded in the code comment, not
   only in conversation.
6. **`realtime_tip_ledger` deliberately not used in `progress_pct`.** The archive
   completed against the tip as it stood; denominating a finished archive by a
   live tip would drag it below 100% further every ledger.

## Future Work

- [[0272]] — the SDEX freshness alarm can never fire again, so a future SDEX
  backfill would run without cover. Disclosed in M2 evidence §8.

# 📕 DEPLOY RUNBOOK

Written 2026-09-08. Covers 0176, 0263 and 0264 — they share one sequence.
Nothing below has been run; production is untouched.

## Step 0 — no `sdex-backfill` release exists to cut [nothing to do]

`sdex-backfill` is **not deployed anywhere** — no Lambda, no stack, no schedule.
It is an operator CLI built locally
(`cargo build --release -p sdex-backfill --features aws-mtls`) and run with
`--transport hetzner`. So [[0263]] and [[0264]]'s writer fixes need no ship.

Consequence: the ordering worry — "the write must come after the writer fix, or
the next run re-stamps it" — is already satisfied. The fix is on `develop`, and
any future run built from `develop` carries it. ⚠️ **The only real requirement is
that nobody runs a backfill from a binary built before 2026-09-08.**

## Step 1 — correct the stored `soroban_amm` row [prod CH, via `CHQ`]

Needs INSERT; `dev_read`/`chq` is read-only and cannot do this. Run from the
operator's shell. `backfill_progress` is `ReplacingMergeTree(updated_at)` keyed on
`task_name`, so this is an insert with a later `updated_at`, selected from the
existing row so no other column is retyped or lost.

```bash
CHQ <<'SQL'
INSERT INTO prices.backfill_progress
SELECT
    task_name,
    start_ledger,
    target_ledger,
    current_ledger,
    'paused'                                   AS status,
    last_push_at,
    toDateTime('2024-03-08 19:00:00')          AS earliest_data_available,
    newest_data_available,
    started_at,
    completed_at,
    now()                                      AS updated_at
FROM prices.backfill_progress FINAL
WHERE task_name = 'soroban_amm';
SQL
```

Two changes only: `status` `running` → `paused` (the run finished at the live
handoff floor — [[0176]]), and `earliest_data_available` `2024-02-20 17:00` →
`2024-03-08 19:00`, the first real AMM candle ([[0264]]).

**Verify:**

```bash
CHQ <<'SQL'
SELECT task_name, status, earliest_data_available, current_ledger
FROM prices.backfill_progress FINAL ORDER BY task_name;
SQL
```

Expect `soroban_amm | paused | 2024-03-08 19:00:00 | 63352611`.

⚠️ This must land **before** step 3, or the AMM freshness alarm goes straight to
ALARM and stays there.

## Step 2 — deploy the api-handler [local machine, repo root then `infra/`]

Closes PR #283's arithmetic (merged 2026-09-04, never deployed) plus 0176's
stalled-status and live-tip fixes. One deploy, three fixes.

```bash
# 2a — build BOTH Rust binaries FIRST. `build-production` does NOT do this.
cd ~/Projects/stellar/stellar-prices-api
git checkout develop && git pull --ff-only origin develop
cargo lambda build -p prices-api              --release --arm64 --features lambda
cargo lambda build -p prices-ledger-processor --release --arm64 --features lambda

# 2a-check — TIMESTAMP BOTH against the live functions. A changed S3Key in the
# diff says the asset DIFFERS; it does NOT say which direction.
ls -l --time-style=+'%F %T' target/lambda/prices-api/bootstrap \
                            target/lambda/prices-ledger-processor/bootstrap
aws lambda get-function-configuration --function-name prices-production-api-handler \
  --query LastModified --output text
aws lambda get-function-configuration --function-name prices-production-ledger-processor \
  --query LastModified --output text

# 2b — inspect the change before applying it
cd infra
make diff-production

# 2c — deploy (chains flush-production-cache, needed for the OpenAPI TTL)
make deploy-production-compute
```

🔴 **The Compute stack holds BOTH functions, so a deploy ships both.** On
2026-09-08 only `prices-api` had been rebuilt; the ledger-processor artifact on
disk was from 2026-09-02, **older than the 2026-09-04 deployment**. Its `S3Key`
changed in the diff and that read as an upgrade — it was a two-day **rollback**
that would have silently removed `ClickHouseWriteLatencyMs`, the metric behind a
dashboard tile whose screenshots ship in the Milestone 2 package. Caught by the
timestamp check above, not by the diff.

⚠️ **Verifying that a ClickHouse query runs is not verifying that it
deserialises.** The same deploy took `/backfill/status` down for 46 seconds: a
scalar subquery types as `Nullable` while the struct declared a bare `u64`, so
RowBinary drifted one byte and the *next* row failed on an unrelated column. The
pre-deploy check used `FORMAT TSVWithNames`, which carries no null-flag byte and
cannot show it. Type-check every projected expression with `toTypeName()` against
its struct field before shipping a query change — see
[[clickhouse-scalar-subquery-is-always-nullable]].

🔴 **2a is load-bearing.** `build-production` builds only the CDK TypeScript. The
Lambda is a pre-built asset at `../target/lambda/prices-api`; skip 2a and CDK
packages a stale bootstrap, the deploy goes green, and production keeps serving
the old code. That is task [[0141]] and it caused an outage once.

**Verify — the running process, not the file you built:**

```bash
curl -sS -H "x-api-key: $KEY" "$API/v1/backfill/status" | jq
```

| field | before | expect after |
|-------|--------|--------------|
| `realtime_tip_ledger` | 63795749 | ~64.3M and rising |
| `sdex.progress_pct` | 0.0 | 100.0 |
| `sdex.ledgers_remaining` | 63795748 | 0 |
| `soroban_amm.status` | running | paused (from step 1) |

## Step 3 — deploy the AMM freshness alarm [local machine, `infra/`]

**Only after step 1 is verified.**

```bash
cd ~/Projects/stellar/stellar-prices-api/infra
make diff-production                     # expect: 1 new alarm resource
make deploy-production-observability
```

Then confirm `prices-production-amm-push-freshness` is `OK`, not `ALARM`. If it
is in ALARM, step 1 did not take — re-read the row before doing anything else.

## Step 4 — close out [local machine]

- Re-read `/backfill/status` and paste the output into [[0128]]'s freshness
  checklist; §5 AC 5 quotes several of these values.
- Run the one-off reconciliation from [[0272]]: stored `earliest_data_available`
  vs `min(timestamp)` per source in `price_ohlcv_1d`. Confirms the correction
  took. 0272 itself stays backlog.
- Archive [[0263]] and [[0264]]; 0264 closes with its AC 4 deferred to [[0272]].


## Notes

- Found closing 0088's last AC. `backfill_status_maps_both_streams`
  (`packages/prices-api/tests/endpoints_it.rs`) passes and is not wrong — it
  covers stream *mapping*, not the arithmetic or staleness, so this defect sits
  in its blind spot.
- ⚠️ **Do not resolve this with a manual `UPDATE` on `prices.backfill_progress`.**
  Defect 1's data is correct, and hand-patching defect 2 fixes one row while
  leaving the mechanism that produced it intact — the next crash reproduces it.
- Related: [[0072]] shipped the `/price` surface and found a similar
  "measure on the right table" trap; [[0136]] is the canonical lying-health-
  signal precedent.
