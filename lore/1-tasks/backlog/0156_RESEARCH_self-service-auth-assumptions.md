---
id: "0156"
title: "Confirm the two flagged self-service auth assumptions — Discord account verification and one-key-per-account"
type: RESEARCH
status: backlog
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
model a key collection per user, plus a rotation rule ([[0160]]) that means
something different in each case.

## Implementation

**Question 1 — Discord verification**

- Establish what Stellar's Discord actually requires of a new member today:
  Discord's own account-level email/phone verification, server verification
  level, any onboarding/screening gate, and any role-gating that a fresh account
  cannot clear.
- Note that our OAuth flow may not even see it: Discord OAuth authenticates a
  *Discord account*, not membership of a *server*. If the barrier we are relying
  on is a Stellar-server-level gate, then unless we check guild membership via
  the `guilds` scope we are not benefiting from it at all. **Answer this
  explicitly** — it may be the real finding.
- If the barrier turns out to be weak or invisible to us, do not silently absorb
  the risk: write down the options (require `guilds` membership check, account
  age minimum, lower the free quota further, re-introduce a manual approval for
  the first key) with cost, and let the epic owner pick.

**Question 2 — one active key per account**

- Confirm with the epic owner. Take note that the once-per-calendar-month
  rotation cap ([[0160]]) is only coherent under a one-key model: with multiple
  concurrent keys, AWS's native per-key monthly quota stops bounding a user's
  total consumption, which is precisely the aggregation work the epic's rotation
  rule exists to avoid.

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
- [ ] If the barrier is weaker than assumed — mitigation options costed and a
      decision taken, not just noted
- [ ] One-active-key-per-account confirmed or replaced, with the rotation-cap
      dependency stated
- [ ] ADR written and cross-linked; [[0158]] and [[0160]] updated if the answers
      change their shape

## Notes

- Promote as a **directory** when started — RESEARCH tasks collect notes
  (`Q-`/`R-`/`S-`) per `lore/1-tasks/CLAUDE.md`.
- Deliberately not in scope: the "user leaves the Discord server after issuance"
  case. The epic resolves that as a conscious non-goal — record it in the ADR as
  a decision, not as an open question.
