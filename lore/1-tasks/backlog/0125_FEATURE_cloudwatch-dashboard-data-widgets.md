---
id: "0125"
title: "CloudWatch dashboard — replace the empty prices-production-overview scaffold with real data widgets"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0056", "0093", "0121", "0128"]
tags: [layer-infra, priority-medium, effort-medium, milestone-M2, observability, cloudwatch, dashboard]
milestone: 2
links:
  - "../../../docs/scf/milestone-1-evidence.md"
  - "../../../infra/src/lib/stacks"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Closes the "CloudWatch
      dashboard" row of `milestone-1-evidence.md` Table 4 — the M1 submission
      states `prices-production-overview` exists as "a scaffold with no data
      widgets" and explicitly does not offer it as evidence.
---

# CloudWatch dashboard with real data widgets

## Summary

`prices-production-overview` is deployed but empty. The M1 evidence document
says so directly and declines to screenshot it. The seven alarms behind it are
real and fire-tested (task 0056) — it is only the dashboard that is a shell.

Tranche 3 AC 8 eventually requires *"CloudWatch dashboard accessible to Stellar
team (read-only IAM role); all alarms OK"*, so building it in M2 both closes the
M1 promise and de-risks M3.

## Context

§9 Tranche 3 lists the dashboard content as *"API latency, error rate, ingestion
lag, ClickHouse write latency, mTLS cert NotAfter, backfill progress"* — a good
starting widget list.

The useful constraint: the dashboard should make the **M2 acceptance criteria**
observable, not just be a wall of graphs. If a reviewer cannot see p95 latency,
error rate, and cache hit rate on one screen, it does not serve [[0121]] or
[[0122]].

**The hard part is the non-AWS metrics.** API Gateway, Lambda and alarm state
come free from CloudWatch. ClickHouse-side numbers (write latency, query time,
enrichment lag, backfill frontier) live on **Hetzner**, behind mTLS, on a box
BE owns — there is no metric stream into CloudWatch today. Options, cheapest
first:

1. Extend the existing probe Lambdas (`backfill-freshness-probe`,
   `mtls-notafter-probe` — both already scheduled and already speaking mTLS to
   CH) to emit `PutMetricData` custom metrics. Reuses a proven path; adds a
   small per-metric cost.
2. A new dedicated metrics-probe Lambda. Cleaner separation, more moving parts.
3. Skip CH-side metrics on the dashboard and link out. Cheapest, weakest.

**Recommended: (1)** — the probes already run on a schedule, already hold the
certs, and already query the tables the numbers come from.

## Implementation

- Define the widget set against the §9 list plus the M2 ACs:
  - **API** — request count, p50/p95/p99 latency, 4xx/5xx rate, throttles,
    cache hit/miss ratio ([[0122]])
  - **Lambda** — duration, errors, throttles, concurrency, cold starts, per
    function (api-handler and the workers)
  - **Ingestion** — ledger-processor lag (an alarm already exists at 120s),
    SQS depth / DLQ, invocation errors
  - **ClickHouse** — write latency, query latency, enrichment lag, USD-coverage
    percentage (the 0114 metric worth watching permanently)
  - **Backfill** — `earliest_data_available` and `last_push_at` trajectory, so
    the [[0127]] depth milestone is visible over time rather than sampled
  - **Alarm status strip** — all seven alarms, current state
- Emit whatever custom metrics the above needs via the chosen option; keep the
  metric namespace and dimensions consistent with BE's conventions.
- Define the dashboard in **CDK**, not by hand in the console — it must survive
  a redeploy, and Tranche 3 AC 7 requires `cdk deploy` from a clean account to
  reproduce everything.
- Sensible default time range and periods; a dashboard defaulting to 1h hides
  the backfill trajectory, one defaulting to 2 weeks hides a latency spike.
  Consider splitting real-time vs trend rows.
- Provide the read-only IAM role for external (Stellar) access now — M3 needs
  it, and it is a few lines.

## Acceptance Criteria

- [ ] `prices-production-overview` renders real data in every widget; no empty
      panels
- [ ] Every §9-listed topic has a widget: API latency, error rate, ingestion
      lag, ClickHouse write latency, mTLS NotAfter, backfill progress
- [ ] Cache hit rate and p95 latency are visible on one screen (serves
      [[0121]] / [[0122]])
- [ ] ClickHouse-side metrics reach CloudWatch by a documented mechanism;
      the chosen option is recorded with its cost
- [ ] Alarm-status widget shows all seven alarms and their current state
- [ ] Dashboard is defined in CDK and survives a redeploy
- [ ] Read-only IAM role for external viewers exists and is documented
- [ ] Screenshot captured for [[0128]] — this is the evidence M1 could not give

## Notes

- Custom metrics are billed per metric per month. Keep the set deliberate; a
  per-asset metric explosion is the easy mistake here.
- [[0093]] (freshness alarms for backfill + live) overlaps on the probe path.
  Whichever lands first should leave the metric-emission hook in place for the
  other.
