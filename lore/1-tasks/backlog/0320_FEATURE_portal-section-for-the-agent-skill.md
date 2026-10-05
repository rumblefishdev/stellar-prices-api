---
id: "0320"
title: "Portal section for the agent skill — a landing box to hand the skill to an agent, and a dashboard line that sends an agent-referred visitor back with the key"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0318", "0316"]
tags: [layer-frontend, portal, marketing, agents, epic-self-service-onboarding, priority-low, effort-small]
links:
  - "../active/0318_FEATURE_agent-skill-for-the-prices-api-on-skills-stellar-org.md"
  - "../../../web/portal/src/app/app.tsx"
  - "../../../web/portal/src/landing/Documentation.tsx"
history:
  - date: "2026-09-29"
    status: backlog
    who: stkrolikiewicz
    note: >
      Split out of 0318: the skill and its skills.stellar.org listing stay
      there; showing the skill on our own portal is this task. Wait for 0318's
      listing, because the section links to it.
---

# Portal section for the agent skill

## Summary

Show the prices-API agent skill (0318) on the portal. There are two places to
do it, and one of them closes the agent funnel.

## Context

The skill sends a human to `https://sorobanscan.rumblefish.dev/api/?utm_source=stellar-skill`
when their agent has no key. Today that visitor signs in, gets a key and lands
on a dashboard that says "Welcome! Your API key has been generated… the Quick
Start guide has everything you need" (`web/portal/src/app/app.tsx` ~2779).
Nothing there tells them to go back to the agent that sent them.

## Proposal (from the 0318 discussion, not yet decided)

1. **Dashboard line (closes the funnel).** Under the welcome text, add: "Came
   here from an AI agent? Copy the key, set it as `STELLAR_PRICES_API_KEY` and
   go back to your agent." The variable name must match the skill.
2. **Landing box "For AI agents"** between `<Documentation />` and `<Faq />`.
   It holds three things:
   - a copyable one-line prompt, "Read <SKILL.md URL> before you fetch Stellar prices.";
   - `npx skills add rumblefishdev/stellar-prices-api`;
   - links to the skills.stellar.org listing and to the SKILL.md.

   Do not add a seventh card to the Documentation grid: it is 3×2 on `lg`,
   and a seventh card leaves an orphan.

Alternative: a `#ai-agents` section in Quick Start instead of the landing box.

## Dependencies

- 0318: the skills.stellar.org listing, and the hosting decision that fixes the
  SKILL.md URL shown here.
- 0316 edits the same portal files (`index.html`, privacy policy); sequence the
  two deploys.

## Acceptance Criteria

- [ ] Placement chosen (dashboard line, landing box or Quick Start)
- [ ] Copy uses the final SKILL.md URL and the `STELLAR_PRICES_API_KEY` name
- [ ] Portal deployed with `make -C infra sync-portal-explorer`, and the page checked live
