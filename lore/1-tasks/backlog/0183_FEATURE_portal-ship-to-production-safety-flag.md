---
id: "0183"
title: "Ship-to-production safety — portal feature flag and allowlist, because there is no test environment"
type: FEATURE
status: backlog
related_adr: ["0007", "0010"]
related_tasks: ["0157", "0184", "0185", "0186", "0187", "0188", "0189", "0191", "0192", "0194"]
tags: [layer-infra, priority-high, effort-small, milestone-M3, epic-self-service-onboarding, feature-flag, ssm, safety, slice-0]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Added at the re-slice, and it is the thing the original 0158–0162 set
      missed entirely. There is one environment: `envName` is typed
      `'production'` and `infra/envs/` holds only `production.json`, so every
      `cdk deploy` lands on production. Twelve slices that each ship something
      half-built therefore need a way to be invisible until they are finished,
      and a way to be exercised by us in the meantime. First in the order,
      before hosting.
---

# Ship-to-production safety — flag and allowlist

## Summary

**Story:** *as the operator, I can land an unfinished slice on production
without a stranger being able to reach it — and still walk the whole flow myself
against the real stack.*

Two SSM parameters and a middleware. Everything else in the epic depends on it
existing first.

## The problem it solves

**There is no staging.** `envName` is typed `'production'`, `infra/envs/` holds
only `production.json`, and the archived [[0159]] already noted in passing that
this means "one parameter whose value is flipped in place, not a per-environment
matrix". Nobody drew the consequence for the portal: **a deploy is a release.**

Concretely, what lands on production without this task:

| Slice | What is publicly reachable the moment it deploys |
| --- | --- |
| [[0184]] / [[0185]] | A half-built page on the URL we are about to put in a Tranche 3 submission |
| [[0186]] | A live OAuth callback on the production hostname |
| **[[0187]]** | **Anyone with a Discord account minting a real key on the real usage plan** — the eligibility gate is three slices later |
| [[0187]] | A reconciler running `DeleteApiKey` against production keys, with the snowflake prefix hazard live |
| [[0191]] / [[0192]] | Endpoints that delete production keys |

The third row is the one that matters. An earlier draft of [[0187]] said the gap
was "fine on the dev distribution" — **there is no dev distribution**, and that
sentence is corrected in that task.

## Implementation

**Two parameters, operator-seeded, read at runtime.** Same contract as
`min-account-age-minutes` in [[0189]] and the mTLS material: never
`new ssm.StringParameter`, read through the SSM SDK and not
`valueForStringParameter`. `lambda-baseline.ts` already grants
`ssm:GetParameter*` on `arn:…:parameter/prices/{env}/*`, so there is no IAM work.

| Parameter | Default | Meaning |
| --- | --- | --- |
| `/prices/{env}/portal-enabled` | `false` | Master switch for every `/api-tokens/api/*` route |
| `/prices/{env}/portal-allowlist-discord-ids` | *(empty)* | Comma-separated Discord user IDs that bypass the switch |

**Why SSM and not a CDK context flag.** A context flag can only be flipped by a
deploy, and a deploy is exactly what we are trying to make safe. The kill switch
has to be faster than a rollback. It also has to survive the next `cdk deploy`,
which a CloudFormation-managed parameter would not — the same trap [[0189]]
documents for the guild id.

**Behaviour when off:**

- Every `/api-tokens/api/*` route returns **`404`**, not `403` and not `503`. A
  `503` advertises that something is there and invites someone to come back; a
  `404` is indistinguishable from a route that does not exist. The one exception
  is `GET /api-tokens/api/config` below.
- **Except for an allowlisted Discord ID**, for whom every route behaves
  normally. This is what replaces the missing test environment: the flow is
  walkable end-to-end on the real stack by us, and invisible to everyone else.
  Without it the flag would only mean "nothing is testable until everything is
  finished", which is the failure the re-slice exists to avoid.
- The allowlist is checked against the **session** ([[0186]]), so sign-in itself
  has to work while the flag is off. Sign-in therefore checks the allowlist
  *after* resolving the identity, and refuses at that point.
- **The static bundle stays public.** It holds no secret and makes no privileged
  call, so the exposure is reputational, not a security matter, and gating it
  would need a CloudFront Function for no benefit. Instead the app drives itself
  from **`GET /api-tokens/api/config`** — the one route that answers while the
  flag is off — which reports whether the portal is open. Closed → the page says
  so and renders no sign-in button.

**Consequence of issuing into the real usage plan** (decided 2026-08-13 — no
separate "incubation" plan): keys minted while the flag was off are real keys on
the real free-tier plan, counted in its accounting. They have to be cleaned up
rather than left to be discovered. That is an acceptance criterion here and a
check in [[0194]].

**Who turns it on.** Flipping `portal-enabled` to `true` is an explicit
acceptance criterion of [[0194]] (the audit), and is gated on [[0189]] (the
eligibility gate) having passed. Nobody flips it as a side effect of finishing
their own slice.

**Every slice that deploys inherits one line of acceptance criteria:**

> - [ ] With `portal-enabled=false`, this slice's routes return `404` to a
>       non-allowlisted caller, and behave normally for an allowlisted one

## Acceptance Criteria

- [ ] Both parameters are operator-seeded and read at runtime; flipping either
      takes effect **without a redeploy**
- [ ] Neither is created by CDK — a `cdk deploy` does not reset the switch
- [ ] With the flag off, every `/api-tokens/api/*` route returns `404` to a
      non-allowlisted caller, `GET .../config` excepted
- [ ] With the flag off, an allowlisted Discord ID completes every flow normally
- [ ] `GET /api-tokens/api/config` answers in both states and never leaks the
      allowlist
- [ ] The static page renders "not yet available" and no sign-in button while
      the portal is closed
- [ ] A procedure exists — and is tested once — for enumerating and deleting
      keys created while the flag was off, so build-period keys do not survive
      into the real plan's accounting
- [ ] Both parameter names and the seeding step are in the deploy-prep runbook,
      alongside the mTLS material and [[0189]]'s two parameters
- [ ] The single-environment fact is written into the epic doc, so the next
      person does not assume a staging deploy exists

## Notes

- This is the cheapest task in the epic and the one whose absence is most
  expensive. Roughly two parameters, one middleware and a runbook entry.
- It also gives the rollback story for everything after it: if a slice
  misbehaves in production, the first move is flipping one SSM value, not
  reverting a stack.
- Worth revisiting once, later: if the flag is still off by [[0195]]'s custom
  domain, the Discord redirect URI has been pointed at a closed portal for
  weeks. Harmless, but confirm sign-in on the new hostname with an allowlisted
  account before advertising it.
