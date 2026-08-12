---
id: "0180"
title: "Settle the nine undocumented Discord/AWS behaviours the onboarding design depends on"
type: RESEARCH
status: active
related_adr: ["0010"]
related_tasks: ["0156", "0157", "0158", "0159", "0160", "0179"]
tags: [layer-backend, priority-high, effort-small, milestone-M3, epic-self-service-onboarding, discord, aws, api-gateway, spike, blocks-build]
milestone: 3
links:
  - "../../2-adrs/0010_discord-account-model-and-abuse-barrier.md"
  - "../active/0156_RESEARCH_self-service-auth-assumptions/notes/S-account-model-and-abuse-barrier.md"
history:
  - date: 2026-08-10
    status: backlog
    who: akot
    note: >
      Spawned from 0156. Nine behaviours that 0157-0160 currently assume are
      documented, and are not. Two of them are stated in those tasks as if they
      were vendor guarantees. All are minutes-cheap to measure once the Discord
      app and `stellar_test` guild exist.
  - date: 2026-08-12
    status: backlog
    who: akot
    note: >
      Renumbered 0171 → 0180, alongside 0170 → 0179. PRs #196 and #197 landed a
      different 0171 (the `Decimal128::MIN` bug) on develop before PR #187
      merged this one, so both numbers were taken. The rename matters more here
      than for 0179: `[[0171]]` is cited by id in 0158, 0160, ADR 0010 and the
      archived 0157, always meaning "the spike measures this" — and the bug it
      collided with measures nothing.
  - date: 2026-08-12
    status: active
    who: akot
    note: >
      Activated after 0157 completed. First task of the epic's remaining set:
      `blocks-build` and effort-small, and two of its findings are corrections
      to text already written into 0158/0160 — so it runs before the registry
      and the endpoints, not after. Manual prerequisite is on Adam: the Discord
      application registered and the `stellar_test` guild configured.
  - date: 2026-08-12
    status: active
    who: akot
    note: >
      Converted file → directory and scaffolded `notes/`: one Q-, three R- (the
      nine items split by prerequisite, not by topic) and a G- runbook. Two
      things surfaced while writing them. Item 4 needs a *second* guild with
      screening off, and item 1 a *second* account that is not a member —
      neither was stated. And item 7 cannot be fully measured before
      1 September; the runbook proxies it with a `DAY`-period plan, which
      settles the wording without proving AWS's `MONTH` implementation.
---

# Settle the undocumented behaviours before building on them

## Summary

0156 verified every source behind the onboarding design against the original
documentation. Seven behaviours the design leans on turn out to be
**undocumented**, and two of them appear in already-written tasks as though they
were vendor guarantees.

None is hard to settle. All are a short spike against the `stellar_test` guild
(ADR 0010) and a scratch AWS usage plan. The point of filing it as a task is
that they are currently invisible risks: the tasks read as though these facts
were sourced.

## Context

This is not speculative hardening. Two items are **corrections to text already
written into 0157/0158/0160**, and the rest are load-bearing branches in code
nobody has written yet — the cheapest possible moment to check them.

## Layout

Converted to directory format on activation (2026-08-12) — nine measurements
need somewhere to land, and RESEARCH tasks are directories by convention.

| Note | Covers | Prerequisite |
| --- | --- | --- |
| [Q-which-undocumented-behaviours-hold](notes/Q-which-undocumented-behaviours-hold.md) | The question, and the 4+3+1+1 split | — |
| [R-discord-member-endpoint-response-shape](notes/R-discord-member-endpoint-response-shape.md) | Items 1–5 | Discord app, **two** guilds, **two** accounts |
| [R-apigw-namequery-quota-and-disable](notes/R-apigw-namequery-quota-and-disable.md) | Items 6–8 | Scratch usage plan |
| [R-all-in-per-call-cost](notes/R-all-in-per-call-cost.md) | Item 9 | **None — can run first** |
| [G-measurement-runbook](notes/G-measurement-runbook.md) | Ordered steps, who does what | — |

