---
id: "0315"
title: "\"Raise user X's plan\" has no entry point — a change-plan skill, and the tier runbook split into check / change / record for self-service keys"
type: CHORE
status: completed
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
  - date: "2026-09-28"
    status: completed
    who: akot
    note: >
      Closed. PR #361 merged to develop as 3d5d0cc3 (59cbb68f skill +
      runbook + CLAUDE.md; 288abb0f permissions.ask rules). The AWS profile
      became a skill parameter, and mutating aws apigateway calls now
      prompt even in auto mode (tested). The read-only check was run live
      on production. The skill's own stop before a move is not yet
      exercised in a fresh session. 4/4 criteria met; no follow-up tasks.
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

- [x] The runbook has a read-only "Check a user's plan" section. Run on
      production 2026-09-28: a free key, the loadtest key (Custom) and
      Adam's key (Lite, `jfgkw6r9na`) all reported correctly.
- [x] Self-service plan moves are recorded in their own table ("Plan
      changes of self-service keys", append-only); step 6 and the downgrade
      path point at it.
- [x] `.claude/skills/change-plan/SKILL.md` exists, is committed, and stops
      for confirmation before any production mutation. There are two gates:
      the skill's step 4, and a Claude Code permission prompt from
      `permissions.ask`. The second was tested in auto mode. The first is
      written but not yet exercised in a fresh session.
- [x] `CLAUDE.md` points at the skill and the runbook (section "Operations").

## Implementation Notes

- `docs/runbooks/manual-api-key-tier.md`:
  - "Check a user's plan (read-only)": exact-name key lookup, then each
    record's plan filtered to our API id + `production`, with rules for
    reading the result;
  - the new registry table;
  - a header note naming the skill and the prompt rules.
- `.claude/skills/change-plan/SKILL.md`:
  - `argument-hint: <discord-id> [tier] [profile=<aws-profile>]`;
  - resolves the profile (argument → `$AWS_PROFILE` → ask), then
    `sts get-caller-identity` must return `750702271865`;
  - eight steps that point at the runbook's commands;
  - rules: no CDK-plan edits, no key values, no `items[0]`, profile only
    via `export`.
- `.claude/settings.json`: `permissions.ask` on
  `aws apigateway {create,delete}-usage-plan-key`,
  `{create,update,delete}-usage-plan` and `{create,update,delete}-api-key`.
- `CLAUDE.md`: an "Operations" section with one line.

## Design Decisions

### From Plan

1. **The commands live in the runbook only.** The skill carries the rules
   and step order, so the two cannot drift apart.
2. **A separate append-only table for self-service moves.** "Issued manual
   keys" stays for hand-made keys. A key's current plan is what AWS says,
   not the last row of the table.

### Emerged

3. **The AWS profile is a skill parameter** (Adam). Profile names differ per
   machine. The skill never picks one on its own, and it checks the account
   before any other call.
4. **`permissions.ask` rules instead of a mode switch.** A skill cannot
   change the permission mode, and a PreToolUse hook returning `ask` is
   ignored in auto mode (Claude Code docs). Ask rules are evaluated before
   the auto-mode classifier and match inside `if`/`&&`/`$( )`. They apply
   repo-wide on purpose: these calls mutate production whoever makes them.
5. **Profile only via `export`.** The ask rules match the literal prefix
   `aws apigateway <verb>`, so `--profile` in front of the verb or an inline
   `AWS_PROFILE=… aws` would slip past them.

## Issues Encountered

- **The first prompt test was inconclusive from the agent's side.** The call
  ran and returned `NotFoundException` (the ids were `none`), and the agent
  cannot see the prompt dialog. Adam confirmed he got the prompt.
- **`--dangerously-skip-permissions` disables the ask rules.** In that mode
  only the skill's step 4 remains. Accepted; stated in the skill.
