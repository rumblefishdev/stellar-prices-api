---
title: "Do the two flagged self-service auth assumptions hold?"
type: question
status: mature
spawns:
  - notes/R-discord-platform-verification-mechanics.md
  - notes/R-discord-oauth-observable-signals.md
  - notes/R-stellar-discord-server-posture.md
  - notes/R-apigw-usage-plan-quota-mechanics.md
  - notes/R-abuse-mitigation-options-costed.md
  - notes/S-account-model-and-abuse-barrier.md
tags: [discord, auth, abuse-prevention, account-model, epic-self-service-onboarding]
links:
  - "../../../../../docs/epics/self-service-onboarding.md"
history:
  - date: 2026-08-10
    status: seed
    who: claude
    note: "Root question for 0156 — decomposed into five research lines"
  - date: 2026-08-10
    status: developing
    who: claude
    note: "Research lines dispatched"
  - date: 2026-08-10
    status: mature
    who: claude
    note: "Research complete; sources re-verified and citation slips corrected after audit"
---

# Do the two flagged self-service auth assumptions hold?

## Context

The Self-Service Onboarding epic declares its scope settled except for two
items it marks "confirm before build". Both sit under "Auth & key handling"
and both are load-bearing.

**Assumption 1 — Discord verification is the abuse barrier.** The epic's
argument is a single chain: Discord login is the only barrier to self-issuing a
key, and it suffices *because* Stellar's own Discord requires some form of
verification for new accounts/members, which makes churning disposable accounts
non-trivial. The epic states outright that this is unverified:

> **Unverified assumption (confirm before build):** our understanding is
> Stellar's own Discord requires some form of verification for new
> accounts/members [...] If that verification turns out not to be there, this
> residual risk is bigger than assumed and worth revisiting.

Nothing else in the epic covers the gap — no captcha, no email confirmation, no
manual approval. The assumption carries the whole abuse model alone.

**Assumption 2 — one active key per Discord account.** The epic writes this
twice, inconsistently: as a "**Recommendation** [...] confirm before build"
under "Auth & key handling", and as settled fact under "Out of scope"
("Org/team accounts — one key per Discord account only").

## What would answer this

| # | Sub-question | Answered by |
|---|---|---|
| 1 | What can Discord itself require of a new account, and what can a server require of a new member? | [R-discord-platform-verification-mechanics](R-discord-platform-verification-mechanics.md) |
| 2 | What does *our OAuth flow* actually observe, per scope? | [R-discord-oauth-observable-signals](R-discord-oauth-observable-signals.md) |
| 3 | What is Stellar's Discord posture in practice, and who owns the relationship? | [R-stellar-discord-server-posture](R-stellar-discord-server-posture.md) |
| 4 | Does AWS quota accounting force the one-key model? | [R-apigw-usage-plan-quota-mechanics](R-apigw-usage-plan-quota-mechanics.md) |
| 5 | If the barrier is weaker than assumed, what do the alternatives cost? | [R-abuse-mitigation-options-costed](R-abuse-mitigation-options-costed.md) |

The decomposition matters for question 1. "Does Stellar's Discord verify new
members" and "does our flow see that verification" are different questions, and
the epic conflates them. Discord OAuth authenticates a *Discord account*; guild
membership is a separate API surface behind a separate scope. A barrier that
exists but is invisible to us protects nothing we build.

## Non-goals

- **User leaves the Discord server after issuance.** The epic resolves this as a
  conscious non-goal; the key keeps working. Record as a decision in the ADR,
  not as an open question.
- Revocation. Already tracked as a known gap in the epic and in
  [[0160]] "Open".
