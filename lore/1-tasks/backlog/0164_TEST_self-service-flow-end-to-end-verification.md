---
id: "0164"
title: "Self-service flow — end-to-end verification against production and Tranche 3 evidence"
type: TEST
status: backlog
related_adr: []
related_tasks: ["0156", "0157", "0158", "0159", "0160", "0161", "0162", "0163"]
tags: [layer-test, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, verification, scf-evidence]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../../docs/scf/api-endpoints.md"
history:
  - date: 2026-08-06
    status: backlog
    who: akot
    note: >
      Closes the epic. Verifies all five of its acceptance criteria against the
      deployed system and produces the artefacts a Tranche 3 submission cites.
      Last task in the 0156–0164 set.
  - date: 2026-08-07
    status: backlog
    who: akot
    note: >
      Updated after the 2026-08-07 meeting: rework is in scope so its checks are
      unconditional, and the concurrency check now verifies the reconciler
      rather than a store-level guarantee, which ClickHouse cannot give.
---

# Self-service flow — end-to-end verification

## Summary

Walk the whole path as an outsider would, against production, and write down
what happened. The epic's five acceptance criteria are all statements about the
deployed system, not about code — none of them can be closed by a unit test.

## Context

The reviewer's sign-off wording is *"Onboarding portal accessible at documented
URL; self-service API key request flow functional"*. That is one person opening
a URL and getting a working key. This task is the rehearsal, done before they
do it, with the transcript kept.

Precedent for the artefact: `docs/scf/milestone-1-evidence.md` and the running
record `docs/scf/api-endpoints.md` that [[0124]] started. The output here feeds
whatever Tranche 3 package is assembled, the way [[0128]] does for M2.

## Implementation

**Run the flow from a genuinely fresh account.** Not a maintainer's own Discord
login — a new account, so the path a stranger takes is the path that gets
tested. This also puts a number on [[0156]]'s assumption: whatever friction a
new account meets is exactly the abuse barrier the epic is relying on, and this
is the moment we see it directly.

**Checks, each mapped to an epic AC:**

| # | Check | Epic AC |
| --- | --- | --- |
| 1 | Portal loads over TLS at the documented URL; Swagger UI renders the live spec | 1 |
| 2 | Discord sign-in → key issued → key returns `200` from a data route | 2 |
| 3 | Same request without `x-api-key` returns `403` | 2 |
| 4 | Signing in again shows the same key, not a new one | 2 |
| 5 | Quickstart commands run verbatim and return data | 3 |
| 6 | Dashboard's usage figure moves after a burst of calls and matches `GetUsage` | 4 |
| 7 | Sustained calls hit `429` at the documented rate; burst of 5 does not | 5 |
| 8 | Quota shown on the dashboard equals the plan's configured monthly quota | 5 |

**Checks that are ours rather than the epic's**, because they are the ways this
fails quietly:

- Two parallel first sign-ins converge on **one** key. Note that this verifies
  the reconciler, not a store guarantee: ClickHouse has no conditional insert
  ([[0158]] "Accepted consequences"), so the correct outcome is "two keys may be
  created, one survives and the loser is deleted", not "only one is ever
  created". Check the surviving key works and the deleted one returns `403`.
- The standing "users holding more than one key" query returns empty after the
  run.
- No API key value appears in CloudWatch logs or X-Ray traces from any portal
  route.
- Portal responses are not served from the gateway cache — repeat a key-reveal
  call from two sessions and confirm each gets its own.
- Rework refused a second time within the period, returning `409` with the next
  eligible date. Verify the meeting's worked example: reworked on 3 August →
  refused until 1 September.
- The rework modal will not confirm until `delete-key` is typed, and the old key
  returns `403` immediately after.

**Evidence to keep:** the curl transcripts, the dashboard screenshots, the
documented URL, and the date each was taken. Store under `docs/scf/`, and
record any check that could not be run and why — a gap named is evidence; a gap
omitted is a finding waiting for the reviewer.

## Acceptance Criteria

- [ ] All eight epic-mapped checks executed against production and recorded
      with dates
- [ ] The quiet-failure checks executed and recorded, including the reconciler
      convergence and the `delete-key` rework path
- [ ] Run performed with a fresh Discord account, and the friction it met
      written down for [[0156]]
- [ ] Evidence file in `docs/scf/`, with the portal URL and every command a
      reviewer can repeat
- [ ] Any deferred or unrunnable check named explicitly, with the reason
- [ ] Epic ACs 1–5 each marked satisfied or explicitly not

## Notes

- Run this **after** a real deploy, not against a local stack — every criterion
  is about the deployed system, and the [[0124]] experience is that the
  deployment-only half is the half that slips.
- The fresh-account run is destructive in one direction: it creates a real key
  on the production plan. Note the Discord ID used so the record can be cleaned
  up or kept deliberately as a test account.
