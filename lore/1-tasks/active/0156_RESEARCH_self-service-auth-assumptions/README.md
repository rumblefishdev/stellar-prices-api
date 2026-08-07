---
id: "0156"
title: "Confirm the two flagged self-service auth assumptions — Discord account verification and one-key-per-account"
type: RESEARCH
status: active
related_adr: []
related_tasks: ["0157", "0158", "0159", "0160"]
tags: [layer-docs, priority-high, effort-small, milestone-M3, epic-self-service-onboarding, discord, auth, abuse-prevention, blocks-build]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-08-06
    status: backlog
    who: akot
    note: >
      First task of the Self-Service Onboarding epic. Owns the two items the
      epic marks "confirm before build" — the only unresolved points in an
      otherwise settled scope.
  - date: 2026-08-07
    status: active
    who: akot
    note: >
      Promoted as a directory per the task's own note (RESEARCH collects
      Q-/R-/S- notes). Starts the Self-Service Onboarding epic — 0157-0164
      all wait on the account model this task settles.
---

# Confirm the self-service auth assumptions

## Summary

The Self-Service Onboarding epic declares its scope settled and its open
questions resolved, with exactly two exceptions it flags for confirmation
before anything is built: whether Stellar's Discord actually verifies new
accounts (the load-bearing assumption behind building no captcha/email layer),
and whether one active key per Discord account is the agreed account model.

Both are cheap to answer and expensive to get wrong after the fact — the first
determines whether the abuse story holds, the second is baked into the registry
schema ([[0158]]) and every backend endpoint ([[0160]]).

## Context

The epic's abuse-prevention argument is a single chain: Discord login is the
*only* barrier to self-issuing a key, and it is sufficient **because** throwaway
Discord accounts are non-trivial to churn against Stellar's server. The epic
states plainly that this is an **unverified assumption**, and that if the
verification is not there, "this residual risk is bigger than assumed and worth
revisiting."

Nothing else in the epic covers that gap: there is no captcha, no email
confirmation, no manual approval, and no handling of a user leaving the Discord
server after issuance (a conscious non-goal). So this assumption carries the
whole abuse model on its own.

The second item is smaller but structural. "One active key per Discord account"
is written as a **recommendation** to confirm, not a decision — and it is the
difference between a registry keyed by Discord ID ([[0158]]) and one that has to
model a key collection per user, plus a rework rule ([[0160]]) that means
something different in each case.

## Implementation

**Question 1 — Discord verification**

- Establish what Stellar's Discord actually requires of a new member today:
  Discord's own account-level email/phone verification, server verification
  level, any onboarding/screening gate, and any role-gating that a fresh account
  cannot clear.
- Note that our OAuth flow may not even see it: Discord OAuth authenticates a
  *Discord account*, not membership of a *server*. If the barrier we are relying
  on is a Stellar-server-level gate, then unless we check guild membership we
  are not benefiting from it at all. **Answer this explicitly** — it may be the
  real finding.
- **Establish the server's posture first, then pick the scope.** Doing it the
  other way round produces a consent screen that buys nothing: if the server is
  open and joining is one click, a membership check costs friction and proves
  nothing.
- **If a membership check is warranted, use `guilds.members.read`, not
  `guilds`.** `guilds` returns the full list of servers the user belongs to —
  data we have no reason to see. `guilds.members.read` asks about one named
  guild and returns its member object, with three fields worth more than a
  yes/no:
  - `pending` — `true` while the user has not cleared Membership Screening. If
    Stellar's server has screening on, this *is* the barrier the epic assumes,
    observable directly. If it is always `false`, there is no screening.
  - `joined_at` — how long they have been on the server.
  - `roles` — whatever the server gates behind roles.

  The guild ID becomes configuration (SSM, like the rest), not a constant.
- **Account age is free.** A Discord snowflake encodes its creation timestamp,
  so a minimum-account-age rule needs no extra scope and no extra consent screen
  beyond the `identify` we already request. This makes one mitigation option
  much cheaper than it looks.
- If the barrier turns out to be weak or invisible to us, do not silently absorb
  the risk: write down the options (membership check, account age minimum, lower
  the free quota further, re-introduce a manual approval for the first key) with
  cost, and let the epic owner pick.

**Question 2 — one active key per account**

- **This is lighter than it looks: the epic contradicts itself.** Under "Auth &
  key handling" it is a "**Recommendation** … confirm before build", but under
  **Out of scope** it is stated as settled fact — "Org/team accounts — one key
  per Discord account only". The second reading is also the only one under which
  the rework cap makes sense. Treat this as a one-sentence confirmation with the
  epic owner, not as an open design question.
- Take note that the once-per-quota-period rework cap ([[0160]]) is only
  coherent under a one-key model: with multiple concurrent keys, AWS's native
  per-key monthly quota stops bounding a user's total consumption, which is
  precisely the aggregation work the epic's rework rule exists to avoid.

**Output**

- An ADR in `lore/2-adrs/` recording the account model: Discord identity *is*
  the account, one active key, what the abuse barrier actually is, and what
  would reverse the decision. This is the document [[0158]]/[[0159]]/[[0160]]
  build against.

## Acceptance Criteria

- [ ] Stellar Discord's new-account/new-member verification posture established
      and written down, with the date it was checked
- [ ] Explicitly answered: does our OAuth flow observe that barrier, or only
      Discord account existence?
- [ ] Scope decision recorded as `identify` alone, or `identify` +
      `guilds.members.read` — never `guilds`
- [ ] Owner named for the Stellar Discord relationship and for registering the
      Discord application ([[0159]] currently says "someone")
- [ ] If the barrier is weaker than assumed — mitigation options costed and a
      decision taken, not just noted
- [ ] One-active-key-per-account confirmed or replaced, with the rework-cap
      dependency stated
- [ ] ADR written and cross-linked; [[0158]] and [[0160]] updated if the answers
      change their shape

## Notes

- Promote as a **directory** when started — RESEARCH tasks collect notes
  (`Q-`/`R-`/`S-`) per `lore/1-tasks/CLAUDE.md`.
- Deliberately not in scope: the "user leaves the Discord server after issuance"
  case. The epic resolves that as a conscious non-goal — record it in the ADR as
  a decision, not as an open question.
