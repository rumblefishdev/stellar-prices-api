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

## Implementation — re-scoped 2026-09-25: access on request, no standing identity

- **Mechanism, decided by the operator:** no standing role or user. A reviewer
  asks by name (first name, surname, e-mail, purpose and end date); the
  operator creates an IAM user for that person with the scoped dashboard-read
  policy, MFA enforced by an `aws:MultiFactorAuthPresent` condition, a
  one-time password, and deletes the user after the review. Same model as the
  explorer's D3 AC 3 (their task 0129, "available on request"). Declared in
  [[0294]] as the substitution for the criterion's "read-only IAM role".
- **Remove what 0125 left:** `prices-production-stellar-viewer` — the IAM user
  in the Observability stack and its console login (created out of band on
  2026-09-04, no MFA, in an account with no password policy). Login profile
  deleted 2026-09-25 11:57 CEST; the user leaves the stack with this task.
- **Keep the rule in code:** `verify-dashboard-synth.mjs` asserts the template
  creates no `AWS::IAM::User`, `AccessKey` or `LoginProfile` — replacing its
  earlier "exactly one viewer user" check.
- **Runbook:** `docs/runbooks/0295-dashboard-access-on-request.md` — the
  request fields, the create/hand-over/verify/remove commands, and the policy
  (0125's nine CloudWatch read actions + the MFA condition + the self-service
  statements a user needs to enrol a device).
- **Evidence:** AC 8 in `milestone-3-evidence.md` states the dashboard, the
  alarm state and "available on request, named reviewer, MFA"; deviations §4
  declares the substitution.

## Acceptance Criteria

- [x] The mechanism is chosen and the reason recorded; the deviation is
      declared in [[0294]] → on-request named user with MFA; history entry
      2026-09-25; deviations §4 (PR #354)
- [x] No standing identity in `infra/`: the viewer user and its policy are
      removed from the Observability stack, and the synth verifier fails on any
      IAM user, access key or login profile in the template
- [ ] The removal is deployed to production and `aws iam list-users` shows no
      `prices-*` user
- [ ] The runbook was walked once end to end on a throwaway name (create →
      MFA → dashboard renders → log groups denied → remove), with the dates in
      this task
- [ ] Access instructions are in the evidence package's access table
- [ ] On the review date every `prices-production-*` alarm is OK, or each
      exception is named with its cause

## Notes

- ~~Needs one input from outside the team: the Stellar team's AWS account id.~~
  No longer: nothing is built until a named person asks, and then the request
  itself carries every input.
