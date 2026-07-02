---
id: "0056"
title: "CloudWatch alarms — sdex.last_push_at freshness + mTLS cert NotAfter"
type: FEATURE
status: active
related_adr: ["0005", "0007"]
related_tasks: ["0011", "0050", "0051", "0055", "0028", "0026"]
tags: [layer-infra, priority-medium, effort-small, milestone-M1, observability, cloudwatch, alarms, sns, mtls, backfill]
milestone: 1
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../2-adrs/0005_stream2-sdex-local-workstation-backfill.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "./0050_FEATURE_be-side-prep-sns-mtls-prices-db-provisioning.md"
  - "./0051_FEATURE_clickhouse-prices-schema-and-mv-chain-migration.md"
  - "./0055_FEATURE_backfill-status-endpoint-tranche-1-isolated.md"
  - "./0028_FEATURE_sdex-cloud-push.md"
history:
  - date: 2026-05-21
    status: backlog
    who: okarcz
    note: >
      Spawned during Tranche 1 task-set creation. The §9 Tranche 1
      bullet enumerates two specific CloudWatch alarms ("sdex.last_push_at
      older than the Tranche 1 push-cadence threshold → SNS",
      "mTLS cert NotAfter < 30 days → SNS"). Acceptance criterion
      #5 explicitly requires the freshness alarm to fire under a
      skipped push. No existing task owns these alarms; fold them
      into 0011 conflates infra-bootstrap with observability.
      Carve out as a small dedicated task.
  - date: 2026-07-02
    status: active
    who: okarcz
    note: >
      Promoted from backlog to begin implementation — last remaining
      pure-code M1 task. Two probe Lambda crates
      (backfill-freshness-probe, mtls-notafter-probe) + CloudWatch
      alarms + prices-ops-alarms SNS topic.
---

# CloudWatch alarms — SDEX push freshness + mTLS NotAfter

## Summary

Build the two CloudWatch alarms that gate Tranche 1 acceptance:

1. **SDEX push freshness:** alarm fires when
   `prices.backfill_progress.sdex_archive.last_push_at` is older
   than the configured Tranche 1 push-cadence threshold (default
   7 days per §5.6, operator-tunable).
2. **mTLS cert NotAfter:** alarm fires when either of the per-env
   client cert + key pairs in AWS Secrets Manager has fewer than
   30 days until its `NotAfter` boundary.

Both publish to a Tranche 1 SNS topic for ops notification.

## Context

§9 Tranche 1 "Work" lists both alarms explicitly. Acceptance
criterion #5 directly says: "skip a scheduled `sdex-cloud-push`
cycle → freshness alarm fires once `sdex.last_push_at` exceeds
the configured Tranche 1 threshold". §7 names the mTLS NotAfter
alarm as a security primitive, and §11.4's risk row "mTLS cert
expiry not detected" pegs the mitigation to it.

The freshness alarm is unusual: it reads a field stored in
**Hetzner CH** (`prices.backfill_progress.last_push_at`), not in
CloudWatch metrics. The integration pattern is a small Lambda
that runs on a schedule (e.g. every 15 min), queries the field
via the 0052 mTLS CH client, computes age, and publishes a
custom metric `prices.backfill.sdex.push_age_seconds` to
CloudWatch. The alarm then fires on the metric.

The NotAfter alarm is also custom: scheduled Lambda reads the
two Secrets Manager secrets, parses the X.509 cert, extracts
`NotAfter`, computes days-to-expiry, publishes
`prices.mtls.days_to_notafter` to CloudWatch.

## Implementation Plan

### Step 1: Freshness-probe Lambda

Add `packages/backfill-freshness-probe/` (binary crate).

- Trigger: EventBridge Scheduler `rate(15 minutes)`.
- Behaviour: `SELECT task_name, last_push_at FROM
  prices.backfill_progress FINAL`; for each row, compute
  `now() - last_push_at` in seconds; publish a CloudWatch
  metric per stream (`prices.backfill.sdex.push_age_seconds`,
  `prices.backfill.soroban_amm.push_age_seconds`).
