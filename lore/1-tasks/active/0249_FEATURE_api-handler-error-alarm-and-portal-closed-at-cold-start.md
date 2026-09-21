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
      lines. **"Fires on it" open**: needs a deploy.
- [ ] Neither alarm fires over a week of ordinary traffic. (Open — needs a
      deploy and a week.)
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

## Issues Encountered

- **The signal had already fired, and nothing saw it.** `filter-log-events`
  over 30 days: **243** `portal closed at cold start` lines, all on
  2026-09-18 (SSM reads through the extension returning HTTP 400 for
  `pricing-api-free-plan-id`, `discord-guild-id` and
  `min-account-age-minutes`). None since. Both eligibility parameters exist
  today. This is the case the alarm exists for.
- **`Errors` alone would have missed that day's outage.** On 2026-09-18
  14:00 CEST the API returned **14,865 5xx**, exactly the api-handler's
  **14,865 Lambda Throttles** (concurrency peaked at 700). Lambda `Errors`
  stayed 0: throttles and router-returned 5xx are not invocation errors.
  This is why the 5xx alarm was added (Design Decision 5).
- **`MetricFilter.metric()` defaults to `avg`.** Sum is passed explicitly.
- **`apiMetric()` sets a `label`, and with a label aws-cdk-lib renders the
  alarm as a one-entry metric-math array.** The 5xx alarm builds its own
  `cloudwatch.Metric` on the shared `apiDims`, so it keeps the plain
  single-metric form every other alarm uses.

## Design Decisions

### From Plan

1. **Errors threshold 1.** Baseline 0 (0194: 217 invocations / 4 h, and 0
   every day of the 14 read on 2026-09-21). One router serves every route
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
   original scope; the 2026-09-18 throttling proved `Errors` blind to it.
   Threshold **5**, absolute: the 14-day baseline outside that day was one
   window with 2 × 5xx (2026-09-08), and traffic is sometimes 1–7
   requests/day, so a rate would swing on every single error. `Errors ≥ 1`
   stays for a lone init crash or panic.
6. **Namespace `Prices/ApiHandler`**, not the brief's first guess
   `StellarPrices/…`. Every custom metric in the stack is `Prices/<Component>`.
7. **Two stale comments fixed** (`compute-stack.ts`, `config.rs`) with
   Adam's approval. `done.md`: docs match reality.
8. **The pattern is tested with AWS's evaluator, not a local unit test.** A
   hand-written matcher would only test our copy of AWS's syntax.

## Future Work

- After deploy: check that `prices-production-api-handler-portal-closed`
  fires on an induced closure, and that none of the three alarms fires over
  a week of ordinary traffic (AC 2 second half, AC 3).
