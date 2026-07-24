---
id: "0109"
title: "Preflight guard — sdex-backfill must refuse to start while prices-production-cleanup is ENABLED"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0088", "0090", "0108"]
tags: [layer-ops, priority-high, effort-small, backfill, safety, data-loss, guard]
links:
  - "../../../docs/runbooks/running-ingestion-components.md"
  - "../../../docs/runbooks/preroll-incremental-presoroban.md"
history:
  - date: 2026-07-20
    status: backlog
    who: okarcz
    note: >
      Spawned from 0088's cleanup-incident follow-ups during the 0108 grooming
      sweep. Named there as "the fix that actually prevents recurrence" but never
      made a task, which is the same failure mode it is meant to fix — a
      precondition living only in prose.
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Mechanism correction from the 0114 Phase C session, and it changes the
      design. Cleanup does NOT drop partitions - it deletes by ALTER DELETE
      mutation. system.part_log covers 2026-06-22 to now with 16.4M events and
      ZERO DropPart entries, while system.mutations holds the 2026-07-15
      destructive event verbatim (DELETE WHERE intDiv(toUInt64(version),1000) <
      50457424 on price_ohlcv_1m - every pre-Soroban row, i.e. the backfill's
      output). Fits the access model: prices_writer has ALTER DELETE but cannot
      drop partitions. So a DropPart-based guard would never fire. Added a
      second mechanism-level check on system.mutations for pending DELETEs, with
      the caveat that a starved mutation (six Phoenix deletes pending 07-17 to
      07-23+, empty latest_fail_reason) is armed, not safe.
---

# Preflight guard — refuse to backfill while the cleanup rule is ENABLED

## Summary

Make `sdex-backfill` check the `prices-production-cleanup` EventBridge rule at
startup and **refuse to run** (or at minimum warn loudly and require an explicit
override flag) while it is `ENABLED`. Running a historical backfill against an
enabled cleanup rule is not a degraded mode — it is silent, unrecoverable data
destruction.

## Context

Running both at once deletes the backfill's output **as fast as it is written**:
cleanup removes historical rows immediately rather than honouring the 7-day
retention intent, so a multi-day run produces nothing durable and re-creates the
exact history gap the run existed to close. Recovery is only possible by
re-downloading the span.

> **🔴 Mechanism correction (2026-07-23, from the [[0114]] Phase C session) — this
> changes what the guard must watch.** This task previously said cleanup "drops
> whole historical partitions". It does **not**. Cleanup deletes by
> **`ALTER … DELETE` mutation**. Evidence from prod CH:
>
> - `system.part_log` covers 2026-06-22 → now (16.4M events) and contains **zero
>   `DropPart` events** — the event-type set is `RemovePart`, `MergeParts`,
>   `MutatePart`, `NewPart` and the `*Start` variants. (`RemovePart` is merge
>   housekeeping, *not* deletion — it fires when source parts are retired after a
>   merge. Do not treat it as a deletion signal.)
> - The destructive 2026-07-15 event is recorded in `system.mutations` verbatim:
>   ```sql
>   -- price_ohlcv_1m, mutation_2496036, 2026-07-15 10:24:36, is_done 1
>   DELETE WHERE intDiv(toUInt64(version), 1000) < 50457424
>   ```
>   `version` encodes ledger sequence, so this removed every pre-Soroban row —
>   exactly the running backfill's output.
> - It fits the access model: `prices_writer` holds `SELECT, INSERT, ALTER
>   DELETE, OPTIMIZE` and **cannot** drop partitions (the same grant wall that
>   blocked `FREEZE` in 0114). Deleting by mutation is the *only* path available
>   to it.
>
> **A guard built around partition-drop detection would never fire.**

This cost **~5 days of run time on 2026-07-20**: the pre-Soroban SDEX tail's
output for ledgers `1 → ~21.4M` was wiped, forcing a second ~5-day pass over
`[1, 23423999]` (0088 §Recovery plan).

The precondition is currently documented only in runbook prose. It demonstrably
did not survive the gap between the 0090 re-run (which correctly disabled the
rule, then re-enabled it on completion) and the 2026-07-15 tail start (which did
not re-check). A prose precondition that has already failed once in production
should become a machine-checked one.

Memory: `[[cleanup-rule-shreds-backfill-output]]`.

## Implementation

- At `sdex-backfill` startup (and the combined/events-backfill entrypoints that
  write historical partitions), describe the rule and fail fast when enabled:
  ```
  aws events describe-rule --name prices-production-cleanup \
    --region eu-central-1 --query 'State'
  ```
- Prefer the AWS SDK over shelling out, so the check works the same in CI and on
  an operator box. It needs only `events:DescribeRule`.
- **Add a second, mechanism-level check against CH itself** — the rule state is
  necessary but not sufficient (a mutation already submitted keeps deleting no
  matter what the rule says, and a manual `ALTER … DELETE` bypasses the rule
  entirely). Query `system.mutations` for pending destructive work:
  ```sql
  SELECT table, mutation_id, command, create_time, is_done, parts_to_do
  FROM system.mutations
  WHERE database = 'prices' AND is_done = 0 AND command LIKE '%DELETE%'
  ```
  Treat any row as **armed and pending**, not safe. Note mutations can sit
  starved for days when the merge pool is saturated — six Phoenix deletes were
  pending from 2026-07-17 through at least 07-23 with empty `latest_fail_reason`
  — so "not progressing" must not be read as "not going to happen".
- **Never key the guard on `DropPart`** (see the mechanism correction above); it
  does not occur.
- **Fail closed, but not un-runnable**: exit non-zero with a message naming the
  rule, the risk, and the exact `aws events disable-rule` remediation. Gate the
  bypass behind an explicit `--allow-cleanup-enabled` flag so overriding is a
  deliberate, auditable act rather than a default.
- **Decide how to handle an indeterminate check.** If the describe call fails
  (no credentials, wrong profile, region drift, offline), the guard cannot tell
  enabled from disabled. Failing closed on an unknown result would block every
  local/offline run; failing open re-opens the exact hole. Suggest: warn loudly
  and require the explicit flag, i.e. treat unknown as "not proven safe".
- Only guard the **historical/backfill** write paths. Live ingestion writes to
  the recent window that cleanup is legitimately meant to retain, so it must not
  be gated on this.
- Cross-reference the guard from `running-ingestion-components.md` and
  `preroll-incremental-presoroban.md` so the prose and the code agree.

## Acceptance Criteria

- [ ] `sdex-backfill` refuses to start when `prices-production-cleanup` is
      ENABLED, with a message naming the rule and the remediation command.
- [ ] An explicit override flag exists and is required for the bypass; using it
      is logged.
- [ ] Indeterminate-check behaviour decided, implemented, and documented.
- [ ] Live ingestion is unaffected by the guard.
- [ ] Runbooks reference the guard instead of relying on prose preconditions
      alone.
- [ ] Test coverage for all three states: enabled → refuse, disabled → proceed,
      indeterminate → chosen behaviour.

## Out of scope

- Changing what the cleanup rule itself does, or its retention window.
- The 0088 recovery run — this guard is for the *next* backfill, and must not
  block the recovery currently in flight.
