---
id: "0352"
title: "deploy-production.yml's header says the release diff comes from the PR's head — it comes from the merge ref"
type: CHORE
status: active
assignee: okarcz
related_adr: []
related_tasks: ["0343"]
tags: [layer-infra, ci, docs, priority-low, effort-small]
links:
  - "../../../.github/workflows/deploy-production.yml"
  - "../../../infra/README.md"
history:
  - date: 2026-11-23
    status: backlog
    who: okarcz
    note: >
      Spawned from 0343 future work.
  - date: 2026-11-23
    status: active
    who: okarcz
    note: "Activated; one-line comment fix, PR to develop."
---

# deploy-production.yml's header says the release diff comes from the PR's head — it comes from the merge ref

## Summary

The comment block at the top of `.github/workflows/deploy-production.yml`
tells whoever opens a release PR to run `make -C infra diff-production` "from
its head". [[0343]] decision 20 changed that to the PR's merge ref
(`refs/pull/<n>/merge`), and `infra/README.md` "A release" already says so. A
reader who follows the workflow comment diffs `develop`, where a hotfix not yet
merged back shows as reverted.

## Implementation

- Change "from its head" to "from its merge ref (`refs/pull/<n>/merge`)" in
  the header, pointing to `infra/README.md` for the commands.
- Comment only: the workflow's behaviour does not change. It ships with the
  next release like any other change.

## Acceptance Criteria

- [ ] The workflow header and `infra/README.md` name the same ref for the
      release diff
