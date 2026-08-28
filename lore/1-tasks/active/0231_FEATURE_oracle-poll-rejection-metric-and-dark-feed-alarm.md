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

## Review fixes (PR #267, 2026-08-28)

Four findings from `/code-review`, all accepted and fixed.

1. 🔴 **The synth cadence guard only matched `rate(N minutes)`.** Every other
   unit fell through as if it were a cron expression — and `rate(1 hour)` is
   the idiom `assetSupply`, `assetDiscovery` and `enrichment` already use in the
   same `production.json`, so it is precisely how someone would slow this poll
   down. The guard would have waved through the exact change it exists to
   catch. Now `rateExpressionMinutes` normalises minute/hour/day and **throws on
   any `rate(...)` it cannot read**, rather than returning "unknown".
2. ⚠️ **The guard's message and its condition disagreed.** It said "at least 2x
   the cadence" while checking `cadence > bucket`, so `rate(6 minutes)` passed
   while violating the stated rule. The 2x is the real invariant — at one pass
   per bucket, two passes drifting into the same bucket leave the next empty —
   so the check now enforces what it says.
3. ⚠️ **`-dark-feed` cannot tell a dark feed from a broken metric path.** The
   publish only logs a warning, so a mis-scoped namespace grant, a
   `METRIC_NAMESPACE` typo, or deploying Observability ahead of EventBridge all
   page as a data outage while `oracle_prices` fills normally. Kept `BREACHING`
   (decided above), but the runbook description now names the metric path as a
   candidate cause — it previously listed four causes, none of them this one.
   ⏳ **Deploy-order consequence, still owed:** on first rollout this alarm
   enters ALARM the moment ObservabilityStack lands and cannot clear until the
   new oracle binary from EventBridgeStack has run a pass. **Deploy EventBridge
   first, then Observability.** There is no `addDependency` between them and the
   Makefile deploys stacks individually, so nothing enforces this.
4. 🔴 **A failed pass dropped the rejection count it had already measured.**
   Rejections happen in the per-symbol loop, ahead of the ClickHouse writes that
   are the likely failure — so a pass could refuse a reading and then die, and
   the one metric this task exists to produce was discarded on exactly the
   invocation that had something to say. `run_oracle` now returns
   `OracleFailure { error, timestamp_rejected }`; `failure_metrics` takes the
   count and publishes it. New test covers it.
5. ⚠️ **A comment asserted a reason that was not true.** The old
   `failure_metrics` doc claimed omitting a `0` avoided "holding that alarm's
   average down" — but the alarm is `Sum` with `>= 1`, where zeros are inert.
   Subsumed by fix 4, which removes the special case entirely. Recorded because
   a plausible-sounding cross-file claim is the kind of thing the next editor
   builds on.

---

# 📕 DEPLOY RUNBOOK — read this before deploying 0231

> **This is the runbook.** When the operator asks for deploy instructions for
> 0231, point here rather than re-deriving them.

## 🔴 Order is load-bearing: EventBridge FIRST, Observability SECOND

`-oracle-dark-feed` treats missing data as `BREACHING` (decided 2026-08-28). The
metric it watches does not exist until the **new** oracle binary has run a pass.
So if Observability lands first, the alarm goes to ALARM the moment it is
created and **cannot clear** until EventBridge is deployed and a pass runs — a
self-inflicted page on a feed that is perfectly healthy.

Nothing enforces this: there is no `addDependency` between the two stacks in
`infra/src/lib/app.ts`, and the Makefile deploys them individually. It is on the
operator.

## Steps

### 1. [local machine, repo root] Build the Lambda bootstraps

⚠️ `make -C infra deploy-production-*` does **NOT** build the Rust —
`build-production` is CDK + portal bundle only. Skipping this deploys the
**previous** binary and every metric below stays absent.

Both the `--features lambda` flag and the explicit `-p` are required; without
the feature the build produces nothing and still exits 0.

```
cargo lambda build --release --arm64 --features lambda -p oracle-worker
tools/scripts/verify-lambda-assets.sh
```

**Checkpoint:** the verifier must pass before going on.

### 2. [local machine] Record the pre-deploy `CodeSha256`

`CodeSha256` is the only proof a deploy actually landed — not the deploy's exit
status, and not the CDK output.

