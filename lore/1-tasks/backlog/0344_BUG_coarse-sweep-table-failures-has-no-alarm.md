---
id: "0344"
title: "CoarseSweepTableFailures has no alarm — a coarse-sweep table that errors passes as a successful run"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0218", "0347"]
tags: [layer-infra, priority-medium, effort-small, observability, cloudwatch, alarms]
links:
  - "../../../infra/src/lib/stacks/observability-stack.ts"
  - "../../../packages/enrichment-worker/src/repair.rs"
  - "../../../packages/coarse-sweep-worker/src/main.rs"
  - "../../../docs/runbooks/repair-coarse-usd-values.md"
history:
  - date: 2026-11-13
    status: backlog
    who: stkrolikiewicz
    note: >
      From the 2026-11-13 read-only CloudWatch review (shared account
      750702271865, eu-central-1). Re-checked on origin/develop (66724665) and
      live CloudWatch the same day. Protection gap, not a cost item.
---

# CoarseSweepTableFailures has no alarm

## Summary

When one table's coarse-sweep pass errors, the sweep logs it and carries on, and
the Lambda still returns `Ok`. The only signal is the custom metric
`Prices/Enrichment CoarseSweepTableFailures`, and **no alarm reads it**. The
runbook says to alarm on it. `prices-production-coarse-sweep-errors` cannot see
it, because the invocation did not fail.

## Evidence (origin/develop 66724665, 2026-11-13)

- `packages/enrichment-worker/src/repair.rs:685-694`: the per-table `Err` arm
  logs `"coarse sweep: table failed (continuing)"`, pushes the table onto
  `failed_tables` and the function still returns `Ok(summary)`.
- `packages/coarse-sweep-worker/src/main.rs:160-187`: the `Ok(sum)` arm publishes
  the sweep metrics and returns `Ok`. Only a whole-run `Err` (`:189-207`) fails
  the invocation.
- `docs/runbooks/repair-coarse-usd-values.md:377-379`: "`CoarseSweepTableFailures`
  … **Alarm on `> 0`** — the dead-sweep signal." `packages/enrichment-worker/src/metrics.rs:114`
  says the same.
- Live `aws cloudwatch describe-alarms`: 155 metric alarms, **none** reads any
  `CoarseSweep*` metric.
- The metric is published on every run: `get-metric-statistics` 2026-10-14..11-13
  gives 720 samples, Sum 0, Max 0. So an alarm with `notBreaching` has real data
  to read, and today's baseline is 0.

## Implementation

- One alarm in `observability-stack.ts`, next to the coarse-sweep worker-health
  alarms (`workerHealth` entry `name: 'coarse-sweep'`, ~:2331):
  `Prices/Enrichment CoarseSweepTableFailures`, `Environment` dimension, Maximum,
  `>= 1`, period = sweep cadence (hourly: 720 runs in 30 days), 1 of 1,
  `notBreaching` (a missing run is already the `-no-invocations` alarm's job),
  alarm and OK actions to the ops topic.
- Description: name the WARN line to search for and the table field it carries,
  and point at `docs/runbooks/repair-coarse-usd-values.md`.
- Keep the description under the 1024-character guard (`assertAlarmDescriptionsFitCloudWatch`).
- Update the runbook line so it names the alarm instead of telling the reader to
  create one.

## Acceptance Criteria

- [ ] `prices-<env>-coarse-sweep-table-failures` exists after synth and reads
      `CoarseSweepTableFailures`.
- [ ] A synth test asserts the alarm's metric, threshold and missing-data mode.
- [ ] Runbook §"Metrics to watch" names the alarm.
- [ ] Deployed only after the other teams on the shared AWS account agree.

## Notes

- [[0347]] stops publishing four other `CoarseSweep*` metrics. `CoarseSweepTableFailures`
  is **kept** there on purpose: it is the one signal the Lambda's own metrics
  cannot give.
