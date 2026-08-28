---
id: "0231"
title: "Nothing alarms when the oracle poll feed goes dark — 0227's guard turns silently-wrong rows into a silently-absent feed"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0227", "0226", "0086", "0199", "0056", "0167"]
tags: ["priority-medium", "effort-medium", "observability", "oracle", "infra", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/oracle-worker/src/lib.rs"
  - "../../../packages/enrichment-worker/src/metrics.rs"
  - "../../../infra/src/lib/stacks/observability-stack.ts"
history:
  - date: 2026-08-27
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0227]]'s own review. 0227's implementation plan called a
      rejected-and-**alarmed** timestamp "the single most valuable deliverable
      here"; what shipped rejects and logs. `run_oracle` still returns `Ok`, so
      the Lambda succeeds, and there is no metric filter or alarm on
      `oracle-worker`'s `skipped`, its ERROR logs, or `written == 0` anywhere in
      `observability-stack.ts` / `lambda-baseline.ts`. Recorded as owed rather
      than quietly dropped.
  - date: 2026-08-28
    status: active
    who: okarcz
    note: >
      Activated. Picked as the next real task off the backlog: the operator's
      only other open item is [[0220]]'s daily soak, and every other active
      task belongs to another developer. This is the honest completion of
      [[0227]], which shipped this morning — that guard rejects and logs, never
      rejects and alarms. Plan for the induction first: AC 5 (no false-fire on
      an idle environment) is the criterion that cost [[0204]] and [[0218]]
      their attended windows, and [[0222]]'s `FILL(m, 0)` sliding window is the
      shape to start from rather than rediscover.
---

# The guard is silent, and silence is what 0227 was about

## Summary

[[0227]] made the oracle worker refuse an implausible Reflector timestamp
instead of writing it. That is correct, and it is only half the requirement:
the rejection increments `skipped` and logs at `error`, but `run_oracle` returns
`Ok`, the Lambda reports success, and **nothing watches**.

## The failure this leaves open

Reflector changes `lastprice`'s unit again — to microseconds, say — or starts
returning `0`.

1. Every reading is refused. Correctly.
2. The poll feed goes **100% dark**.
3. `written = 0` every five minutes; the Lambda succeeds every time.
4. No alarm fires. It is discoverable only by someone going to look.

That is the same shape as the defect 0227 fixed: five months of silently wrong
rows becomes an indefinite silently **absent** feed. Trading one silence for
another is not the win the guard was meant to be.

⚠️ Note the interaction with [[0226]]: if that task establishes the poll write
is wholly redundant and removes it, this alarm's scope narrows to the event
path. Sequence accordingly — but do not let that possibility hold this open,
because "the feed might be deleted later" is not a reason to run it unwatched
now.

## Implementation

The pattern already exists and should be copied, not reinvented —
`enrichment-worker/src/metrics.rs` publishes to `Prices/Enrichment` behind the
`lambda` feature, with the mapping as a pure, unit-testable function and the
publish best-effort so CloudWatch trouble never fails the pass.

- Publish, per pass: `OracleRowsWritten`, `OracleRowsSkipped`, and a dedicated
  `OracleTimestampRejected` — the last one distinct from `skipped`, which
  already counts ordinary fetch failures and would bury a unit change in noise.
- Grant `cloudwatch:PutMetricData` narrowed by a `cloudwatch:namespace`
  condition, as the enrichment role already is.
- Alarm on **`OracleRowsWritten = 0`** sustained across several passes — the
  symptom that catches every cause, including ones not yet imagined — and a
  second, tighter alarm on `OracleTimestampRejected > 0`, which names the cause
  directly.
- ⚠️ `AWS/Lambda` publishes nothing when idle, and a `FILL(m,0)` alarm at `1/1`
  reads a partially-filled bucket as zero. Use the sliding-window +
  `FILL(m,0)` shape established by [[0111]]/[[0218]], and never evaluate at
  `1/1`.

⚠️ **Deploying Observability alone once left 10 of 13 alarms blind** (task
0204). Whatever ships here must be verified by *inducing* the condition, not by
reading a green deploy.

## Acceptance Criteria

- [ ] `OracleTimestampRejected` and `OracleRowsWritten` are published by a
      real pass and visible in CloudWatch.
- [ ] An alarm fires when the poll feed writes nothing across several
      consecutive passes — **verified by inducing it**, not by deploy status.
- [ ] An alarm fires on a rejected timestamp, with the raw value reachable from
      the alarm's description or the linked log.
- [ ] The metric mapping is a pure function with unit tests, per the enrichment
      precedent.
- [ ] Neither alarm false-fires on an idle environment.