## Prerequisites (manual, owned by Adam)

Nothing Discord-related exists in the repo yet — `api-gateway-stack.ts:396`
mentions it only in a comment, no SSM parameters, no secrets. Needed before
items 1–5 can be touched:

- Discord application registered; scopes `identify` + `guilds.members.read`
- `stellar_test` guild with Membership Screening **on**
- **A second scratch guild with screening off** — item 4 is a comparison
- **A second account that is not a member** — item 1 is unmeasurable without it

## The nine

Detail, reasoning and result tables live in the `R-` notes above. In short:

| # | Behaviour | Correction? |
| --- | --- | --- |
| 1 | Status code + JSON error code when the user is **not** a member | new branch |
| 2 | Is `pending` present on the REST member response? | new branch |
| 3 | Is `flags` populated on that response? | new branch |
| 4 | What `pending` means with screening **off** | new branch |
| 5 | Consent-screen copy with/without `guilds.members.read` | friction unknown |
| 6 | `nameQuery` matching — prefix or exact? | **yes — 0158, 0160, runbook** |
| 7 | Monthly quota reset instant and timezone | **yes — 0157, 0158, 0160** |
| 8 | `UpdateApiKey(enabled=false)` effect on usage counters | unblocks costing revocation |
| 9 | All-in per-call backend cost | **yes — ADR 0010's $0.38 argument** |

Items **6 and 7** are corrections to text already written into 0157/0158/0160 as
though AWS guaranteed it; item **9** re-checks ADR 0010's proportionality
argument rather than any task's wording. Item 1 carries the highest stakes: fail
closed on a `429` and legitimate users are denied a key; fail open on a `404`
and the abuse barrier is void.

## Sequencing

Item 9 needs no setup at all — it is arithmetic over CloudWatch metrics from the
deployed `prices-api`, so it runs **first**, before the Discord prerequisites
land. Items 6–8 need only a scratch usage plan. Items 1–5 wait on Step 0 of the
[runbook](notes/G-measurement-runbook.md).

One honest limit, recorded up front: **item 7 cannot be fully measured before
1 September 2026** — that is the next real `MONTH` rollover. The runbook uses a
`DAY`-period plan as a proxy for the instant and timezone. That is strong
evidence, not proof, and it is enough: the criterion is to stop presenting the
rule as AWS-documented, not to prove AWS's implementation.

## Acceptance Criteria

- [ ] Items 1-4 measured against `stellar_test` and a screening-off scratch
      guild, results written down with the date
- [ ] Consent screen captured with and without `guilds.members.read`
- [ ] `nameQuery` semantics measured; [[0158]], [[0160]] and
      `docs/runbooks/manual-api-key-tier.md` updated so the client-side filter is
      documented as load-bearing rather than redundant
- [ ] Quota rollover instant measured; [[0157]]/[[0158]]/[[0160]] corrected to
      stop presenting "00:00 UTC on the 1st" as AWS-documented behaviour
- [ ] `enabled=false` effect on usage counters measured and recorded against
      [[0160]] "Open"
- [x] All-in per-call cost established and ADR 0010's proportionality argument
      re-checked against it — **done 2026-08-12**, $0.55–$0.89 per fully-drained
      key (1.4–2.3× the $0.38), argument survives. See
      [R-all-in-per-call-cost](notes/R-all-in-per-call-cost.md)
- [ ] Any finding that changes ADR 0010's shape reflected in the ADR

## Notes

- Do the AWS items on a scratch usage plan. `UpdateUsagePlan` is throttled to
  **1 request every 20 seconds per account, non-adjustable**, and the whole
  control plane shares a **10 rps / burst 40** budget with our deploys — a
  careless loop here slows CI down for everyone.
- `UpdateUsage` (`op:replace` on `/remaining`) moves the quota counter directly
  without touching the key. Useful for constructing test states cheaply, and
  worth knowing about independently of this spike.
