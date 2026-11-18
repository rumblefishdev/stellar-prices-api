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
  - date: 2026-11-17
    status: active
    who: okarcz
    note: >
      Phase 0 step 1 opened as PR #421. Read-only diff and AWS checks recorded
      under "Measured on 2026-11-17". Prices-Cicd was never deployed, and its
      OIDC provider would collide with the account's existing one.
  - date: 2026-11-17
    status: active
    who: okarcz
    note: >
      Phase 0 done. #421 merged (7d9e9e70), Secrets deployed (13:05 UTC),
      #418 merged to master (7a85607b, 13:37 UTC), master merged back into
      develop (ebeaa31f, identical tree), deploy/production created at
      7a85607b, unprotected until Phase 1.
  - date: 2026-11-18
    status: active
    who: okarcz
    note: >
      Planned: Phase 1 goes in while 0242's backfill runs. 0242's alarms release is the
      first release through develop → deploy/production → master, with Phase 1
      only: a manual deploy from deploy/production's tip and master moved
      through the admin bypass.
---

# Production is deployed by hand from develop and master trails it — release through deploy/production, and move master only after a successful deploy

## Summary

Anyone reading the repository should be able to trust one thing: the code on
`master` is the code running in production. Today that does not hold. This task
introduces a three-branch release flow, `develop` → `deploy/production` →
`master`, in which production is deployed only from `deploy/production` and
`master` moves only to a commit whose deploy succeeded.

## Status: Active

**Current state:** Phase 0 done on 2026-11-17: `master` is what production
runs, `develop` holds the same tree, and `deploy/production` exists without
protection. Next is Phase 1 (GitHub settings).

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

## Measured on 2026-11-17

Read-only, `soroban-readonly`, `eu-central-1`.

- **`make diff-production` on the #421 branch:**
  - Observability and ApiGateway show no differences.
  - Compute and EventBridge show only Lambda `S3Key` hashes, which differ
    between build machines and prove nothing either way.
  - Secrets shows one difference: the description text of
    `PortalOauthSecretNameParam` (changed in `e8ed5c0e`, 2026-09-02; the stack
    was last deployed 2026-08-28). The value is unchanged. It is closed by
    `make deploy-production-secrets` before #418 merges.
- **`Prices-Cicd` was never deployed.** The account holds only the platform's
  `Explorer-Cicd`.
- **`cicd-stack.ts` would fail to deploy.** It creates
  `new iam.OpenIdConnectProvider` for `token.actions.githubusercontent.com`, but
  the account already has that provider, from `Explorer-Cicd`. An account can
  hold one provider per URL. The stack must import the existing one
  (`OpenIdConnectProvider.fromOpenIdConnectProviderArn`) before step 11.

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

### Phase 0: `master` equal to production (manual) — done 2026-11-17

| Step | Result |
| --- | --- |
| 1 | #421 merged into `develop` as `7d9e9e70`. The production synth before and after differs only in the three alarms (dashboard strip 90 → 87) |
| 2 | Only the Secrets description differed. `make deploy-production-secrets` from `7d9e9e70` at 13:05 UTC changed it; the value is unchanged (SSM version 1 → 2) |
| 3 | #418 merged into `master` as `7a85607b` (13:37 UTC), with a merge commit |
| 4 | `master` merged into `develop` as `ebeaa31f`; `git diff origin/master origin/develop` is empty |
| 5 | `deploy/production` created at `7a85607b`. No workflow runs on it yet |


1. PR to `develop` (#421): a config flag in `infra/envs/production.json` that leaves
   out the three 0242 alarms (`ledger-processor-sac-unproven`,
   `sac-contract-identities`, `ledger-processor-sac-candidates-unavailable`).
   The alarm code stays. Turning the flag on after 0242's cleanup is an
   ordinary release. 0242's owner reviews the PR.
2. Read-only `AWS_PROFILE=soroban-readonly make diff-production` on the new
   `develop` head. Expected: nothing except Lambda code hashes, which differ
   between build machines. Anything else is deployed before step 3. Done
   2026-11-17: only the Secrets description differs (see Measured on
   2026-11-17), so `make deploy-production-secrets` goes before step 3.
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

**First test of the flow: the [[0242]] alarms release (runbook 0242 §6).** It
runs with Phase 1 only (decided 2026-11-18): the flag PR to `develop`, a
release PR to `deploy/production`, `make deploy-production-observability` by
hand from its tip, then `master` fast-forwarded and tagged through the admin
bypass. It tests the branch rules, not the `production` environment approval
or the `promote` job; those get their first test on the first release after
Phase 2.

### Phase 2: the deploy workflow

9. `.github/workflows/deploy-production.yml`, triggered by a push to
   `deploy/production` and by a manual start. One run at a time, never
   cancelled.
10. Job `build`: the Lambdas, built the same way as CI's Lambda build.
11. Job `deploy`: in the `production` environment, through the OIDC role from
    `infra/src/lib/stacks/cicd-stack.ts`, runs `cdk deploy --all` from the
    run's commit. `Prices-Cicd` is not deployed (checked 2026-11-17). First
    change it to import the account's existing GitHub OIDC provider, then
    deploy it with `make deploy-cicd`.
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
- [x] `master` equals production after Phase 0: #418 merged, `make
      diff-production` shows only Lambda code hashes (2026-11-17)
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
