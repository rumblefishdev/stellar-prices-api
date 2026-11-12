---
id: "0343"
title: "Production is deployed by hand from develop and master trails it — release through deploy/production, and move master only after a successful deploy"
type: FEATURE
status: active
assignee: okarcz
related_adr: []
related_tasks: ["0141", "0242"]
tags: ["deploy", "release", "ci", "branching", "priority-medium", "effort-medium"]
links:
  - "../../../infra/Makefile"
  - "../../../infra/src/lib/stacks/cicd-stack.ts"
  - "../../../.github/workflows/ci.yml"
  - https://github.com/rumblefishdev/stellar-x402/blob/develop/.github/workflows/deploy-production.yml
history:
  - date: 2026-11-12
    status: active
    who: okarcz
    note: >
      Task created and activated. Scope so far is the branch flow below; no
      code yet. The mechanism (who runs the deploy, how the commit is stamped)
      is still open and listed under Open decisions.
---

# Production is deployed by hand from develop and master trails it — release through deploy/production, and move master only after a successful deploy

## Summary

Anyone reading the repository should be able to trust one thing: the code on
`master` is the code running in production. Today that does not hold. This task
introduces a three-branch release flow, `develop` → `deploy/production` →
`master`, in which production is deployed only from `deploy/production` and
`master` moves only to a commit whose deploy succeeded.

## Status: Active

**Current state:** flow described, nothing implemented. Waiting for the
operator's decisions on the open points before any code.

## Context

Measured on 2026-11-12:

- Production is deployed from an operator's machine with
  `make deploy-production-*`, from `develop`, one stack at a time.
- `master` is updated later by a release PR. It was 164 commits behind
  `develop` (release PR #418, open).
- `master` has no branch protection. The repository has no release tags.
- A deployed Lambda carries no commit id. Which commit the 2026-11-06 Compute
  deploy was built from could only be answered from the deployer's reflog.
- One stack was held back on purpose (0242 D8: Observability, three alarms), so
  "what `develop` describes" and "what is deployed" differed by design, with
  the difference recorded only in a task file.
- `infra/src/lib/stacks/cicd-stack.ts` defines a GitHub OIDC deploy role scoped
  to a `production` environment. No workflow uses it. Whether that stack is
  deployed was not checked.

`stellar-x402` already releases through a protected `deploy/production` branch:
a PR from `develop`, a deploy started by the merge, a build stamped with its
commit. This task brings the same order here and adds `master` as the record of
what is live.

## The flow

| Branch | Meaning | Who moves it |
| --- | --- | --- |
| `develop` | work in progress | feature PRs |
| `deploy/production` | approved for release | release PR from `develop`, approved, merge commit |
| `master` | confirmed live | the deploy, after it succeeds; fast-forward only |

1. Feature PRs merge to `develop`, as now. Task documents keep going straight
   to `develop`.
2. A release is a PR from `develop` to `deploy/production`. CI runs on it, and
   the production `cdk diff` is read on it before approval.
3. The merge starts (or permits) the deploy of **all** stacks from that exact
   commit. A stack with no change is a no-op.
4. On success, `master` is fast-forwarded to the deployed commit and the commit
   is tagged (for example `prod-2026-11-12`).
5. On failure, `master` does not move. The fix or a revert goes to
   `deploy/production` by PR and is deployed like any release.

Rules that keep it true:

- `master` takes no PRs and no direct commits; only the deploy moves it.
- Hotfixes go to `deploy/production` by PR and are merged back into `develop`.
- A rollback is a revert PR on `deploy/production`. `master` is never
  force-moved.
- No stack is "held back". A resource that must wait is switched off by a
  config flag, so every stack can always be deployed from the release commit.
- Changes that are not branch state (ClickHouse schema applies, Ansible runs,
  the portal sync, data repairs) are applied from the release commit and noted
  on the release tag.

## Open decisions

1. **Who runs the deploy.** GitHub Actions on `deploy/production` behind a
   `production` environment approval, or the operator's machine with a guard in
   `infra/Makefile` that refuses any other commit. Possibly the guard first and
   the workflow second.
2. **How the deployed commit is stamped.** Per stack, since a five-stack deploy
   can fail part-way. Candidate: an SSM parameter written by each stack. A
   Lambda environment variable would restart every Lambda on every release.
3. **Drift check.** A scheduled job that compares the stamps with `master` and
   reports a mismatch.
4. **What happens to the current branches.** How `deploy/production` is created
   (from `master` after #418, or from `develop`), and how the one commit that
   exists only on `master` (`7d52fa9b`) is carried over.
5. **Branch protection.** Rules for `deploy/production` and `master`, and which
   identity is allowed to fast-forward `master`.
6. **Non-AWS targets.** How far the Hetzner side (ClickHouse, Ansible) is
   brought under the same rule in this task.

## Implementation Plan

To be written once the open decisions are answered.

## Acceptance Criteria

- [ ] `deploy/production` exists, is protected, and takes only approved PRs
- [ ] A production deploy from any other commit is refused
- [ ] Every stack records the commit it was deployed from, readable without
      access to the deployer's machine
- [ ] After a successful deploy `master` points at the deployed commit and
      carries a release tag, with no manual step
- [ ] After a failed deploy `master` is unchanged
- [ ] A scheduled check reports when the deployed commit and `master` differ
- [ ] The flow, the hotfix path and the rollback path are documented where
      deploys are documented (`infra/README.md`)

## Notes

- Out of scope: the catch-up release itself (PR #418) and the 0242 cleanup.
  Both are handled separately and do not wait for this task.
- [[0141]] is the history behind `build-lambdas`: a deploy shipped stale Lambda
  assets from a laptop. Building in CI removes that class of error.