- The `last_push_at = NULL` case (no push yet) publishes a
  sentinel value (e.g. `-1`) that the alarm explicitly handles
  as "ok, no push expected yet" during pre-Tranche-1-first-push
  window — or alarms once the Tranche 1 window opens, per the
  §5.6 freshness subsection.

### Step 2: mTLS NotAfter-probe Lambda

Add `packages/mtls-notafter-probe/` (binary crate).

- Trigger: EventBridge Scheduler `rate(1 day)`.
- Behaviour: read both Secrets Manager secrets (cert + key),
  parse the X.509 PEM with the `x509-parser` crate, extract
  `tbs_certificate.validity.not_after`, compute days remaining
  vs `now()`, publish `prices.mtls.days_to_notafter`.
- The handler is small (~50 lines); split per-env if needed,
  or fold into a single function that iterates env-scoped
  secrets.

### Step 3: CloudWatch alarms + SNS

In the 0011 CDK app, add:

- Alarm `sdex-push-freshness-T1`: threshold 7 days * 86400 = 604800
  seconds on `prices.backfill.sdex.push_age_seconds`, 1
  datapoint at 15-min granularity, action → SNS topic
  `prices-ops-alarms`.
- Alarm `mtls-notafter-30d`: threshold 30 on
  `prices.mtls.days_to_notafter`, action → SNS topic same.
- SNS topic `prices-ops-alarms`: subscriber list seeded with
  operator email (parametrise via SSM so subscribers can be
  managed without redeploy).

### Step 3.5: Post-deploy — subscribe the ops topic (operational)

**By design, the `prices-{env}-ops-alarms` topic ships with no
subscribers.** `config.opsAlarms.notificationEmail` is left unset in
`envs/production.json` on purpose: subscriptions are managed directly
in SNS (email / Slack / PagerDuty) so the on-call roster can change
without a redeploy. The consequence is that a fresh deploy routes every
alarm to a topic **no one is listening to** until an operator subscribes
an address — the alarms fire correctly but deliver nowhere.

Deploy checklist — **must be done once per env, immediately after the
first deploy, before the fire-tests below can pass:**

1. Confirm the topic exists:
   `aws sns list-topics --profile soroban-explorer | grep prices-<env>-ops-alarms`
