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

The live-ingestion signals that answer the same question — _is the data
current?_ — for a service whose history is complete: ledger-processor lag
against the chain tip (`realtime_tip_ledger`), rollup freshness per tier, and
the alarms on both. Uptime, error rate and p95 are reported as written, from
gateway-side metrics, with the queries beside them (task 0296).

### Status

Disclosed here; the report (task 0296) is written after the window closes on
2026-09-30 09:40 CEST.

## 3. AC 7: what "works in a fresh AWS account" can mean — _to decide_

### The wording

_"GitHub repository public; `cdk deploy` from README works in a fresh AWS
account."_ The repository is public. The second half has never been rehearsed
(task 0297).

### Why it cannot be literal

A fresh account cannot hold four inputs the stack depends on, none of which our
CDK app can create without secrets it must not have:

| Input                                             | Where it comes from                                                               |
| ------------------------------------------------- | --------------------------------------------------------------------------------- |
| mTLS client certificate and key (Secrets Manager) | issued out of band from the Soroban Block Explorer's CA; never in version control |
| The `prices` tenancy on the shared ClickHouse     | provisioned by the explorer team on their box (explorer tasks 0314, 0567)         |
| The Discord OAuth application bundle              | created in the Discord developer portal; secret value uploaded by the operator    |
| DNS for the custom domain                         | the explorer's zone                                                               |

Milestone 1 graded the sibling criterion (_"from a clean AWS account … no
manual steps"_) exactly this way: the stack deploys end to end; two
prerequisites are operator actions performed once and out of band, documented,
and deliberately not automated.

### Options

1. **Rehearse it.** An empty sandbox account in the organisation; follow the
   README as a stranger; fix every defect found; declare the four inputs as
   documented prerequisites. Result for the package: _works as written, with
   the declared prerequisites_.
2. **Declare it.** No sandbox available in time: state the reason, the
   definition of "works" (synth and deploy succeed with placeholder inputs; a
   serving API needs the four inputs above), and what the README says about
   each.

### Status

_To decide_ — the question is whether a sandbox account can be had before
submission. Either outcome is recorded here and in task 0297.

## 4. AC 8: no standing "read-only IAM role" — access on request, per person, with MFA

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

## 5. AC 3: which Discord guild gates self-service — _candidate_

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
