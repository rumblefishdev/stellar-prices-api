---
id: "0347"
title: "Stop publishing 14 custom metrics that nothing reads or that duplicate AWS/Lambda, and trim prices-production-overview to the 50-metric free tier"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0218", "0231", "0111", "0056", "0344"]
tags: [layer-infra, layer-workers, priority-low, effort-small, observability, cloudwatch, cost]
links:
  - "../../../packages/enrichment-worker/src/metrics.rs"
  - "../../../packages/oracle-worker/src/metrics.rs"
  - "../../../packages/rollup-freshness-probe/src/disk.rs"
  - "../../../packages/mtls-notafter-probe/src/lib.rs"
  - "../../../infra/src/lib/stacks/observability-stack.ts"
  - "../../../docs/runbooks/repair-coarse-usd-values.md"
  - "../../../tools/scripts/verify-dashboard-synth.mjs"
history:
  - date: 2026-11-13
    status: backlog
    who: stkrolikiewicz
    note: >
      From the 2026-11-13 read-only CloudWatch review. Re-checked on
      origin/develop (66724665) and live CloudWatch the same day. About $3.7
      (metrics, from the review's hourly-sample count) + $3 (dashboard) a month.
      All but DaysToNotAfter are sent every hour, so each costs the full $0.30.
---

# Stop publishing unread custom metrics; trim the overview dashboard

## Summary

Fifteen custom metric series are published, and nothing reads them: no alarm, no
dashboard widget, no runbook check. Four of them duplicate the free AWS/Lambda
metrics. Separately, the overview dashboard plots 59 metrics, over the 50 that
the free tier allows.

"Nothing reads" was checked three ways on 2026-11-13. Live `describe-alarms`
(155 alarms) reads none of them. The metric arrays of all 3 dashboards name none
of them. `git grep` on develop, outside `lore/1-tasks/archive`, finds only the
publishing code, its doc comments and a comment at `eventbridge-stack.ts:436-437`.

## 1. Coarse-sweep metrics that equal AWS/Lambda (4 series)

`CoarseSweepRuns`, `CoarseSweepFailedRuns`, `CoarseSweepDurationMs`,
`CoarseSweepTablesSkipped`. Live, 2026-10-14..11-13, one 30-day period:

| custom | samples / sum / max | AWS/Lambda (`prices-production-coarse-sweep`) |
|---|---|---|
| CoarseSweepRuns | 720 / 720 / 1 | Invocations sum 720 |
| CoarseSweepFailedRuns | 720 / 0 / 0 | Errors sum 0 (also counts timeouts and init failures) |
| CoarseSweepDurationMs | 720 / 13,385,826 / 55,718 | Duration sum 13,671,610 / max 56,163 |
| CoarseSweepTablesSkipped | 720 / 0 / 0 | the runbook calls it "informational hygiene, not an alarm series" |

- Code: `packages/enrichment-worker/src/metrics.rs:195-238` (Ok path),
  `:255-277` (failure path). Tests assert these names at `:395`, `:400-404`,
  `:427-429` and `:470-473`.
- Update the runbook (`docs/runbooks/repair-coarse-usd-values.md:371-394`,
  "Metrics to watch") so it points the three run states at AWS/Lambda
  Invocations, Errors and Duration.
- Keep `CoarseSweepRowsEnriched`, `CoarseSweepDeadlineHit`, `CoarseSweepTablesSwept`
  and `CoarseSweepTableFailures` ([[0344]] alarms on the last one).
- One edge case: with `COARSE_SWEEP_TABLES` empty, the handler returns Ok and
  publishes nothing. Invocations then reads 1 while Runs is absent. That state
  stays visible, because `RowsEnriched` is absent too and a warning is logged on
  every invocation.

## 2. Metrics nothing reads (10 names, 11 series)

| metric | namespace | publisher (develop) | test that asserts it |
|---|---|---|---|
| EnrichmentFrontierMonth | Prices/Enrichment | `enrichment-worker/src/metrics.rs:136-180` | none |
| EnrichmentFrontierMonthsReopened | Prices/Enrichment | same | none |
| EnrichmentHistoricalDeadlineHit | Prices/Enrichment | same | none |
| EnrichmentHistoricalRowsEnriched | Prices/Enrichment | same | none |
| ClickHouseDiskFreeBytes | Prices/Rollup | `rollup-freshness-probe/src/disk.rs:75` | `disk.rs:311-317` |
| OracleRuns, OracleFailedRuns | Prices/Oracle | `oracle-worker/src/metrics.rs:67-141` | `:223-224` |
| OracleRowsSkipped, OracleSymbolsQueried | Prices/Oracle | same | `:214`, `:219` |
| DaysToNotAfter (Role=api, Role=ingestion) | Prices/Mtls | `mtls-notafter-probe/src/lib.rs:29, :103-127` | none |

- `ClickHouseDiskFreePercent` stays: `ch-disk-free` alarms on it.
- `MinDaysToNotAfter` stays: the mtls-notafter alarm and the dashboard read it,
  and the probe also logs each role's days.
- `OracleRuns` and `OracleFailedRuns` repeat AWS/Lambda Invocations and Errors,
  the same as section 1. `OracleRowsWritten` stays, because dark-feed reads it.
  Fix the state table in the doc comment at `oracle-worker/src/metrics.rs:34-35`.
- `DaysToNotAfter` is sent once a day, so it costs ~$0.01. Drop it to keep the
  catalogue honest, not for the money.

## 3. Dashboard over the free tier

`get-dashboard prices-production-overview`: 29 widgets, **59 distinct metrics**
in metric widgets (AWS/Lambda 30, Prices/Rollup 8, AWS/ApiGateway 6,
Prices/Enrichment 6, Prices/Oracle 4, AWS/SQS 2, and 1 each for Backfill, Mtls
and Ingest). Two alarm-status widgets list 87 alarms and are not counted.

The free tier covers 3 dashboards with up to 50 metrics each. The review's Cost
Explorer read for October 2026 shows `DashboardsUsageHour` $3.00. Cost Explorer
was not re-run here, because each call is billed. None of the metrics in sections
1 and 2 is on the dashboard, so this needs its own cuts, most likely in the 30
AWS/Lambda per-worker series. The dashboard is built at
`observability-stack.ts:479`. Its construct id and name must not change
(`:474`). Re-run `tools/scripts/verify-dashboard-synth.mjs`.

## Acceptance Criteria

- [ ] None of the 15 series above is published after a deploy (`list-metrics`
      stops showing them after 14 days).
- [ ] Unit tests and doc comments are updated. The runbook's "Metrics to watch"
      names the AWS/Lambda metrics.
- [ ] `prices-production-overview` has ≤ 50 distinct metrics. Synth or the
      verify script checks the count.
- [ ] Agreed with the other teams on the shared account before the deploy.
