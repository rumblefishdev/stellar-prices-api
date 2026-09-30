---
id: "0322"
title: "Hide the repo's internal skills from `npx skills add` — mark them `internal`, so the public install offers only the prices-API skill"
type: CHORE
status: active
related_adr: []
related_tasks: ["0318", "0315"]
tags: [docs, agents, marketing, priority-low, effort-small]
links:
  - "../active/0318_FEATURE_agent-skill-for-the-prices-api-on-skills-stellar-org.md"
  - "../../../.claude/skills"
  - "../../../.agents/skills"
history:
  - date: "2026-09-30"
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from 0318. Once the public skill landed on master (#372), the
      skills CLI started offering our internal tooling to anyone who installs
      from the repo.
  - date: "2026-09-30"
    status: active
    who: stkrolikiewicz
    note: "Activated at the user's request, right after the Stellar PR #141 went out."
---

# Hide the repo's internal skills from `npx skills add`

## Summary

`npx skills add rumblefishdev/stellar-prices-api` lists every `SKILL.md` it
finds in the repo's default branch, not only the public one. Mark the internal
ones `internal`, so the community sees only `stellar-prices-api`.

## Context

Measured 2026-09-30 on `master` after #372. `--list` found 11 skills:

- `stellar-prices-api`, the public skill (0318);
- 7 Nx dev skills in `.agents/skills/`: `link-workspace-packages`,
  `monitor-ci`, `nx-generate`, `nx-import`, `nx-plugins`, `nx-run-tasks`,
  `nx-workspace`;
- 3 lore skills in `.claude/skills/`: `branch`, `pr`, `promote-task`.

`.claude/skills/change-plan` (0315, the prod usage-plan runbook) is only on
`develop`. It joins the list at the next develop→master merge. Nothing in it
is secret, since the repo is public, but it is operator tooling, not
something to offer the Stellar community.

The skills card on skills.stellar.org links the raw `SKILL.md` directly, so
this does not affect that listing. Only the `npx skills add owner/repo` path is
affected.

**The mechanism, verified 2026-09-30** on a throwaway repo:

- The CLI hides a skill whose frontmatter has `metadata:` / `internal: true`.
- `--list` on a repo with one public and one internal skill printed
  "Found 1 skill". With `INSTALL_INTERNAL_SKILLS=1` it printed "Found 2".
- The Agent Skills validator (`uvx --from skills-ref agentskills validate`)
  accepts both `internal: true` and `internal: "true"`.

## Implementation Plan

1. Add `metadata: {internal: true}` to the frontmatter of the 4
   `.claude/skills/*` and the 7 `.agents/skills/*`.
2. Check whether Nx regenerates `.agents/skills/` (`nx configure-ai-agents`).
   If it does, the marker will be lost on the next regeneration. Record how to
   re-apply it, or exclude that directory another way.
3. Land it on `develop` by PR, then on `master` by a targeted PR like #372. The
   CLI reads the default branch, so `develop` alone changes nothing publicly.
4. Confirm Claude Code still loads the four `.claude/skills` after the change.

## Acceptance Criteria

- [ ] `npx skills add rumblefishdev/stellar-prices-api --list` shows only `stellar-prices-api`
- [ ] `INSTALL_INTERNAL_SKILLS=1` still lists the internal ones (nothing deleted)
- [ ] `/branch`, `/pr`, `/promote-task`, `change-plan` still work in Claude Code
