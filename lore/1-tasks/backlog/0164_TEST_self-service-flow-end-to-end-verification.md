---
id: "0164"
title: "Self-service flow — end-to-end verification against production and Tranche 3 evidence"
type: TEST
status: backlog
related_adr: ["0010"]
related_tasks: ["0156", "0157", "0158", "0159", "0160", "0161", "0162", "0163", "0179", "0180"]
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
  - date: 2026-08-10
    status: backlog
    who: akot
    note: >
      Rewritten where ADR 0010 broke it. "Run from a genuinely fresh account"
      would now fail by design — the account-age minimum refuses new accounts —
      so the run needs two outsider accounts and three new checks (non-member
      refusal, too-young refusal, and Discord-unavailable not presenting as
      not-a-member). Also corrected the `nameQuery` prefix claim.
  - date: 2026-08-10
    status: backlog
    who: akot
    note: >
      Sequencing recorded: this now runs after [[0179]], not simply "last in
      the 0156–0164 set". The evidence must be gathered against the real guild
      — a flow gated on our private `stellar_test` guild is not functional for
      any outside developer, which is what the Tranche 3 criterion asks about.
      Added a rehearsal pass on `stellar_test` before the evidence pass.
---

# Self-service flow — end-to-end verification

## Summary

Walk the whole path as an outsider would, against production, and write down
what happened. The epic's five acceptance criteria are all statements about the
deployed system, not about code — none of them can be closed by a unit test.

## Sequencing — this runs LAST, after [[0179]]

**This task must not run until [[0179]] has flipped the production SSM guild ID
from `stellar_test` to `897514728459468821`.** Recorded 2026-08-10; the original
"last task in the 0156–0164 set" framing predates [[0179]] existing.

The reason is not tidiness. This task produces the **Tranche 3 evidence** that
the submission cites, against a reviewer criterion that reads *"self-service API
key request flow functional"*. Development and the earlier tasks run against a
private `stellar_test` guild (ADR 0010) — and a flow gated on membership of a
guild only we can join is **not functional for any outside developer**. Evidence
gathered in that configuration would certify a door nobody else can open.

The same fact bounds the launch itself: until the SSM flip, no external user can
obtain a key at all. The flip precedes any announcement, not the other way
round.

**Run it twice.** Once against `stellar_test` as a rehearsal, to find breakage
cheaply while the config is still ours to change; then again after the flip, as
the evidence run. The second pass is fast because everything already works, and
only the second one goes in `docs/scf/`.

Order for the whole epic, for whoever picks this up cold:

```
0156 ✅ → 0157 · 0158 · 0161  (no Discord at all)
       → Discord app + stellar_test → 0171 → 0159
       → 0160 → 0162 → 0163
       → 0170  (SDF contact, verify real guild, FLIP SSM)
       → 0164  (evidence, against the real guild)
```

## Context

The reviewer's sign-off wording is *"Onboarding portal accessible at documented
URL; self-service API key request flow functional"*. That is one person opening
a URL and getting a working key. This task is the rehearsal, done before they
do it, with the transcript kept.

Precedent for the artefact: `docs/scf/milestone-1-evidence.md` and the running
record `docs/scf/api-endpoints.md` that [[0124]] started. The output here feeds
whatever Tranche 3 package is assembled, the way [[0128]] does for M2.

## Implementation

**Run the flow from an outsider's account, not a maintainer's.** The path a
stranger takes is the path that gets tested.

**Rewritten 2026-08-10 — "genuinely fresh account" is now wrong.** ADR 0010 adds
a **minimum Discord account age**, so a brand-new account is *supposed* to be
refused: the original instruction would have produced a failing run and read as
a bug. It also said this run would "put a number on [[0156]]'s assumption" —
[[0156]] is answered, and the answer is that the barrier is now something we
enforce ourselves rather than something we observe passively.

Two accounts are therefore needed, and neither is a maintainer's:

| Account | Purpose |
|---|---|
| **A** — outsider, member of the guild, older than the age threshold | The happy path. Every check below unless stated |
| **B** — outsider, **not** a member, and newer than the threshold | The two refusal paths |

Account B exercises both refusals, but **test them one at a time** — join the
guild to isolate the age refusal, or age past the threshold to isolate the
membership refusal. Otherwise a pass proves only that *something* refused.

**The age threshold is 5 minutes** (ADR 0010), which makes check 10 a race
rather than a scenario: account B must be used **within five minutes of
creation** or the age refusal cannot be observed at all. Create B immediately
before running check 10, and if the window is missed, raise the SSM threshold
temporarily rather than waiting for a new account — but **put it back**, and
record that you did.

**Checks, each mapped to an epic AC:**

