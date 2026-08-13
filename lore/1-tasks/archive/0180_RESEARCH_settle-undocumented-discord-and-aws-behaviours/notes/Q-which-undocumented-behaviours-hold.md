---
title: "Which of the nine undocumented behaviours actually hold?"
type: question
status: seed
spawns:
  - notes/R-discord-member-endpoint-response-shape.md
  - notes/R-apigw-namequery-quota-and-disable.md
  - notes/R-all-in-per-call-cost.md
  - notes/G-measurement-runbook.md
tags: [discord, aws, api-gateway, spike]
links:
  - "../../../../2-adrs/0010_discord-account-model-and-abuse-barrier.md"
  - "../../../archive/0156_RESEARCH_self-service-auth-assumptions/notes/S-account-model-and-abuse-barrier.md"
history:
  - date: 2026-08-12
    status: seed
    who: akot
    note: "Note created on task activation; splits the nine items across three measurement notes"
---

# Which of the nine undocumented behaviours actually hold?

[[0156]] verified every source behind the onboarding design against original
documentation. Nine behaviours the design leans on are **not documented**
anywhere — and two of them are already written into 0157/0158/0160 as though
they were vendor guarantees.

The question this task answers is narrow and per-item: **for each of the nine,
what does the platform actually do, and which already-written text has to
change as a result?**

## The nine, and where each is measured

| # | Behaviour | Note | Corrects existing text? |
|---|---|---|---|
| 1 | Status code + JSON error code when the user is not a guild member | [R-discord](R-discord-member-endpoint-response-shape.md) | No — new branch |
| 2 | Is `pending` present on the REST member response? | [R-discord](R-discord-member-endpoint-response-shape.md) | No — new branch |
| 3 | Is `flags` populated on that response? | [R-discord](R-discord-member-endpoint-response-shape.md) | No — new branch |
| 4 | What `pending` means with screening **off** | [R-discord](R-discord-member-endpoint-response-shape.md) | No — new branch |
| 5 | Consent-screen copy with/without `guilds.members.read` | [R-discord](R-discord-member-endpoint-response-shape.md) | No — friction unknown |
| 6 | `nameQuery` matching — prefix or exact? | [R-apigw](R-apigw-namequery-quota-and-disable.md) | **Yes — 0158, 0160, runbook** |
| 7 | Monthly quota reset instant and timezone | [R-apigw](R-apigw-namequery-quota-and-disable.md) | **Yes — 0157, 0158, 0160** |
| 8 | `UpdateApiKey(enabled=false)` effect on usage counters | [R-apigw](R-apigw-namequery-quota-and-disable.md) | No — unblocks costing revocation |
| 9 | All-in per-call backend cost | [R-cost](R-all-in-per-call-cost.md) | **Yes — ADR 0010's proportionality argument** |

## Why the split is 4 + 3 + 1 + 1

The three measurement notes have genuinely different prerequisites and
different failure modes:

- **Discord (1–5)** needs a registered app, two guilds and two accounts. Nothing
  can be measured until that manual setup exists.
- **AWS (6–8)** needs only a scratch usage plan, and is rate-limited rather than
  blocked — see the throttling warning in the [runbook](G-measurement-runbook.md).
- **Cost (9)** needs neither. It is arithmetic over CloudWatch metrics from the
  already-deployed `prices-api`, not a measurement, and can be done first.

The fourth note is the [runbook](G-measurement-runbook.md): the concrete
ordered steps, kept separate so it can be executed without re-reading the
reasoning.

## What "answered" means here

Not "we found a doc that says so" — the premise of this task is that no such
doc exists. Answered means: **observed, dated, written down, and the
already-written text either sourced or restated as our own decision.**

That distinction is the whole point. A rule we chose is fine. A rule we chose
while believing AWS chose it for us is a trap for whoever reads the task next.
