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
  - date: 2026-11-17
    status: active
    who: okarcz
    note: >
      All six open decisions answered by the operator; Implementation Plan
      written. Scope change: the catch-up release (#418) becomes Phase 0 of
      this task, with the three 0242 alarms switched off by a config flag
      instead of waiting for 0242's cleanup. The drift check is dropped.
---

# Production is deployed by hand from develop and master trails it — release through deploy/production, and move master only after a successful deploy

## Summary

Anyone reading the repository should be able to trust one thing: the code on
`master` is the code running in production. Today that does not hold. This task
introduces a three-branch release flow, `develop` → `deploy/production` →
`master`, in which production is deployed only from `deploy/production` and
`master` moves only to a commit whose deploy succeeded.

## Status: Active

**Current state:** decisions made and plan written (2026-11-17), nothing
implemented. Phase 0 starts with the alarm-flag PR.

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

## Decisions (2026-11-17)

Answered by the operator. Numbers follow the former open decisions.

1. **Who runs the deploy:** GitHub Actions, on a push to `deploy/production`,
   behind the `production` environment. Same as `stellar-x402`. A guard in
   `infra/Makefile` keeps a manual deploy possible, but only from
   `deploy/production`'s tip.
2. **How the deployed commit is recorded:** GitHub's environment deployment
   (it holds the commit) plus a release tag on `master`. No SSM parameter and
   no Lambda environment variable.
3. **Drift check:** none. The acceptance criterion is dropped.
4. **Current branches:** first `master` is made equal to production through
   #418 (Phase 0). `master` is then merged back into `develop`, which carries
   over its three commits not on `develop` (`7d52fa9b`, `be01f103`,
   `be941b1b`). `deploy/production` is created from that `master` commit.
5. **Branch protection:**
   - `deploy/production` takes only an approved PR, with no bypass.
   - `master` takes no PRs and no pushes, except the deploy workflow's
     fast-forward. Repository admins may also bypass, for emergencies.
   - The `production` environment approval: any team member, never the person
     who started the run (prevent self-review).
6. **Non-AWS targets:** the ClickHouse server's Ansible belongs to the platform
   repository, not this one. This repository's Hetzner change is the ClickHouse
   schema (`init.sql`, `views.sql`), applied by hand by CH's `default` user.
   It follows `stellar-x402`'s Ansible rule: run only from a detached checkout
   of `origin/deploy/production`, written in the docs. There is no enforcing
   script. Each apply is noted on the release tag.

Also decided: Phase 0 ships first. The workflow and the GitHub settings follow
as separate PRs.

## Implementation Plan

### Phase 0: `master` equal to production (manual)

1. PR to `develop`: a config flag in `infra/envs/production.json` that leaves
   out the three 0242 alarms (`ledger-processor-sac-unproven`,
   `sac-contract-identities`, `ledger-processor-sac-candidates-unavailable`).
   The alarm code stays. Turning the flag on after 0242's cleanup is an
   ordinary release. 0242's owner reviews the PR.
2. Read-only `AWS_PROFILE=soroban-readonly make diff-production` on the new
   `develop` head. Expected: nothing except Lambda code hashes, which differ
   between build machines. Anything else is deployed before step 3.
3. Merge #418 with a merge commit.
4. Merge `master` back into `develop`.
5. Create `deploy/production` from `master`.

### Phase 1: GitHub settings

6. The `production` environment: required reviewers (the team), prevent
   self-review, and deployments allowed from `deploy/production` only.
7. Ruleset on `deploy/production`: PR required with one approval, CI required,
   no bypass, no force push or deletion.
8. Ruleset on `master`: updates blocked, fast-forward only, no deletion.
   Bypass: GitHub Actions and repository admins.

### Phase 2: the deploy workflow

9. `.github/workflows/deploy-production.yml`, triggered by a push to
   `deploy/production` and by a manual start. One run at a time, never
   cancelled.
10. Job `build`: the Lambdas, built the same way as CI's Lambda build.
11. Job `deploy`: in the `production` environment, through the OIDC role from
    `infra/src/lib/stacks/cicd-stack.ts`, runs `cdk deploy --all` from the
    run's commit. Whether `CicdStack` is deployed is not yet checked. If it is
    not, deploying it comes first.
12. Job `promote`, only after `deploy` succeeds: pushes the run's commit to
    `master` (rejected unless it is a fast-forward), then tags it
    `prod-YYYY-MM-DD` (`-2`, `-3` for a second release on the same day). A
    failed deploy leaves `master` and the tags as they were.

### Phase 3: guard and docs

13. `infra/Makefile`: `deploy-production*` refuses unless the working tree is
    clean and `HEAD` equals a freshly fetched `origin/deploy/production`.
14. `infra/README.md`: the flow, a hotfix (PR to `deploy/production`, then merged
    back into `develop`), a rollback (a revert PR on `deploy/production`;
    `master` is never force-moved), the ClickHouse schema rule, and the other
    changes that are not branch state (data repairs, the portal sync), each
    noted on the release tag.

## Acceptance Criteria

- [ ] `deploy/production` exists, is protected, and takes only approved PRs
- [ ] A production deploy from any other commit is refused
- [ ] `master` equals production after Phase 0: #418 merged, `make
      diff-production` shows only Lambda code hashes
- [ ] Every deploy records the commit it was deployed from (GitHub
      deployment), readable without access to the deployer's machine
- [ ] After a successful deploy `master` points at the deployed commit and
      carries a release tag, with no manual step
- [ ] After a failed deploy `master` is unchanged
- ~~A scheduled check reports when the deployed commit and `master` differ~~
  (dropped 2026-11-17, decision 3)
- [ ] The flow, the hotfix path and the rollback path are documented where
      deploys are documented (`infra/README.md`)

## Notes

- Out of scope: the 0242 cleanup (§5–§7) and its backfill. They stay with
  0242. Its alarms come back by turning the Phase 0 flag on.
- The catch-up release (#418) was out of scope until 2026-11-17. It is now
  Phase 0.
- Open point for Phase 2: whether a release that changes only `lore/` or
  docs still runs the deploy. If it skips the deploy, as `stellar-x402` does
  with `paths-ignore`, `master` does not reach `deploy/production`'s tip until
  the next code release. Running it every time is cheap, because a stack with
  no change is a no-op.
- [[0141]] is the history behind `build-lambdas`: a deploy shipped stale Lambda
  assets from a laptop. Building in CI removes that class of error.
