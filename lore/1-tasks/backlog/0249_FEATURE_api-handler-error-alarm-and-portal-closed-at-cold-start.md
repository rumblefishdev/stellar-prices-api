---
id: "0249"
title: "The api-handler has no error alarm — and since 0194's review the portal closes itself at cold start with only a log line to say so"
type: FEATURE
status: backlog
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

- [ ] An `Errors` alarm exists on the api-handler in the synthesized
      Observability template, and `cdk diff` shows only additions.
- [ ] The metric filter matches a real `portal closed at cold start` line
      (proved by a log-insights query over an induced one, or by a unit test
      on the pattern against a captured line), and the alarm fires on it.
- [ ] Neither alarm fires over a week of ordinary traffic.
- [ ] The runbook's "nothing pages on it" sentence is updated.
