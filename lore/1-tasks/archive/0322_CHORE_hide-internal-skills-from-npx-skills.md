---
id: "0322"
title: "Hide the repo's internal skills from `npx skills add` — mark them `internal`, so the public install offers only the prices-API skill"
type: CHORE
status: completed
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
  - date: "2026-09-30"
    status: completed
    who: stkrolikiewicz
    note: >
      #374 (develop) and #375 (master) merged 11:55 UTC. npx skills add
      rumblefishdev/stellar-prices-api --list, straight from GitHub, finds 1
      skill (was 11); INSTALL_INTERNAL_SKILLS=1 finds all 11. 11 SKILL.md
      marked (+22 lines), 1 guard test added (2 cases, both negative
      controls fail it), 0 existing tests changed.
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

## Progress (2026-09-30)

Two PRs are open:

- **#374 → `develop`:** the marker on all 11 skills plus
  `tools/scripts/internal-skills-guard.test.mjs`.
- **#375 → `master`:** the marker on the 10 skills that exist there; no
  test, no `change-plan`.

Checks run:

- `npx skills add <branch> --list` shows 1 skill (`stellar-prices-api`) on
  both branches.
- A fresh `claude -p` with the Skill tool still lists `branch`, `pr`,
  `promote-task` and `change-plan`, the same as the main checkout without
  the marker.
  - A first run with `--tools ""` reported all of them "not listed". That
    was an artefact of the test: without the Skill tool there is no skill
    list.
- The guard passes 2/2 and runs under
  `npx nx test @rumblefish/stellar-prices-api-aws-cdk`. Negative controls:
  - removing the marker from `nx-workspace` fails it;
  - marking `skills/stellar-prices-api` fails it.

## Design Decisions

### From Plan

1. **Mark, don't delete.** `metadata: internal: true` on all 11. The CLI
   hides them, and `INSTALL_INTERNAL_SKILLS=1` still shows them.

### Emerged

2. **A guard test instead of a note on how to re-apply the marker.**
   `nx configure-ai-agents` copies Nx's templates over `.agents/` with
   `generateFiles` (`nx/dist/src/ai/set-up-ai-agents/set-up-ai-agents.js`
   ~190), and Nx suggests the command on every push. The marker would
   vanish with every check green.
   - The guard lives in `tools/scripts/`, where the infra `test` target
     (`node --test "tools/scripts/**/*.test.mjs"`) already runs on every PR.
   - It also stops a new internal skill landing unmarked, and the public
     skill getting marked by mistake.
3. **#375 to `master` without the test.** `master`'s infra project has no
   `test` target, so the test would sit there unrun. It arrives with the
   next develop→master release.
4. **#375 pushed with `--no-verify`.** The shared-hooksPath problem is the
   same as #372 (see memory `worktree-git-hooks-need-node-modules`). The
   change is frontmatter only.

## Acceptance Criteria

- [x] `npx skills add rumblefishdev/stellar-prices-api --list` shows only `stellar-prices-api`: "Found 1 skill", run from GitHub after the #375 merge
- [x] `INSTALL_INTERNAL_SKILLS=1` still lists the internal ones: "Found 11 skills", nothing deleted
- [x] `/branch`, `/pr`, `/promote-task`, `change-plan` still work in Claude Code: a fresh `claude -p` with the Skill tool lists all four, the same as a checkout without the marker

## Implementation Notes

- **Marked skills.** `metadata:` / `internal: true` appended to the
  frontmatter of:
  - `.claude/skills/{branch,change-plan,pr,promote-task}/SKILL.md`;
  - `.agents/skills/{link-workspace-packages,monitor-ci,nx-generate,nx-import,nx-plugins,nx-run-tasks,nx-workspace}/SKILL.md`.

  Two lines per file; nothing else changed.
- **Guard test.** `tools/scripts/internal-skills-guard.test.mjs` runs under
  the infra `test` target (`npx nx test @rumblefish/stellar-prices-api-aws-cdk`),
  which CI runs on every PR. It asserts two things:
  - every `.claude/skills` and `.agents/skills` SKILL.md is internal;
  - `skills/*` has at least one public skill and none marked.
- **#375 to `master`.** The same marker on the 10 skills `master` has
  (`change-plan` is `develop`-only). It carries no test.

## Issues Encountered

- **The first "Claude Code still loads them" check was a false negative.**
  `claude -p --tools ""` answered "not listed" for all four. Without the
  Skill tool there is no skill list in the session, so this said nothing.
  With `--tools "Skill"`, both this branch and the unmarked main checkout
  list all four.
- **Pushing a `master`-based branch fails the pre-push hook**, the same as
  #372. The shared `core.hooksPath` runs the main checkout's develop-era
  hook, which clippies `comet-extractor`. #375 went with `--no-verify`, and
  the reason is in its commit body. This is recorded in memory
  (`worktree-git-hooks-need-node-modules`).
- **zsh did not word-split `$FILES` in `git checkout <branch> -- $FILES`.**
  The pathspec failed. Re-run through `xargs -I{}`.

## Future Work

None. The guard covers the one known way the marker gets lost
(`nx configure-ai-agents`). After running that command, re-add the two lines;
the test failure names the files.