```
aws lambda get-function-configuration \
  --function-name prices-production-oracle \
  --query CodeSha256 --output text
```

### 3. [local machine] Deploy EventBridge — the worker and its IAM grant

```
make -C infra deploy-production-eventbridge
```

**Checkpoint:** re-run step 2. The `CodeSha256` **must have changed**. If it has
not, the bootstrap was stale — go back to step 1.

⚠️ Also confirm the cleanup rule did not get re-enabled as a side effect: CDK
asserts `prices-production-cleanup` ENABLED while production reality is
DISABLED, so every EventBridge deploy can silently flip it. `describe-rule`
before *and* after. See [[cleanup-rule-shreds-backfill-output]].

### 4. [local machine] Wait for one pass, then confirm the metric exists

The schedule is `rate(5 minutes)`. Do not go on until `Prices/Oracle` actually
has data — this is the step that would have caught task [[0204]]'s 10-of-13
blind alarms.

```
aws cloudwatch list-metrics --namespace Prices/Oracle \
  --query 'Metrics[].MetricName' --output text
```

**Checkpoint:** `OracleRowsWritten` and `OracleTimestampRejected` must both
appear. If the namespace is empty, the publish is failing — check the worker
logs for `cloudwatch metric publish failed`, and check the grant's
`cloudwatch:namespace` condition against `METRIC_NAMESPACE`.

### 5. [local machine] Deploy Observability — the alarms

```
make -C infra deploy-production-observability
```

⚠️ Deploy the **whole** Observability stack, never a subset — task 0204 left 10
of 13 alarms blind exactly that way.

⚠️ If `cdk diff` reports "no differences" alongside *"Omitted N changes because
they are likely mangled non-ASCII characters"*, re-run it with `--strict`. These
descriptions are full of non-ASCII, so that line will appear.

### 6. [local machine] Confirm the alarms are BOUND, not merely created

```
aws cloudwatch describe-alarms \
  --alarm-names prices-production-oracle-dark-feed \
                prices-production-oracle-timestamp-rejected \
                prices-production-oracle-no-invocations \
                prices-production-oracle-duration-near-timeout \
  --query 'MetricAlarms[].[AlarmName,StateValue]' --output text
```

**Checkpoint:** every one must read **OK**. `INSUFFICIENT_DATA` on
`-dark-feed` or `-timestamp-rejected` means the alarm is watching a series that
does not exist — almost always an `Environment` dimension that disagrees with
the Lambda's `ENV_NAME`. Fix that before believing any of this works.

### 7. Induce both alarms

Deploying an alarm and proving it fires are different jobs, and the second is
where the time goes ([[0204]], [[0218]]). See **Induction plan** above for the
two procedures — the rejection alarm takes one `put-metric-data`; the dark-feed
alarm needs the throwaway-clone approach, because the live worker keeps every
real bucket non-empty.

## Final test command

One pass's worth of real data, straight from the worker:

```
aws logs tail /aws/lambda/prices-production-oracle --since 10m \
  --filter-pattern '"oracle-worker run complete"'
```

Expect `written` > 0, `timestamp_rejected` = 0, and `queried` matching
`TRACKED_SYMBOLS`. That line, plus four OK alarms from step 6, is the deploy
verified.

---

# ⏸️ SESSION END 2026-08-28 15:02 UTC — deployed, 3 of 5 ACs verified

**PR #267 merged 14:15:25 UTC. Both stacks deployed to production.** Task stays
`active`: two items below must be settled before it can be archived.

## Deploy record

| | |
|---|---|
| EventBridge deployed | 14:21:26 UTC — CodeSha256 `iPZJeb…` → **`lL4mT/k0AAruvkpxtTtlKK3bZstvBo3rWBLKy5E6Ed8=`** |
| new binary confirmed running | the 14:22:38 pass is the first log line carrying `timestamp_rejected` |
| Observability deployed | ~14:30 UTC — all four alarms created 14:30-14:31 |
| `prices-production-cleanup` | still **DISABLED** after the EventBridge deploy ✅ |
| metric publish failures | **0** in the logs |

## Acceptance criteria

- [x] **AC 1** — `OracleTimestampRejected` and `OracleRowsWritten` published by a
      real pass and visible in CloudWatch. All 7 series present; first pass read
      `Runs=1 FailedRuns=0 RowsWritten=2 TimestampRejected=0 RowsSkipped=0`.
