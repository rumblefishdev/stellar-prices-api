---
id: "0295"
title: "Tranche 3 AC 8 asks for a read-only IAM role giving the Stellar team the CloudWatch dashboard — no such role exists in infra"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0294", "0249", "0214", "0223"]
tags: [layer-infra, priority-high, effort-small, milestone-M3, observability, iam, scf]
milestone: 3
links:
  - "../../../infra/src/lib/stacks/observability-stack.ts"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-09-25
    status: active
    who: stkrolikiewicz
    note: >
      Activated, and RE-SCOPED by the operator: no standing access. Access to
      the account is granted on request from a named person (name, surname,
      e-mail) with MFA enforced, and removed after the review — the model the
      Soroban Block Explorer used for its D3 AC 3 (explorer task 0129, closed
      without implementation; their package says "available on request").
      What this task found: 0125 had already created `prices-production-
      stellar-viewer` (IAM user, 9 read actions, 2026-09-03) and Adam created
      its console login on 2026-09-04 — a standing credential, no MFA, in an
      account with no password policy, the only IAM user in an SSO-only
      account. Login profile deleted 2026-09-25 12:0x CEST; this task removes
      the user from the Observability stack and keeps the scoped read policy
      in the runbook as the template for an on-request grant. The deviation
      (on-request user instead of a standing role) is declared in 0294.
  - date: 2026-09-18
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0294]]. Tranche 3 AC 8 had no owner: a search of
      `infra/src` on 2026-09-18 finds the dashboard but no read-only role, and
      the M2 evidence showed the dashboard by screenshot only.
---

# Read-only dashboard access for the Stellar team

## Summary

Tranche 3 AC 8: _"CloudWatch dashboard accessible to Stellar team (read-only IAM
role); all alarms OK."_ The dashboard exists and is asserted at synth; the
access does not. This task builds the access and records how a reviewer uses it.

## Context

- M2 proved the dashboard with screenshots (`milestone-2-evidence.md` §7.2).
  M3's wording is different — _accessible to the Stellar team_ — and a
  screenshot does not meet it.
- The second clause, **"all alarms OK"**, is a state, not a build item: it has to
  be true on the day of review. 53+ `prices-production-*` alarms exist; the
  stuck-alarm digest ([[0214]]) and the 0223 descriptions make a non-OK alarm
  explainable, but a reviewer will look at the strip.
- The account is shared with soroban-block-explorer. Whatever is granted must be
  scoped to this project's dashboard and metrics, not the account.

## Implementation

- Decide the mechanism and record why: a **cross-account IAM role** (needs the
  Stellar team's AWS account id and an external id — the letter of the AC), or
  **CloudWatch dashboard sharing** (no AWS account needed on their side, but not
  "an IAM role"). If sharing is chosen, declare it as a deviation in [[0294]].
- Least privilege: `cloudwatch:GetDashboard` / `ListDashboards` / `GetMetricData`
  / `DescribeAlarms` and nothing that lists the rest of the account. No wildcard
  resource where a dashboard ARN will do — AC 6 is graded on the same package.
- CDK, in the Observability stack, behind config so staging does not get it.
- Verify as the reviewer would: assume the role (or open the shared link) from
  outside the account and load the dashboard; confirm it cannot read anything
  of the explorer's.
- Write the access instructions for the evidence package's access table.

## Acceptance Criteria

- [ ] The mechanism is chosen and the reason recorded; a deviation is declared
      in [[0294]] if it is not an IAM role
- [ ] Access is defined in CDK and deployed to production
- [ ] Verified from outside the account: the dashboard loads, nothing else does
- [ ] No `resources: ['*']` added without a named reason (AC 6)
- [ ] Access instructions written for the evidence package
- [ ] On the review date every `prices-production-*` alarm is OK, or each
      exception is named with its cause

## Notes

- Needs one input from outside the team: the Stellar team's AWS account id, or
  their preference for a shared link. Ask early — it is the long pole.
