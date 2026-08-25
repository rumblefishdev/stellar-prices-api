---
id: "0221"
title: "Confirm the MONTH quota rollover instant on production, on or after 1 September 2026"
type: RESEARCH
status: backlog
related_adr: ["0010"]
related_tasks: ["0191", "0180", "0157"]
tags: [layer-backend, priority-low, effort-small, milestone-M3, epic-self-service-onboarding, api-gateway, usage-plan, measurement]
milestone: 3
links:
  - "../../archive/0191_FEATURE_rework-key-once-per-quota-period.md"
  - "../../archive/0180_RESEARCH_settle-undocumented-discord-and-aws-behaviours/README.md"
history:
  - date: "2026-08-25"
    status: backlog
    who: claude
    note: >
      Spawned from [[0191]] future work — the one acceptance criterion 0191
      could not close, because the next real `MONTH` rollover is 1 September
      2026. Dated, not blocked: nothing waits on it.
---

# Confirm the MONTH rollover instant on production

## Summary

AWS documents neither the quota reset instant nor its timezone (ADR 0010
correction #2). [[0191]] stopped presenting "00:00 UTC on the 1st" as
AWS-documented and states it as **our** rule, defined once in
`portal/period.rs` — that closed the wording half of the criterion. What is
still unmeasured is what AWS actually does at a `MONTH` boundary.

## Context

[[0180]] item 7's `DAY`-period proxy was abandoned on 2026-08-24 after two runs
died silently, and the scratch stack was torn down — the proxy existed to avoid
waiting for 1 September, and by then 1 September was 8 days away. The
replacement is to read the real rollover off production instead of a scratch
plan.

## Implementation

- On or after **2026-09-01**, look in the api-handler log for the warn
  `summarize_days` emits when `GetUsage` reports a **quota reset inside the
  queried period** (`packages/prices-api/src/portal/keys/gateway.rs`). The
  timestamp of the reset row is the answer.
- If production traffic is too thin to produce the warn, the fallback is the
  archived `item7-quota-rollover.sh` against a `MONTH` scratch plan drained on
  31 August — **its defects are recorded at the top of the file and must be
  fixed first**, chiefly the `teardown` that reports success while leaving the
  usage plan alive (it deletes the plan before the REST API that still
  references its stage, and every delete is `|| true`).
- If it is ever re-run: `UpdateUsagePlan` is throttled to **1 request per 20 s
  per account, non-adjustable**, and the control plane shares a **10 rps /
  burst 40** budget with our deploys. A careless loop slows CI for everyone.

## Acceptance Criteria

- [ ] The `MONTH` reset instant and its timezone are recorded with the date and
      the source (log line or measurement), or recorded as still unobserved
      with the reason
- [ ] ADR 0010 correction #2 updated: either closed with the measured value, or
      restated with what is now known
- [ ] If AWS's instant differs from ours, the difference is written down as a
      dashboard-label wrinkle — **not** a change to the cap, which is ours by
      definition ([[0191]] decision #2) and stays one definition in
      `portal/period.rs`