2. Subscribe the ops address/alias (role alias, not a personal
   address — see [[feedback-no-personal-names-in-docs]]):
   `aws sns subscribe --profile soroban-explorer \`
   `  --topic-arn arn:aws:sns:eu-central-1:<account>:prices-<env>-ops-alarms \`
   `  --protocol email --notification-endpoint prices-ops@<domain>`
3. Confirm the subscription: the endpoint receives an AWS confirmation
   email whose link **must be clicked** before delivery starts. Verify
   with `aws sns list-subscriptions-by-topic --topic-arn <arn>` →
   `SubscriptionArn` is a real ARN, not `PendingConfirmation`.

If an operator address ever becomes canonical, the alternative is to set
`opsAlarms.notificationEmail` in the env JSON and let CDK seed the
subscription at deploy (still requires the click-to-confirm step). The
`notificationEmail?` field remains supported for that.

### Step 4: Manual fire-test

For acceptance: skip a `sdex-cloud-push` cycle (operator
abstention), wait for `last_push_at` to age past the threshold,
confirm the SNS notification lands in the subscriber inbox.
Capture the test artefact (timestamp + alarm history) in
`notes/G-alarm-fire-test.md`.

Mirror for the NotAfter alarm by temporarily issuing a short-lived
test cert (e.g. valid for 25 days) into a dev-only secret,
running the probe, and confirming the alarm fires once the
threshold trips. Restore the canonical cert post-test.

### Step 5: Tests

- Unit: probe handlers with mocked CH/Secrets responses;
  assert correct metric values published.
- Integration: against Docker CH + LocalStack secrets, run
  both probes and confirm metrics appear in LocalStack
  CloudWatch.

## Acceptance Criteria

> **Legend.** `[x]` = code-complete + unit-tested + `cdk synth`-verified.
> `[ ]` marked **(operational)** = mechanism implemented + verified in synth,
> but only *confirmed* by a real deploy + fire-test against AWS/Hetzner (see
> **Implementation status**). Standing rules keep those operator-run.

- [x] `packages/backfill-freshness-probe` runs on the 15-min schedule and
      publishes push age to CloudWatch. *Metric `PushAgeSeconds` under
      `Prices/Backfill`, one datum per stream via a `Stream` dimension
      (`sdex_archive`, `soroban_amm`) — supersedes the two per-name metrics in
      the plan; see Design Decisions. Age computed server-side in CH (clock-skew
      immune) as `now() - coalesce(last_push_at, started_at)`, so a never-pushed
      stream ages from registration and fires once overdue (no permanently-OK
      sentinel). Rule + Lambda + rate synth-verified.*
- [x] `packages/mtls-notafter-probe` runs daily and publishes days-to-expiry.
      *Per-role `DaysToNotAfter` (dim `Role`) + aggregate `MinDaysToNotAfter`
      under `Prices/Mtls`, across the `ingestion` + `api` cert bundles. X.509
      parse via `x509-parser`; unit-tested against an embedded cert.*
- [x] Two CloudWatch alarms wired (push-freshness, mTLS NotAfter); both publish
      to the `prices-{env}-ops-alarms` SNS topic. *`ObservabilityStack` creates
      the topic + both alarms + `SnsAction`; also back-wired the previously
      action-less enrichment-backlog alarm. Synth asserts topic + both alarms +
      metrics.*
- [ ] **(operational)** An ops address/alias is subscribed to
      `prices-{env}-ops-alarms` and confirmed (not `PendingConfirmation`). *Topic
      ships subscriber-less by design; see Step 3.5 for the one-time per-env
      subscribe checklist. Prerequisite for the fire-test below to deliver.*
- [ ] **(operational)** Manual fire-test for the freshness alarm produces an
      SNS delivery to the configured operator email (Tranche-1 AC #5). *Requires
      a real deploy + a skipped push cycle + a subscribed address (Step 3.5).*
- [x] Threshold for the freshness alarm is operator-tunable.
      *`config.opsAlarms.sdexPushFreshnessSeconds` (default 604800) +
      `mtlsNotAfterDaysThreshold` (30); validated in `validateConfig`. Per-env
      JSON, no code change.*
- [ ] **(operational)** `notes/G-alarm-fire-test.md` records the fire-test
      timestamps + SNS message IDs for both alarms. *Produced during the deploy
      fire-test above.*

## Blocked on

- **0011** — EventBridge + CloudWatch alarm + SNS CDK
  scaffolding.
- **0050** — mTLS material + Hetzner CH endpoint provisioning.
- **0051** — `prices.backfill_progress` table.
- **0052** — shared mTLS CH client (for the freshness probe).

## Out of scope

- The Stream 1 (Soroban AMM) freshness alarm — the AMM stream
  completes in a single push during T1 and then transitions to
  `status='completed'`; ongoing freshness monitoring isn't
  meaningful. The metric is still published (for forensic value)
  but no alarm is wired.
- Backfill orchestration / push automation — see 0028.
- Ingestion lag alarm on the live Ledger Processor (§5.1 names
  a Galexie-side lag alarm, which is BE-owned) — separate concern.

## Implementation Notes (2026-07-02)

Landed on branch `feat/0056_cloudwatch-alarms-push-freshness-mtls-notafter`.

**Rust — two probe crates (mirror the 0039 worker shape: `lib.rs` pure +
unit-tested, `main.rs` cfg-gated on `lambda`; features
`default`/`aws-mtls`/`lambda`):**

- `packages/backfill-freshness-probe` — SELECTs `backfill_progress FINAL` over
  the 0052 `client_from_lambda_env` (ingestion identity), age computed
  server-side as `now() - coalesce(last_push_at, started_at)` (never-pushed
  streams age from registration so an overdue first push still fires), publishes
  `Prices/Backfill` `PushAgeSeconds` per stream. Publish failures propagate and
  fail the invocation (trips the ops-wired error alarm — a swallowed publish
  would leave the NOT_BREACHING freshness alarm silently green). 3 unit tests.
- `packages/mtls-notafter-probe` — reuses `fetch_bundle_from_extension` to read
  each role's `{cert,key,ca}` bundle, parses `NotAfter` with `x509-parser`
  (added to `[workspace.dependencies]`), publishes `Prices/Mtls`
  `DaysToNotAfter` (per-`Role`) + `MinDaysToNotAfter`. Healthy certs publish
  first (kept fresh), then **any** per-cert fetch/parse failure fails the run —
  not only a total wipeout — so a single unreadable cert can't be masked by a
  healthy sibling (it is excluded from `MinDaysToNotAfter`, so its silence is
  invisible on the expiry alarm; the error alarm is the channel that catches
  it). 7 unit tests.
- Both added to the workspace `members`. `cargo test` (default) + `cargo check
  --features lambda` + `fmt` + `clippy` all clean; `cargo check --workspace` green.

**Infra (CDK):**

- `types.ts` — `scheduleExpressions.{backfillFreshnessProbe,mtlsNotafterProbe}`
  + a new `opsAlarms` config block (`notificationEmail?`,
  `sdexPushFreshnessSeconds`, `mtlsNotAfterDaysThreshold`), all validated in
  `validateConfig`. `envs/production.json` seeds `rate(15 minutes)` /
  `rate(1 day)` and the 604800 / 30 thresholds.
- `eventbridge-stack.ts` — two rules + two `createWorkerLambda` probes, scoped
  `PutMetricData` grants (namespace-conditioned), and a second
  `secretsmanager:GetSecretValue` grant so the mTLS probe reads the `api` bundle
  too (baseline only grants `ingestion`). `MTLS_PROBE_SECRETS` env threads both
  role secret names in.
- `observability-stack.ts` — `prices-{env}-ops-alarms` SNS topic (optional
  seeded email subscription), the two alarms + `SnsAction`, and back-wired the
  previously action-less enrichment-backlog alarm to the same topic.
- `tsc -b` + `eslint` + `prettier` clean; `cdk synth` of the Observability +
  EventBridge stacks asserts the topic, both alarms (`PushAgeSeconds` /
  `MinDaysToNotAfter`), both probe functions, and all three IAM grants.

**Post-review cleanups (PR #73 code review):** ① freshness publish propagates
(no swallow) + probe `-errors` alarms now routed to the ops topic via a
generalized `createWorkerLambda` `errorAlarmActions` (imported by deterministic
`opsAlarmsTopicName`, no cross-stack ref); ② enrichment-backlog alarm re-designed
to a non-latching *lack-of-progress* metric-math signal before getting a live
channel; ③ mTLS probe fails on *any* per-cert failure (not only total); ④
freshness age uses `coalesce(last_push_at, started_at)` so never-pushed streams
age out; ⑤ ops topic subscription documented as a per-env deploy step (Step 3.5);
⑥ minor: stricter `notificationEmail` regex, dropped redundant cold-start
`SELECT 1`, `min_days` via `reduce(f64::min)`, and a shared
`prices_clickhouse::observability::init_tracing()` helper the two probes adopt.

## Design Decisions

### From Plan

1. **Two standalone probe crates over the mTLS CH client + Secrets Extension.**
   As the plan specified — freshness probe reads CH, NotAfter probe reads the
   cert bundles; both publish custom metrics an alarm fires on.
2. **Freshness threshold operator-tunable via config.** Satisfied with a typed
   `opsAlarms` config block (the "CDK parameter" option), not SSM — simpler and
   validated at synth.

### Emerged

3. **`Stream`/`Role` dimensions instead of per-name metrics.** The plan named
   `prices.backfill.sdex.push_age_seconds` etc. as distinct metrics; I publish a
   single `PushAgeSeconds` (dim `Stream`) and `DaysToNotAfter` (dim `Role`).
   Matches the house convention (`Prices/Enrichment` + `Environment` dim),
   extends to the schema's "additional `task_name`s" note without a code change,
   and lets the alarm target exactly `Stream=sdex_archive`. A separate
   `MinDaysToNotAfter` aggregate gives the NotAfter alarm one value covering
   "either cert."
4. **Age computed server-side in ClickHouse.** `now() - coalesce(last_push_at,
   started_at)` is evaluated in CH, not from the Lambda clock, because the
   timestamps were written with CH's `now()` — removes cross-host skew from the
   freshness signal. The `coalesce(…, started_at)` fallback (added when closing
   review finding #4) ages a *never-pushed* stream from its registration time
   instead of publishing a permanently-OK `-1` sentinel, so a first push that is
   overdue (or never lands) fires the alarm once the stream has existed past the
   threshold — the plan's "alarm once the Tranche-1 window opens" (§5.6). The
   sink preserves `started_at` across row updates, so it is a stable base. (Fully
   covering the *missing-row* case — no `sdex_archive` row at all — relies on the
   `seed.sql` invariant that both canonical rows always exist post-deploy.)
5. **`treatMissingData: NOT_BREACHING` on both alarms.** The freshness probe
   keeps publishing a *rising* age when pushes stop, so the climbing value —
   not missing data — is the signal; a dead probe is caught by its own
   `-errors` alarm. Avoids double-firing / flapping on deploy gaps.
6. **Re-designed + back-wired the enrichment-backlog alarm.** It shipped in
   0026 with no action (0026 explicitly left routing to 0056), so it now points
   at `prices-{env}-ops-alarms`. Before giving it a live paging channel the
   latch/storm (incoming finding #5) was fixed: the alarm no longer watches the
   absolute `EnrichmentRowsRemainingAtVolumeZero > 100_000` level (which sat in
   ALARM for hours during a legit post-backfill drain, and latched forever on
   the permanent exotic-quote floor). It now fires on *lack of progress* —
   metric-math `(EnrichmentRowsEnriched < 1) * (EnrichmentRowsRemainingAtVolumeZero > 0)`
   sustained 3×1h — which self-clears the instant a pass enriches ≥1 row, so a
   draining catch-up and a steady-state floor both read OK while a true stall
   fires. Residual (idle env + nonzero floor + no new enrichable rows for 3h can
   trip, but self-clears) needed the worker's recency-bounded remaining metric to
   excise fully. **Resolved in task 0026 (2026-07-02):** the worker now emits
   `EnrichmentRowsRemainingRecent` (a `now()`-windowed volume-zero count that
   excludes the deep-history floor) and this alarm's backlog term switched to it,
   so an idle env reads 0 and no longer trips. Finding #7
   (`EnrichmentBatchDurationMs` whole-pass, not per-batch) also **resolved in
   0026**: renamed to `EnrichmentPassDurationMs` + derived
   `EnrichmentAvgBatchDurationMs`.
7. **mTLS probe reads `SystemTime::now()` for the clock.** Cert validity is
   absolute UTC, so the Lambda wall-clock is fine here (no CH involved).

## Notes

- The freshness threshold is per-tranche-tunable. Tranche 1's
  default is 7 days because the first-chunk push covers
  ~6 months of history. Tranche 2/3 may tighten or loosen
  based on push cadence; the alarm threshold should be a
  parameter, not a constant.
- Once 0039 (full periodic-workers bundle) lands, both probe
  Lambdas could be folded into the worker set if process
  count matters. For T1, keep them standalone for clarity
  and to avoid coupling alarm health to worker bundle health.

## Incoming from task 0026 (enrichment) — PR #66 code review

Task 0026 published the enrichment spec-§5 metrics under the
`Prices/Enrichment` namespace (`EnrichmentRowsEnriched`,
`EnrichmentOracleMiss`, `EnrichmentRowsRemainingAtVolumeZero`,
`EnrichmentBatchDurationMs`) and authored a **scaffold** backlog alarm in
`infra/src/lib/stacks/observability-stack.ts`
(`prices-{env}-enrichment-backlog`). Two review findings are deferred here for
0056 to resolve when it owns the dashboard + alarm tuning end to end:

- **#5 — the enrichment backlog alarm latches / storms.** As shipped it is
  `EnrichmentRowsRemainingAtVolumeZero` Maximum > 100_000 over 6×1h,
  `treatMissingData: NOT_BREACHING`. Two problems: (a) during a legitimate
  multi-million-row post-backfill catch-up the backlog sits above the threshold
  for many consecutive hours → the alarm fires on the exact operation the
  one-shot drain exists for; (b) a permanent floor of exotic-quote candles
  (quote ∉ {USDC,USDT,XLM}, no oracle) never drains by design, so once that
  floor exceeds the threshold the alarm latches in ALARM with no path back to
  OK. Re-design when tuning: e.g. alarm on *lack of progress* (metric-math:
  `EnrichmentRowsEnriched == 0 AND EnrichmentRowsRemainingAtVolumeZero > 0`
  sustained) rather than an absolute backlog level, and/or a `_1m`-recency-bounded
  remaining count that excludes the permanent exotic-quote floor. The 100_000
  threshold is a placeholder.

  **Resolved (0056).** Re-designed to the *lack-of-progress* metric-math signal
  `(EnrichmentRowsEnriched < 1) * (EnrichmentRowsRemainingAtVolumeZero > 0)` ≥ 1
  sustained 3×1h (`GREATER_THAN_OR_EQUAL_TO_THRESHOLD`, `NOT_BREACHING`), in
  `observability-stack.ts`. Non-latching: clears the moment a pass enriches ≥1
  row, so the catch-up drain (a) and steady-state floor (b) both read OK. The
  recency-bounded remaining metric (to excise the residual idle-env + floor
  false-fire completely) was a worker-side change left to 0026 — **done
  2026-07-02:** the worker emits `EnrichmentRowsRemainingRecent` (`now()`-windowed,
  default 2h, shorter than this alarm's 3h sustain) and the alarm's backlog term
  switched to it, so an idle env reads 0 and the residual is closed. Finding #7
  (per-batch duration) also **resolved in 0026** (see below).

- **#7 — `EnrichmentBatchDurationMs` is whole-pass wall-clock, not per-batch.**
  The metric is measured across the entire `run_through` (all batches + the
  count() scans), but the name reads as per-batch latency. When building the
  dashboard, either rename/relabel it as total pass duration or divide by
  `batches` for a true per-batch figure — don't let operators size batch/timeout
  headroom off a value that grows with backlog and one-shot mode.

  **Resolved in task 0026 (2026-07-02).** Both: renamed to
  `EnrichmentPassDurationMs` (accurate whole-pass name) **and** added a derived
  `EnrichmentAvgBatchDurationMs = duration_ms / batches` (emitted only when
  `batches > 0`) for the true per-batch figure. No alarm/dashboard consumed the
  old name, so the rename was safe.

Both live in `observability-stack.ts` / the enrichment worker's `metrics.rs`;
0026 left the alarm as an explicit scaffold (commented as such) precisely so
0056 owns the final shape.