| # | Check | Epic AC |
| --- | --- | --- |
| 1 | Portal loads over TLS at the documented URL; Swagger UI renders the live spec | 1 |
| 2 | Discord sign-in (account A) → key issued → key returns `200` from a data route | 2 |
| 3 | Same request without `x-api-key` returns `403` | 2 |
| 4 | Signing in again shows the same key, not a new one | 2 |
| 5 | Quickstart commands run verbatim and return data | 3 |
| 6 | Dashboard's usage figure moves after a burst of calls and matches `GetUsage` | 4 |
| 7 | Sustained calls hit `429` at the documented rate; burst of 5 does not | 5 |
| 8 | Quota shown on the dashboard equals the plan's configured monthly quota | 5 |
| 9 | **Non-member (B) is refused with a message naming the Discord server and how to join** — not a generic error, and no key is issued | ADR 0010 |
| 10 | **Below-threshold account is refused with the time remaining** (not a date), can retry once it passes, and no key is issued meanwhile | ADR 0010 |
| 11 | **Discord unavailable ≠ not a member.** Force a non-404 failure from the member lookup (bad token, throttle) and confirm the user is *not* told they are not a member, and that no key is issued | ADR 0010 |
| 12 | **A user who leaves the guild keeps their key and their dashboard** — the key still returns `200`, reveal and usage still work. Indefinitely; the key never expires | ADR 0010 §8 |
| 13 | **That same user is refused on rework**, with a message naming the server — membership is required at rework time, and only there | ADR 0010 §8 |
| 14 | **A valid session alone cannot mint or rework a key.** Call the issue and rework endpoints directly with only a session cookie; both must refuse. This is the check that proves the gate is actually enforced rather than assumed | ADR 0010 §8 |

Check 11 is the one most likely to be skipped and the most damaging to get
wrong: the membership test is a *negative* inferred from an undocumented error
shape ([[0180]] #1), so "Discord returned 429" must never read as "you are not a
member".

**Checks that are ours rather than the epic's**, because they are the ways this
fails quietly:

- Two parallel first sign-ins converge on **one** key. Note that this verifies
  the reconciler, not a store guarantee: ClickHouse has no conditional insert
  ([[0158]] "Accepted consequences"), so the correct outcome is "two keys may be
  created, one survives and the loser is deleted", not "only one is ever
  created". Check the surviving key works and the deleted one returns `403`.
- The standing "users holding more than one key" query returns empty after the
  run.
- **A user id that is a prefix of another user's id does not touch that other
  user's key.** Discord snowflakes are 17–19 digits, so one user id can be a
  prefix of another; this is the failure mode [[0158]]'s exact-match filter
  exists to stop, and it is silent and unrecoverable when it fires. Set up two
  keys whose names stand in a prefix relationship, run issue and rework for the
  shorter id, and confirm the longer id's key is neither adopted nor deleted.
  Without this check a regression to a bare `nameQuery` ships unnoticed.

  **Corrected 2026-08-10 ([[0156]]): `nameQuery` is not documented as a prefix
  match** — AWS states no matching semantics for it at all. That makes this
  check *more* important, not less: we are relying on undocumented behaviour
  staying whatever it currently is, so this is the test that would catch AWS
  changing it under us. [[0180]] measures the actual behaviour.
- **A key issued inside the current quota period cannot be reworked.** Distinct
  from the check below: that one covers `last_rotated_at`, this one covers the
  `created_at` fallback, which is the case the original gate let through. Issue a
  fresh key, attempt a rework the same period, expect `409`.
- **A hand-deleted key recovers on the reveal path.** Delete a key in the console
  without touching the registry, then load the dashboard: the user gets a working
  key back, not a dead id. Verifies the recovery sits on reveal rather than on
  issuance, where the populated `api_key_id` short-circuits it ([[0160]] Settled
  #4).
- **The session cookie survives CloudFront.** A signed-in request reaches the
  origin still signed in. CloudFront's managed cache policy strips cookies by
  default, so getting this wrong presents as "every user is permanently signed
  out" with nothing wrong at the gateway ([[0160]], [[0161]]).
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

- [ ] All fourteen mapped checks executed against production and recorded
      with dates
- [ ] The evidence run was performed **after** [[0179]]'s SSM flip, against
      guild `897514728459468821` — and the evidence file states which guild it
      ran against, so a reader can tell
- [ ] If the SSM age threshold was temporarily raised to observe check 10, the
      restore is recorded and verified
- [ ] The quiet-failure checks executed and recorded, including the reconciler
      convergence and the `delete-key` rework path
- [ ] Run performed with two non-maintainer Discord accounts per the table
      above — one eligible, one exercising each refusal in isolation
- [ ] The two refusals (not a member, account too young) each produce a
      specific, actionable message and issue no key
- [ ] A Discord outage or throttle is shown **not** to present as "not a
      member", and to issue no key
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
