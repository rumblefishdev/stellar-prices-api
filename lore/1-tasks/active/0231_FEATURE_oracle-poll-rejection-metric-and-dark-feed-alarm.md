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
  - date: 2026-08-28
    status: active
    who: okarcz
    note: >
      Worker + infra implemented, nothing deployed. `Prices/Oracle` metrics
      published by a pure, unit-tested mapping; `PutMetricData` granted with a
      namespace condition; `-oracle-dark-feed` (FILL, 3 x 10 min = 30 min) and
      `-oracle-timestamp-rejected` (1/1) added, plus the worker joins the 0112
      health list so it finally has `-no-invocations`. Operator decisions this
      session: `timestamp_rejected` stays a SUBSET of `skipped`; the dark window
      is 30 minutes, not 15; missing data on `-dark-feed` stays `BREACHING`.
      Induction plan written before the deploy, deliberately - ACs 1/2/3/5 are
      all still owed and only prod can close them.
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
---

## Implementation notes (2026-08-28)

Two commits on `feat/0231_oracle-poll-rejection-metric-and-dark-feed-alarm`.

### Worker — `packages/oracle-worker/`

`src/metrics.rs` (new), copied in shape from `enrichment-worker/src/metrics.rs`
rather than reinvented: a pure `pass_metrics` / `failure_metrics` mapping
compiled into every build and unit-tested without the AWS SDK, plus a
best-effort `publish` behind the `lambda` feature. Namespace `Prices/Oracle`.

Seven metrics: `OracleRuns`, `OracleFailedRuns`, `OracleSymbolsQueried`,
`OracleRowsWritten`, `OracleRowsSkipped`, `OracleTimestampRejected`,
`OracleUsdRatesSnapshotted`.

`OracleStats` gains `timestamp_rejected`, incremented alongside `skipped` at the
0227 guard's rejection branch. Also threaded into the log line and the Lambda's
JSON response.

### Infra

- `eventbridge-stack.ts` — `PutMetricData` on the oracle role, `*` scoped by a
  `cloudwatch:namespace` condition to `Prices/Oracle`.
- `observability-stack.ts` — `-oracle-dark-feed` (FILL, 3 x 10 min),
  `-oracle-timestamp-rejected` (raw, 1/1), and the oracle worker added to the
  0112 worker-health list, which gives it `-duration-near-timeout` and
  `-no-invocations` as well.

## Design decisions

### From plan

1. **`OracleTimestampRejected` is its own series, not read off
   `OracleRowsSkipped`.** A Reflector unit change reaches the skip total as a
   handful of extra skips among ordinary fetch failures and looks like nothing;
   on its own series it is a step off a flat zero.
2. **Two alarms, symptom and cause.** `-dark-feed` catches every cause including
   ones not yet imagined; `-timestamp-rejected` fires sooner and says which one.
   Neither replaces the other.
3. **`FILL(m, 0)` sliding window, never evaluated at `1/1`,** per [[0222]].

### Emerged

4. **`timestamp_rejected` is a SUBSET of `skipped`, not a disjoint bucket.**
   Confirmed with the operator. `skipped` already ships in the worker's log line
   and its Lambda response; narrowing it there would silently change the meaning
   of a number already in use, to save a count that is published separately
   anyway.
5. **The handler publishes on the `Err` path too** (`failure_metrics`). This is
   [[0218]]'s lesson applied before it could cost a second incident: `Ok`-only
   publishing makes a failed run indistinguishable from a run that never
   happened, because both emit nothing.
6. **No pass-duration metric.** The oracle Lambda does one thing per invocation,
   so `AWS/Lambda` `Duration` already measures the pass with nothing else folded
   in — unlike enrichment, where three stages share an invocation and splitting
   them is the point.
7. **The oracle worker joins the [[0112]] worker-health list.** It only now
   qualifies by that list's own criterion: until it published `Prices/Oracle` it
   had no custom-metric alarm that could go dark. Beyond the literal ACs, but it
   is what makes "the schedule is off" distinguishable from "Reflector changed".
8. **A synth-time guard on the poll cadence.** The 10-minute bucket assumes each
   bucket spans a scheduled pass. If `scheduleExpressions.oracleWatcher` is a
   `rate()` slower than the bucket, empty buckets become normal and the alarm
   false-fires forever — worse than no alarm, because it trains people to ignore
   it. The stack now throws at synth instead. A `cron()` schedule passes through
   on the documented assumption; a general cron parser was not worth the risk in
   a project with no infra test harness.
9. **30-minute dark window (3 x 10 min), chosen with the operator.** Production
   polls every 5 minutes, so a 10-minute bucket holds ~2 passes and cannot be
   emptied by schedule jitter. Slower to fire than 15 minutes, and this is
   5-minute-granularity non-critical data — half an hour dark is not much lost,
   and a transient Reflector or RPC blip must not page anyone.

## Induction plan — write this BEFORE deploying

⚠️ [[0204]] and [[0218]] both show that deploying an alarm and *proving it
fires* are different jobs, and the second is where the time goes. AC 5 is
designed for here rather than discovered later.

### `-oracle-timestamp-rejected` — induce on the real series

One synthetic datapoint on the real metric and dimension:

```
aws cloudwatch put-metric-data --namespace Prices/Oracle \
  --metric-name OracleTimestampRejected \
  --dimensions Environment=production --value 1 --unit Count
```

Proves namespace + dimension + threshold + SNS route end to end. Expect ALARM
within ~5-10 min, then OK on the next period. Cost: one explainable `1` in the
metric history, recorded here. **Mutating AWS call — needs per-session
approval.**

### `-oracle-dark-feed` — induce on a throwaway clone, NOT the real series

The real alarm cannot be induced by *adding* data: the live worker publishes a
non-zero `OracleRowsWritten` every 5 minutes, so no bucket goes dark while it
runs. Inducing it for real would mean stopping the feed for 30 minutes.

So clone the shape onto a scratch metric instead — `OracleRowsWrittenProbe`
under the same namespace, same `FILL(m, 0)` + 3 x 10 min + `LESS_THAN 1`
geometry — publish one datapoint, stop, and let FILL extend past it. That tests
the part actually in doubt: [[0222]]'s whole history is this shape failing to
fire. Delete the clone afterwards.

Then separately confirm the real alarm is bound to the real series: after
deploy it must sit in **OK with datapoints**, never `INSUFFICIENT_DATA`. That is
the check that would have caught 0204's 10-of-13 blind alarms.

### AC 5 — idle-environment behaviour, and one open question

- `-timestamp-rejected`: `NOT_BREACHING`, and the metric is only published by a
  completed pass. An idle env stays OK. Settled.
- `-dark-feed`: missing data is `BREACHING`. A worker that has never written a
  row is genuinely dark, so this is right for a live env.

✅ **DECIDED 2026-08-28 by the operator: keep `BREACHING`.** There is no plan to
disable `prices-production-oracle`, so "no data at all" should be read as bad
news. The alternative — `NOT_BREACHING`, leaning on `-no-invocations` to catch a
stopped schedule — was considered and rejected.

Consequence to know rather than rediscover: if that rule ever *is* disabled,
**both** `-oracle-dark-feed` and `-oracle-no-invocations` go to ALARM and stay
there until it is re-enabled. That is intended, not a double-page bug. Note the
precedent that raised the question — `prices-production-cleanup` has sat
deliberately disabled for weeks (task [[0200]]) — so if the oracle is ever
parked the same way, silencing these two is part of parking it.
