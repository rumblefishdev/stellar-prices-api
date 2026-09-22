---
id: "0249"
title: "The api-handler has no error alarm — and since 0194's review the portal closes itself at cold start with only a log line to say so"
type: FEATURE
status: active
related_adr: ["0008"]
related_tasks: ["0194", "0231"]
tags: ["priority-high", "effort-small", "observability", "layer-infra", "epic-self-service-onboarding", "milestone-M3"]
milestone: 3
links:
  - "../../../infra/src/lib/stacks/observability-stack.ts"
  - "../../../packages/prices-api/src/config.rs"
  - "../../../packages/prices-api/src/main.rs"
history:
  - date: "2026-09-01"
    status: backlog
    who: akot
    note: >
      Spawned from [[0194]] future work — the PR review's finding 1. The
      "fail loudly in `Init Errors`" stance on the portal's cold-start reads
      turned out to be alarmed by nothing; replacing it with "close the
      portal, log, keep `/v1` up" needs the alarm that was always missing.
  - date: "2026-09-21"
    status: active
    who: akot
    note: >
      Activated. Scope: api-handler Errors alarm + metric filter/alarm on
      `portal closed at cold start`, runbook sentence; no deploy.
  - date: "2026-09-21"
    status: active
    who: akot
    note: >
      Implemented on feat/0249 (1fc7128, e67673d, 67f8d49; not pushed): three
      alarms: api-handler Errors ≥1, portal-closed metric filter ≥1, API
      5xx ≥5, added after 2026-09-18 showed 14,865 throttled 5xx with
      Errors = 0. Plus runbook and 2 comments. Pattern proven by
      test-metric-filter; diff additions-only. Stays active: AC2 "fires" and
      AC3 need a deploy.
  - date: "2026-09-22"
    status: active
    who: akot
    note: >
      Merged (PR #330, 15421e3) and deployed to production at 12:08 CEST via
      `make deploy-production-observability` (27 s, additions only; 0151's
      three zero-invariant alarms rode along). 61 alarms live, all six new
      ones in OK, the metric filter created with the exact pattern, and the
      INSUFFICIENT_DATA → OK transitions delivered to the ops topic. A
      forced ALARM on portal-closed (set-alarm-state) delivered too and the
      alarm returned to OK silently, as designed. AC 1 and AC 4 confirmed
      live; the induced-closure half of AC 2 and the quiet week (AC 3, until
      2026-09-29) remain.
---

# An error alarm for the api-handler, and one for a portal that closed itself

## Summary

`observability-stack.ts` alarms on the ledger-processor's `Errors`, on
durations, on freshness, on the oracle feed — and on nothing the
**api-handler** does wrong. An init failure, a panic under a request, a
`502` to a partner: all of it is a metric nobody watches. [[0194]]'s audit
found this the hard way: the "loud" `Init Errors` its cold-start reads were
designed to produce were loud only to whoever ran a probe.

[[0194]] then changed the stance (`AppConfig::load_portal_or_close`): a
portal source that fails to load — the Discord secret, the free-plan id,
either eligibility parameter — **closes the portal in that execution
environment** instead of panicking the Lambda that also serves `/v1`. That is
the right trade for the data API and it leaves one signal behind: an `error`
log line, `portal closed at cold start`, with the failing variable named.
Nothing reads it. This task makes both signals page.

## Context

- One router serves every route group (ADR 0008), so the api-handler's
  `Errors` metric is `/v1`'s error metric. It has never had an alarm; the
  ledger-processor's (`ledgerProcessorErrorAlarm`, `AWS/Lambda Errors ≥ 1
  over 5 min`) is the template.
- The portal's four cold-start reads go through the Parameters and Secrets
  extension with a 2 s timeout and no retry, three of them against Parameter
  Store's 40 TPS account-wide default. A burst of cold starts ([[0121]]'s
  ramp) can throttle one; the environment then serves a closed portal for
  its lifetime — `/config` says `enabled: false` from that one environment
  while the others say `true`. Visible only in the log line, and only if
  someone looks.
- The observability stack's rule: alarm names must not collide
  (`prices-${env}-…`); add here, never a second stack.

## Implementation

- `AWS/Lambda Errors ≥ 1` over 5 min on `prices-${env}-api-handler`, same
  shape and action as `ledgerProcessorErrorAlarm`. Decide the threshold
  against the measured baseline: `Errors` was 0 over every window [[0194]]
  read (217 invocations / 4 h on 2026-08-31), so 1 is defensible; state why
  if a higher one is chosen.
- A `logs.MetricFilter` on the api-handler log group for the JSON line whose
  message is `portal closed at cold start` (the subscriber is
  `tracing_subscriber::fmt().json()` — match on the `fields.message` or
  `message` key the emitted shape actually has, read one off a real log
  before writing the pattern), publishing a custom metric; an alarm at
  `≥ 1` over 5 min, treating missing data as OK.
- Both alarms into the same notification path the existing ones use;
  outputs for their names, as the stack does for the others.
- Runbook: `docs/runbooks/portal-oauth-deploy-prep.md` says the `/config`
  probe after a deploy is the check *because* nothing pages — amend once
  something does.

## Acceptance Criteria

- [x] An `Errors` alarm exists on the api-handler in the synthesized
      Observability template, and `cdk diff` shows only additions.
      (`prices-production-api-handler-errors`; diff: 7 `[+]`, plus the derived
      `DashboardBody` / `DashboardAlarmCount` changes — see Implementation Notes.)
- [ ] The metric filter matches a real `portal closed at cold start` line
      (proved by a log-insights query over an induced one, or by a unit test
      on the pattern against a captured line), and the alarm fires on it.
      **Match half done** — `aws logs test-metric-filter` over real production
      lines, and `filter-log-events` with the same pattern returned 243 real
      lines. **Delivery proven live 2026-09-22**: `set-alarm-state` to ALARM
      executed the SNS action, and the alarm went back to OK without an OK
      notification. **Still open**: a real closure driving the filter →
      metric → ALARM path end to end (induced, or the next load test).
- [ ] Neither alarm fires over a week of ordinary traffic. (Deployed
      2026-09-22 12:08 CEST; the week runs to 2026-09-29.)
- [x] The runbook's "nothing pages on it" sentence is updated.

## Implementation Notes

Branch `feat/0249_api-handler-error-alarm-and-portal-closed-at-cold-start`
from `develop` @ `49be685`; three commits, four files (+185 / −10):

- `1fc7128` feat — `prices-${env}-api-handler-errors`: `AWS/Lambda Errors`,
  `FunctionName=prices-${env}-api-handler`, Sum / 5 min, `≥ 1`, 1/1,
  missing = OK, alarm + OK action on the ops topic, output
  `ApiHandlerErrorAlarmName`. Same shape as `ledgerProcessorErrorAlarm`.
- `e67673d` feat — `AWS::Logs::MetricFilter` on
  `/aws/lambda/prices-${env}-api-handler` (imported by name, no cross-stack
  reference), pattern `{ $.fields.message = "portal closed at cold start*" }`,
  metric `Prices/ApiHandler` / `PortalClosedAtColdStart` = 1 (no default);
  alarm `prices-${env}-api-handler-portal-closed` (Sum / 5 min, `≥ 1`,
  missing = OK). Plus `prices-${env}-api-5xx`: `AWS/ApiGateway 5XXError` on
  the dashboard's `apiDims` (ApiName + Stage), Sum / 5 min, `≥ 5`. Outputs
  `ApiHandlerPortalClosedAlarmName`, `Api5xxAlarmName`.
- `67f8d49` docs — runbook `portal-oauth-deploy-prep.md` §3 and §5; the
  now-false "no error alarm" comments in `compute-stack.ts` (PORTAL_ENABLED
  block) and `config.rs` (`load_portal_or_close` doc).

**Pattern proof.** `aws logs test-metric-filter` with the pattern exactly as
synthesized, over four real production lines — the portal-closed ERROR, an
unrelated ERROR (`clickhouse query failed`), a same-prefix WARN
(`portal sign-in callback rejected`) and an `INIT_START` platform line —
returns `matches[].eventNumber == [1]`. Re-run independently by the verifier.
There is no local CloudWatch-pattern evaluator and no CDK test runner in
infra, so AWS's own evaluator is the test; nothing is committed for it.

**Diffs.** Offline `cdk diff` against the `49be685` synth: `[+]` 3 alarms,
1 MetricFilter, 3 outputs; `[~]` `DashboardBody` (3 ARNs join the strip) and
`DashboardAlarmCount` 58 → 61; no `[-]`. The live diff against production
also shows lines from before this change: 0151's three
`ZeroInvariantAlarmCount*` alarms (merged, never deployed) and
dashboard-rendering noise. That makes `DashboardAlarmCount` 55 → 61 live.
`npm run infra:verify-dashboard`: 61 alarms (52 own + 9 imported). typecheck,
lint, prettier and `cargo fmt --check` are clean. Nothing was deployed.

**Backtest and local run (2026-09-21).** The three alarms' exact template
settings were replayed over 35 days of production 5-minute datapoints (see
Issues Encountered). The api-handler binary built from this branch, run
locally with a broken portal config, emits the portal-closed line in the
production shape. The synthesized pattern, through `test-metric-filter`,
matches it and rejects the healthy run's output, the phrase mid-sentence, a
flattened (`$.message`) line and plain text. Both alarm dimension sets exist
as live metrics; the ops topic has a confirmed subscription and delivered
other `prices-production-*` alarms on 2026-09-18/19. What still needs a
deploy is the state change and the notification themselves.

## Issues Encountered

- **The portal-closed signal had already fired, and nothing saw it.**
  `filter-log-events` over 35 days: **243** `portal closed at cold start`
  lines, all on 2026-09-18, in two 5-minute windows (45 at 11:45 UTC, 198 at
  12:45 UTC). Both were the ramps of [[0293]]'s load test: bursts of cold
  starts whose Parameter Store reads came back HTTP 400 through the
  extension. This is the scenario the task predicted, and they were real
  closures. None since. **Expect this alarm during a load test.**
- **`Errors` alone misses the outages that matter.** Read 2026-09-21, 35 days
  of 5-minute windows:
  - 2026-09-03: a load run exhausted the ClickHouse read quota; **28,853
    5xx** over ~25 min, Lambda `Errors` **0**, nobody paged (the 26-minute
    outage [[0293]] mentions). The 5xx alarm would have fired in 4 windows.
  - 2026-09-18: [[0293]]'s ramp to 1000 req/s; 14,865 5xx, all Lambda
    throttles at that run's temporary reserved concurrency of 700 (since
    removed), `Errors` 0. A controlled test, not an incident, but the same
    blind spot.
  - 2026-09-02: an init panic (`main.rs:42`), `Errors` = 7 and 7 × 5xx in one
    window. The `Errors` alarm would have fired. An init panic fires both
    alarms.
  - Stray 5xx windows below the threshold: 1, 2 and 4. No false alarm from
    any of the three alarms on ordinary days.
  This is why the 5xx alarm was added (Design Decision 5).
- **`MetricFilter.metric()` defaults to `avg`.** Sum is passed explicitly.
- **`apiMetric()` sets a `label`, and with a label aws-cdk-lib renders the
  alarm as a one-entry metric-math array.** The 5xx alarm builds its own
  `cloudwatch.Metric` on the shared `apiDims`, so it keeps the plain
  single-metric form every other alarm uses.

## Design Decisions

### From Plan

1. **Errors threshold 1.** Baseline 0 (0194: 217 invocations / 4 h; in the
   35 days read on 2026-09-21 the only non-zero window is the 2026-09-02
   init panic, which should page). One router serves every route
   group (ADR 0008), so one error is a `/v1` error.
2. **Pattern keyed on `$.fields.message`, read off a real line.** The
   subscriber is `fmt().json()` without `flatten_event`, and the function
   logs in Text format, so the raw JSON is the event.
3. **Log group imported by name.** No CFN reference to ComputeStack. The
   observability stack stays independently deployable.
4. **Missing data = OK** on all three alarms. A quiet log group publishes
   nothing (no `defaultValue`), and that is not a breach.

### Emerged

5. **API Gateway 5xx alarm added (Adam, 2026-09-21).** Not in the task's
   original scope; 2026-09-03 and 2026-09-18 proved `Errors` blind to
   router-returned 5xx and to throttles. Threshold **5**, absolute: the
   stray windows in 35 days were 1, 2 and 4, and traffic is sometimes 1–7
   requests/day, so a rate would swing on every single error. `Errors ≥ 1`
   stays for a lone init crash or panic.
6. **Namespace `Prices/ApiHandler`**, not the brief's first guess
   `StellarPrices/…`. Every custom metric in the stack is `Prices/<Component>`.
7. **Two stale comments fixed** (`compute-stack.ts`, `config.rs`) with
   Adam's approval. `done.md`: docs match reality.
8. **The pattern is tested with AWS's evaluator, not a local unit test.** A
   hand-written matcher would only test our copy of AWS's syntax.
9. **Portal-closed alarm has no OK action** (from `/code-review`, Adam,
   2026-09-21). The line is logged once per closing cold start, so the alarm
   returns to OK one window later while the environment is still closed; an
   OK notification would read as "recovered". The description says so. The
   Errors and 5xx alarms keep their OK actions: their metric keeps flowing
   while the fault lasts.
10. **A guard test ties the log line to the filter**
    (`tools/scripts/portal-closed-filter-guard.test.mjs`, Adam, 2026-09-21).
    Nothing else links `main.rs`'s message to the pattern's prefix, or the
    un-flattened JSON subscriber to `$.fields.message`; a reword would
    silence the alarm with every check green. The TypeScript CI job runs the
    guard, so `packages/prices-api/src/main.rs` was added to that job's path
    filter, or a main.rs-only PR would skip it.
11. **Review answers (Oskar, 2026-09-22; commit `002bfac`).** (a)
    ObservabilityStack now `addDependency(compute)`: the metric filter is the
    stack's first resource created on a Compute-owned one and
    `fromLogGroupName` emits no dependency. Ordering only, still no
    `Fn::ImportValue`; the `--exclusively` target gets a Makefile note
    instead (log group must exist on a fresh env / after a Compute
    destroy). (b) The guard's search is bounded to the portal-closed
    filter's own block — the unbounded version passed on a second filter
    added later in the file (reproduced, now a mutation test). (c) The
    log-format coupling gets a comment on `ApiHandlerFunction`, not an
    assertion: AWS documents that JSON format does not re-encode lines that
    are already JSON, so forbidding `loggingFormat` would block a
    legitimate change on a premise the docs contradict. Any log-path change
    is re-proven with `test-metric-filter` on a deployed line.

## Future Work

- Check that `prices-production-api-handler-portal-closed` fires on a real
  closure (induced, or the next load test's cold-start burst), and that none
  of the three alarms fires over the week to 2026-09-29 (AC 2 second half,
  AC 3). Then close.
