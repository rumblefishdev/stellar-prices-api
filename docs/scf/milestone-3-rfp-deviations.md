# Milestone 3 — deviations from the criteria wording, and why

> ⚠️ **DRAFT — skeleton opened 2026-09-25.** Sections marked _to decide_ carry
> the question and the options, not a decision. Milestone 3 is the last tranche,
> so every row here ends as either a closed criterion or a declared deviation —
> there is no later tranche to defer to.

This document is part of the Milestone 3 submission. It sets out each place
where the delivered system departs from the literal wording of a Tranche 3
acceptance criterion (technical design §9), with the measurement behind it and
the reasoning. The evidence package
([`milestone-3-evidence.md`](milestone-3-evidence.md) §4) points here; it does
not repeat this.

## 1. AC 1 asks a reviewer to confirm the backfill is still running

Carried unchanged from [`milestone-2-rfp-deviations.md`](milestone-2-rfp-deviations.md)
§4, where it is set out in full.

**In one paragraph:** the criterion reads _"`GET /backfill/status` shows
`sdex.status: "running"`, `sdex.last_push_at` within the Tranche 3 push-cadence
window, and `sdex.earliest_data_available` ≤ 2018-01-01"_. The archive walked
from the chain tip to genesis during Tranche 2 and reports `completed` since
2026-07-27, at **2015-11-18** — six years beyond the depth clause. A completed
archive that kept pushing would be the defect. The liveness half is graded on
the signals that are live after a backfill: the rollup-freshness alarms, the
ledger-processor lag alarm, and `realtime_tip_ledger` tracking the chain tip.
The design document carries the amendment since 2026-09-08.

### Status

Delivered early, not short. _To fill on submission day:_ the three signals'
states and the `GET /v1/backfill/status` body.

## 2. AC 9 names two metrics that are flat by construction

### The deviation

AC 9 asks the 7-day post-launch report for _"uptime %, error rate, p95 latency,
SDEX push cadence and `earliest_data_available` trajectory"_. The last two are
progress signals of a running backfill. Since 2026-07-27 there is no push
cadence and `earliest_data_available` has been 2015-11-18 on every day.

### What the report carries instead

Signals that answer the same question, _is the data current?_, for a service
whose history is complete. Each is measured against the threshold of the alarm
that watches it:

| AC 9 wording                         | Reported instead                                                                                                                                                                                                                          | Measured                                     |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| SDEX push cadence                    | live ingestion keeps up: age of the oldest ledger notification waiting in the ingest queue (`AWS/SQS ApproximateAgeOfOldestMessage`, `prices-ingest-production`) against 120 s, the threshold of `prices-production-ledger-processor-lag` | every minute of the window                   |
|                                      | prices are recomputed: `current_prices` lag behind the 1-minute source (`Prices/Rollup RollupLagSeconds`) against 900 s, the threshold of `prices-production-current-prices-freshness`                                                    | every 15-minute probe of the window          |
| `earliest_data_available` trajectory | the value at export time, next to `2015-11-18T03:47:00Z` as re-verified on 2026-09-09 for Milestone 2                                                                                                                                     | once, at export                              |
|                                      | `realtime_tip_ledger` against the network's latest ledger (Horizon)                                                                                                                                                                       | once, at export: a point check, not a series |

The other rollup tiers (15m to 1M) are not charted. Their freshness alarms
exist, and any transition they made in the window appears in the report's
alarm history.

Uptime, error rate and p95 are reported as written, from gateway-side metrics,
with the queries beside them. Nothing probes the API from outside, so uptime is
derived from the requests the API served; the report states how few minutes
carried traffic
([`milestone-3-monitoring-report.md`](milestone-3-monitoring-report.md) §2).

### Status

Disclosed here. The report (task 0296, `milestone-3-monitoring-report.md`) is
filled from `docs/scf/milestone-3-monitoring/export.sh` after the window closes
on 2026-09-30 09:40 CEST.

## 3. AC 8: no standing "read-only IAM role" — access on request, per person, with MFA

### The wording

_"CloudWatch dashboard accessible to Stellar team (read-only IAM role); all
alarms OK."_

### The deviation

No standing role or user exists for the Stellar team in `infra/`, by decision
of the operator on 2026-09-25. A read-only IAM **user** is created for a named
reviewer when they ask — first name, surname, e-mail, purpose and end date — with
a one-time password and a policy that denies every read unless the session is
MFA-authenticated; the user is removed after the review
([`docs/runbooks/0295-dashboard-access-on-request.md`](../runbooks/0295-dashboard-access-on-request.md)).

### Why

- **There is no external principal to trust.** A cross-account role needs the
  reviewer's AWS account id; none has been named. A role that trusts nobody is
  not access.
- **The account is shared** with the Soroban Block Explorer and is otherwise
  SSO-only. A standing credential that nobody asked for is exactly what the
  same package's AC 6 (least privilege) argues against; the block explorer's
  Milestone 3 took the same position ("available on request") for its
  equivalent criterion.
- **What was there before was worse than nothing:** task 0125 had shipped a
  standing viewer user with a console login and no MFA (2026-09-03/04). It was
  removed on 2026-09-25 (task 0295), and the synth verifier now refuses any IAM
  identity in the template.

### What a reviewer gets

Exactly the dashboard, its metrics and the alarm states — the nine CloudWatch
read actions of task 0125's scoped policy — and nothing that can read a log
group, a trace, a secret or a Lambda's configuration. The read actions cannot
be scoped per dashboard, so the explorer's dashboard in the same account is
visible too; stated rather than hidden.

### Status

Disclosed. The runbook's create → verify → remove walk is recorded in task 0295
once it has been done on a throwaway name.

## 4. AC 3: which Discord guild gates self-service — _candidate_

### The wording

_"Onboarding portal accessible; self-service API key request flow functional."_
The portal is public and the flow works end to end on the project's **test
Discord guild**. The design intends the Stellar developer guild as the
eligibility gate (task 0179: agreement with the guild's owner, then the switch
and a test).

### Status

If the Stellar guild integration is agreed and switched before submission, this
section is deleted. If not, the criterion is met on the test guild and the
guild switch is declared as a post-delivery step with a name on it.

## How to read this document

Each numbered section is one criterion whose literal wording the delivered
system does not match. "Disclosed" means the departure is a fact and the
package grades the criterion on the stated replacement; "to decide" means the
outcome is open on the date at the top and will be one of the listed options.
Nothing here weakens a criterion silently: every replacement observable is
named, and the measurement behind it is in the evidence package.
