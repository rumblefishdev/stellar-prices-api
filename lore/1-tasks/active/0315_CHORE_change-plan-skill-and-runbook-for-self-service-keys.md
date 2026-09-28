---
id: "0315"
title: "\"Raise user X's plan\" has no entry point — a change-plan skill, and the tier runbook split into check / change / record for self-service keys"
type: CHORE
status: active
assignee: akot
related_adr: []
related_tasks: ["0311", "0191", "0180"]
tags: [layer-infra, priority-medium, effort-small, usage-plans, runbook, docs, tooling]
links:
  - "../../../docs/runbooks/manual-api-key-tier.md"
history:
  - date: "2026-09-28"
    status: active
    who: akot
    note: >
      Created and activated. 0311 left the operator procedure in
      docs/runbooks/manual-api-key-tier.md, but nothing points a Claude
      session at it, so "podnieś plan userowi <discord id>" starts from
      scratch. Scope decided by Adam: runbook update + project skill +
      one CLAUDE.md line; self-service plan moves get their own registry
      table.
---

# "Raise user X's plan" has no entry point — a change-plan skill and runbook update

## Summary

Since [[0311]] an operator moves a user's key between the five usage plans
(free, basic, analyst, lite, pro) by hand in AWS, following
`docs/runbooks/manual-api-key-tier.md` § "Upgrade a user". The procedure is
complete, but it is found only by someone who knows it exists. The goal: a
request such as "podnieś plan userowi 1234… na pro" lands a Claude session
on the right commands, with a stop before anything mutates production.

## Context

- The runbook resolves `discord-<id>-key` by exact name, finds `FROM`/`TO`
  with an `ok_id` guard, moves the key (`delete-usage-plan-key` →
  `create-usage-plan-key`, seconds of `403` between), and verifies it.
- It has no read-only "which plan is user X on" entry; that check is buried
  inside the upgrade steps.
- Step 6 records the move in "Issued manual keys", a table shaped for
  hand-made keys (plan id, key name with a timestamp, limits), which a
  `discord-` key only half fits.
- Neither `CLAUDE.md` nor any project skill mentions the runbook.

## Implementation Plan

1. **Runbook** (`docs/runbooks/manual-api-key-tier.md`):
   - a "Check a user's plan" section: Discord id → key(s) → plan name, read-only;
   - step 6 points at a new table, "Plan changes of self-service keys"
     (Discord id, key id, from → to, date, who); "Issued manual keys" stays
     for hand-made keys only.
2. **Skill** `.claude/skills/change-plan/SKILL.md`, triggered by requests to
   raise, lower, change or check a user's plan. It carries the rules, not a
   copy of the commands, which stay in the runbook:
   - inputs: Discord id + target tier; ask if either is missing or the tier
     is not one of the five;
   - AWS profile `stellar`, `eu-central-1`; ask if the profile is missing;
   - run the read-only steps, show key(s), current and target plan;
   - **stop and ask** before the delete + create;
   - move, verify, report what the user sees;
   - edge cases (duplicates, revoked-only keys, no key, Custom plans)
     routed to the runbook's own paragraphs;
   - registry row proposed as a diff; commit only with approval.
3. **`CLAUDE.md`**: one line in an "Operations" section pointing at the
   skill and the runbook.

## Acceptance Criteria

- [ ] The runbook has a read-only "Check a user's plan" section.
- [ ] Self-service plan moves are recorded in their own table; step 6 and
      the downgrade path point at it.
- [ ] `.claude/skills/change-plan/SKILL.md` exists, is committed, and
      stops for confirmation before any production mutation.
- [ ] `CLAUDE.md` points at the skill and the runbook.