- [ ] **AC 2** — dark-feed alarm fires, verified by inducing. ⏳ **See loose end 1.**
- [x] **AC 3** — rejection alarm fires, raw value reachable. Induced with one
      synthetic datapoint at 14:42:00; `OK → ALARM` at **14:43:05**, self-cleared
      at 14:48:05. Description carries the log filter and field names.
- [x] **AC 4** — pure mapping with unit tests. 18 tests, non-vacuity checked.
- [ ] **AC 5** — neither alarm false-fires on an idle environment. ⏳ Partially:
      all four alarms sat OK from creation through session end, and
      `-timestamp-rejected` returned to OK by itself. Completing AC 2 completes
      this, since the real `-dark-feed` must have stayed OK while the clone fired.

## ⛔ Loose end 1 — an induction alarm is LIVE ON PROD

`prices-production-oracle-dark-feed-induction`, created 14:50 UTC. A throwaway
clone of the dark-feed geometry on a scratch metric
(`Prices/Oracle` / `OracleRowsWrittenProbe`, probe datapoint at 14:49). **No SNS
action**, so it cannot page — but nothing will publish that probe again, so it
latches in ALARM until removed.

🔑 **Read its state first — that state IS AC 2.**

```
aws cloudwatch describe-alarms \
  --alarm-names prices-production-oracle-dark-feed-induction prices-production-oracle-dark-feed \
  --query 'MetricAlarms[].[AlarmName,StateValue,StateUpdatedTimestamp]' --output text
```

Expected: the **clone** in ALARM (breaching ~15:19 = 14:49 + three 10-minute
periods) and the **real** `-dark-feed` still OK. That pair proves the geometry
fires on a dark series and stays quiet on a live one — AC 2 and AC 5 together.
⚠️ If the clone is still OK well past 15:20, that is a real finding, not a slow
alarm: it would mean the `FILL(m,0)` shape does not breach, which is exactly
what [[0222]] showed can happen silently.

Then delete it:

```
aws cloudwatch delete-alarms --alarm-names prices-production-oracle-dark-feed-induction
```

## ⚠️ Loose end 2 — alarms fire but may not reach Slack (NOT a 0231 defect)

The operator reported the last Slack alarm as **14:31**, yet
`-oracle-timestamp-rejected` went ALARM at 14:43:05 and back to OK at 14:48:05.
Everything AWS controls is green and measured:

| link | evidence |
|---|---|
| alarm transitioned | `OK → ALARM` 14:43:05 on a real datapoint |
| CloudWatch → SNS | "Successfully executed action" ×3 (14:31, 14:43, 14:48) |
| SNS → Chatbot | `NumberOfNotificationsDelivered` = 1 in the 14:39 bucket, `Failed` = 0 |
| subscription | confirmed, **no FilterPolicy**, `RawMessageDelivery` false |

The gap is inside **Chatbot → Slack**, and only after 14:31 — the 14:31 burst of
four alarm-creation notifications did arrive. The AWS Chatbot API was not
reachable from the session's network, so its configuration was never inspected.

🔴 **Not yet ruled out: the operator simply needed to refresh or search Slack.
Ask before filing anything.** If real, spawn a separate task — this routing is
[[0056]]'s wiring and nothing in 0231 touched it. It does not block these ACs;
AC 3 asks only that the alarm fire with the raw value reachable, which it did.

## Prod facts measured here — reuse rather than re-derive

- The oracle polls **2 symbols** and writes **2 rows a pass**, not 30. The
  dark-feed series therefore sits at ~4 per 10-minute bucket against `< 1`.
- Oracle Lambda duration **8.8-10.0 s** against a 96 s threshold (80% of its
  120 s timeout). Flat, because this Lambda does one thing per invocation.
- 🔑 **CloudWatch alarm windows are query-anchored, not clock-aligned.** The
  14:42 datapoint was attributed to a bucket labelled `14:38` and the alarm
  fired at 14:43 — sooner than a clock-aligned estimate predicted.
- 🔴 An alarm was called "did not fire" when the query window had simply closed
  before the datapoint landed. **Wait for the period to close before
  concluding.** Same family as [[0222]]'s query artefacts.
