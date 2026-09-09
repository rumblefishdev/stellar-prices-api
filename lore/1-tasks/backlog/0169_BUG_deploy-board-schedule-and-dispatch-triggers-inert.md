---
id: "0169"
title: "deploy-board's schedule + workflow_dispatch triggers are inert — they exist only on develop, but GitHub reads them from the default branch (master)"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0166", "0141"]
tags: ["priority-medium", "effort-small", "ci", "github-actions", "tooling", "silent-failure", "branch-drift"]
links:
  - "../../../.github/workflows/deploy-board.yml"
history:
  - date: 2026-08-10
    status: backlog
    who: okarcz
    note: >
      Spawned from 0166 while verifying its acceptance criteria for archive.
      0166's Step 1 (cancel-in-progress: false) is confirmed working — six
      consecutive successful deploys since the fix, including two merges 102
      seconds apart. But its Step 2, the recovery path, is non-functional:
      `schedule` and `workflow_dispatch` were added to develop's copy of the
      workflow, and GitHub reads both triggers from the DEFAULT branch, which
      is master. Master still carries the pre-fix workflow. Confirmed by zero
      scheduled runs in three days where ~72 were due.
---

# `deploy-board`'s recovery triggers never fire

## Summary

[[0166]] added two triggers to `.github/workflows/deploy-board.yml` so a failed
board deploy would self-heal:

```yaml
  schedule:
    - cron: '17 * * * *'
  workflow_dispatch:
```

Both are **inert**. GitHub resolves `schedule` and `workflow_dispatch` from the
repository's **default branch**, which for this repo is **`master`**. The fix
landed on `develop` (PR #181, `eda7df5`) and `master` still carries the pre-0166
workflow.

The irony is exact: 0166 called out "this repo's DEFAULT branch is master" as
*load-bearing* — but applied it only to the `actions/checkout` ref pin, not to
the triggers that same fact governs.

## Evidence

**The default branch is `master`:**

```bash
$ gh repo view --json defaultBranchRef -q .defaultBranchRef.name
master
```

**`master`'s copy is the pre-0166 workflow** — push-only, and still carrying the
defect 0166 fixed:

```yaml
on:
  push:
    branches:
      - develop
# ...
concurrency:
  group: pages
  cancel-in-progress: true      # <- the original 0166 defect, still here
```

No `schedule`, no `workflow_dispatch`, no `ref: develop` pin, no `npm ci`.

**Zero scheduled runs in three days.** Every run since the fix is `event: push`:

```
2026-08-10T07:42:35Z  push  success  db73107
2026-08-07T13:02:00Z  push  success  696f58a
2026-08-07T12:40:02Z  push  success  f1504fa
2026-08-07T12:25:35Z  push  success  bee237c
2026-08-07T12:23:53Z  push  success  d37e2ff
2026-08-07T10:30:44Z  push  success  eda7df5   <- the 0166 fix
```

At `cron: '17 * * * *'`, roughly **72 scheduled runs were due** between
2026-08-07 10:30 and 2026-08-10 07:42. **None ran.**

## Why `push` still works (and why that hid this)

For a `push` event GitHub uses the workflow file **from the pushed commit**, so
merges to `develop` correctly run develop's fixed copy. That is why Step 1 is
genuinely working and the board is currently live and correct. Only the triggers
that are resolved repo-wide rather than per-ref — `schedule` and
`workflow_dispatch` — fall back to the default branch.

So the primary defect is fixed, the safety net is not, and every observable
signal looks healthy. **Same silent-failure shape as [[0166]] itself and
[[0141]].**

## Impact

0166's stated reason for adding the schedule still stands and is currently
unmet:

> Step 1 alone does **not** make the board reliable: if the newest run fails,
> everything queued behind it was already discarded and nothing retries.

That is exactly what happened with run #263 on 2026-08-06 — a runner-acquisition
failure with nothing behind it, leaving the board stale until a human noticed.
**Today that scenario would still require manual recovery**, and
`workflow_dispatch` — the documented no-empty-commit recovery path — is not
available either, because a workflow must exist on the default branch for the
*Run workflow* button to appear.

Severity is bounded: the push path works, `develop` receives lore merges
regularly, and staleness is visible on the board itself. This is a missing
safety net, not an active outage.

## Implementation Plan

Get the current `deploy-board.yml` onto `master`. Two routes:

1. **PR `develop` → `master`** — correct if a release is due anyway, but drags
   the whole develop delta along and is not this task's call to make.
2. **Narrow PR to `master` carrying only `.github/workflows/deploy-board.yml`**
   — preferred. Single file, no behaviour change to anything that runs from
   `master`, and it simultaneously clears master's stale
   `cancel-in-progress: true`.

⚠️ **Check for the same drift in the other workflows** while there. Any workflow
relying on `schedule` or `workflow_dispatch` has the identical exposure, and
`master`'s copies of `ci.yml` and friends may be behind for the same reason.

## Acceptance Criteria

- [ ] `master`'s `.github/workflows/deploy-board.yml` matches `develop`'s —
      `cancel-in-progress: false`, `schedule`, `workflow_dispatch`,
      `ref: develop`, `npm ci`.
- [ ] A scheduled run appears in `gh run list --workflow=deploy-board.yml` with
      `event: schedule` within ~2 h of the merge. **This is the acceptance
      test** — not the presence of the YAML, which is what fooled 0166.
- [ ] The *Run workflow* button is present on the workflow's Actions page, and a
      manual dispatch publishes `develop`'s board (not `master`'s lore — verify
      the task count, per the `ref: develop` pin).
- [ ] Audit of the remaining workflows for `master`/`develop` drift; anything
      found either fixed here or spawned as its own task.

## Out of scope

- The BE repo's identical `deploy-board` defect — still out of bounds by
  instruction ([[team-adam-kot-task-ownership]]).
- Changing the repository's default branch. Plausible as a separate discussion,
  but it would affect PR targeting, CI, and release flow far beyond this bug.

## Notes

- Reference: GitHub's docs are explicit that scheduled workflows run from the
  default branch's copy, and that `workflow_dispatch` requires the workflow to be
  on the default branch.
- Generalisable lesson worth carrying: **a workflow trigger added on a
  non-default branch is only live for `push`/`pull_request`.** Anything
  repo-scoped (`schedule`, `workflow_dispatch`, `issues`, …) needs the default
  branch.
