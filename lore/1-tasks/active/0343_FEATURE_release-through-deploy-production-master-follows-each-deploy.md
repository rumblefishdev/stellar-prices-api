---
id: "0343"
title: "Production is deployed by hand from develop and master trails it — release through deploy/production, and move master only after a successful deploy"
type: FEATURE
status: active
assignee: okarcz
related_adr: []
related_tasks: ["0141", "0242", "0351"]
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
  - date: 2026-11-19
    status: active
    who: okarcz
    note: >
      Phase 1 done. Environment production (4 reviewers, prevent self-review,
      deploy/production only) and three rulesets: 27627834 deploy/production,
      27628401 master (update, admin bypass), 27628422 master (history)
      (deletion + non_fast_forward, no bypass). GitHub refused GitHub Actions
      as a bypass actor, so promote's push to master is an open point for
      Phase 2.
  - date: 2026-11-19
    status: active
    who: okarcz
    note: >
      First release through the flow: #426 develop → deploy/production
      (c7dcd26d), Observability deployed from its tip, master fast-forwarded
      through the admin bypass, tag prod-2026-11-19. It carried 0348, whose
      Compute had been deployed by hand from develop the same morning.
  - date: 2026-11-20
    status: active
    who: okarcz
    note: >
      Phase 2 decisions answered by the operator: promote pushes master with
      a deploy key held as a production environment secret; every release
      deploys, docs-only included; the release PR's cdk diff stays manual;
      the deploy role may flush the production stage cache; CI keeps running
      on master; the portal sync stays manual. The team agreed to import
      Explorer-Cicd's OIDC provider as is; moving it to
      soroban-infrastructure is spawned as 0351.
  - date: 2026-11-20
    status: active
    who: okarcz
    note: >
      Phase 2 opened as #427 (deploy workflow, OIDC import, cache-flush
      permission, then a GitHub Release job). Phase 3 opened as #428 (release
      guard on 13 make targets, no override; release-flow docs). Setup
      runbook for the operator under Phase 2 setup.
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

**Done 2026-11-19.** The operator ran each call with `gh api` from the local
machine; every setting was read back from GitHub afterwards.

| Setting | Id | State |
| --- | --- | --- |
| Environment `production` | | Required reviewers karczuRF, adamkoot, stkrolikiewicz, kowalski; `prevent_self_review: true`; custom branch policies on |
| Deployment branch policy | 67398189 | `deploy/production` (type `branch`), the only entry |
| Ruleset `deploy/production` | 27627834 | `deletion`, `non_fast_forward`, `pull_request` (1 approval, dismiss stale, `merge` only), `required_status_checks` (`Detect changes`, `TypeScript (lint, build, typecheck, test)`, `Rust (fmt, clippy, test, lambda build)`, app 15368); no bypass |
| Ruleset `master` | 27628401 | `update`; bypass repository role admin (5), `always` |
| Ruleset `master (history)` | 27628422 | `deletion`, `non_fast_forward`; no bypass |

`develop` has no rules. `master` and `deploy/production` were both at
`7a85607b`.

- **`master` takes two rulesets.** A bypass actor skips every rule of the
  ruleset that lists it, so an admin bypass on a single ruleset would also
  allow a force push or a deletion. With `deletion` and `non_fast_forward` in a
  ruleset of their own, nobody can bypass them, and every move of `master` is
  a fast-forward.
- **No GitHub Actions bypass.** GitHub refused the built-in Actions app (15368)
  as a bypass actor: "Actor GitHub Actions integration must be part of the
  ruleset source or owner organization". It is accepted in required status
  checks, not as a bypass actor. Until Phase 2, an admin moves `master` by
  hand.
- **Admin bypass covers every repository admin.** A repository ruleset cannot
  name individual users. On 2026-11-19 the repository had seven admins:
  karczuRF, adamkoot, stkrolikiewicz, Efem67, kowalski, mmisiorek and
  Roger-RumbleFish. "Emergencies only" is a team rule; GitHub logs each bypass
  under the ruleset's Rule insights.
- GitHub added `require_extra_approval_for_unattributed_changes: true` to the
  PR rule by default. Watch it on the first release PR.

Rollback: `gh api -X DELETE repos/rumblefishdev/stellar-prices-api/rulesets/<id>`
for each ruleset, and `…/environments/production` for the environment.

**First test of the flow: the [[0242]] alarms release (runbook 0242 §6).** It
runs with Phase 1 only (decided 2026-11-18): the flag PR to `develop`, a
release PR to `deploy/production`, `make deploy-production-observability` by
hand from its tip, then `master` fast-forwarded and tagged through the admin
bypass. It tests the branch rules, not the `production` environment approval
or the `promote` job; those get their first test on the first release after
Phase 2.

**First release, 2026-11-19: `prod-2026-11-19` on `c7dcd26d`.**

| Step | Result |
| --- | --- |
| Flag PR #425 → `develop` | Approved by adamkoot, merged as `97b4cd62` |
| Release diff | Also carried [[0348]] (#422, #423), merged to `develop` at 09:39 UTC. Its owner had deployed Compute by hand from `develop` `21b938fa` at 09:50 UTC (confirmed); `cdk diff Prices-production-Compute` showed only Lambda code hashes. Released with it, Compute not redeployed |
| Release PR #426 `develop` → `deploy/production` | `BLOCKED` / `REVIEW_REQUIRED` until approved (adamkoot) and the three checks passed; merged as `c7dcd26d` (parents `7a85607b`, `97b4cd62`). No extra approval was asked for unattributed changes |
| Deploy | `make deploy-production-observability` from a clean detached checkout of `c7dcd26d`, 12:55:28 UTC. Details in [[0242]]'s heal log |
| `master` | `git push origin c7dcd26d…:refs/heads/master`: "Bypassed rule violations … Cannot update this protected ref", `7a85607b..c7dcd26d` |
| Tag | Annotated `prod-2026-11-19` on `c7dcd26d` |

After it, `origin/master` = `origin/deploy/production` = `prod-2026-11-19` =
`c7dcd26d`, and every stack runs that commit's code.

- **Compute went out by hand from `develop` twice (2026-11-18 and 11-19),
  for [[0348]],** before the team had been told about this flow. Nothing
  stops it until Phase 3's Makefile guard. Releasing `develop` brought the
  branches back to what is live; a cherry-picked release would have left
  `master` claiming a Compute that was not running.
- `prices-production-coverage-sweep-unclassified` was already in ALARM before
  this release (since 2026-11-09, [[0345]] part C); the release did not cause
  that red tile.

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
    The `GITHUB_TOKEN` cannot pass ruleset 27628401, because GitHub refuses
    the Actions app as a bypass actor (found 2026-11-19). `promote` pushes with
    a deploy key instead (decision 7 below).

#### Decisions (2026-11-20)

Answered by the operator. Numbering continues from the 2026-11-17 decisions.

7. **How `promote` passes ruleset 27628401: a deploy key.** One SSH key pair
   for this repository. The public half is a write deploy key; the private
   half lives only as a secret of the `production` environment, so only a
   run a teammate approved can read it, and no copy stays on disk (it goes to
   `gh secret set` on stdin). 27628401 gets a `DeployKey` bypass next to the
   admin one; 27628422 keeps no bypass, so the key can only fast-forward
   `master`. The admin bypass stays until step 15. A GitHub App was rejected.
   - The bypass covers every write deploy key of the repository, not one key.
     On 2026-11-20 the repository had none.
   - Rotation: add the new key, overwrite the secret, delete the old key; the
     ruleset does not change. On a leak, delete the key first; `promote` then
     fails and an admin moves `master` by hand until a new key is in.
8. **Every release deploys, docs-only included.** No `paths-ignore`, so
   `master` always reaches `deploy/production`'s tip ("`master` = the last
   release"). Stacks with no change are a no-op.
9. **The release PR's `cdk diff` is manual.** Whoever opens the release PR
   runs `make diff-production` from its head and pastes the result into the
   PR; the approver reads it for removals. `infra/README.md` says so. The
   workflow deploys with `--require-approval never`, because CI cannot answer
   a prompt; the PR approval and the `production` environment approval are
   the human checks.
10. **The deploy role may flush the production stage cache.** One statement:
    `apigateway:DELETE` on
    `arn:aws:apigateway:<region>::/restapis/<id>/stages/production/cache/data`.
    It drops only cached GET responses (`/api-docs-json` 1 h; six `/v1` routes
    10–60 s), never data, keys, the stage or the API, and not the authorizer
    cache. Same flush `make deploy-production` runs today.
11. **CI keeps running on `master`.** A push with a deploy key starts
    workflows, so `ci.yml` runs once more on each released commit. Kept.
12. **The portal sync stays manual.** `make sync-portal-explorer` from the
    release commit when `web/portal/` changed, noted on the release tag. The
    deploy role gets no access to the explorer's bucket or distribution.

13. **`Prices-Cicd` imports the GitHub OIDC provider as is** (agreed with the
    team on 2026-11-20). `fromOpenIdConnectProviderArn` with
    `arn:aws:iam::750702271865:oidc-provider/token.actions.githubusercontent.com`.
    `Explorer-Cicd` does not change. Deleting that stack, or removing the
    provider from its code, would delete the provider and stop both
    pipelines; the risk is accepted because the team owns both repositories
    and the explorer's own deploy depends on the same provider. Moving the
    provider to `soroban-infrastructure` is [[0351]].

#### Measured on 2026-11-20

Read-only, `soroban-readonly`.

- The account has one GitHub OIDC provider, the ARN above, created
  2026-08-12. Audience `sts.amazonaws.com` and thumbprint `6938fd4d…`, the
  same values `cicd-stack.ts` uses, so the import needs no change on the
  explorer's side.
- It is `GitHubOidcProvider7EBF861F` in `Explorer-Cicd`, a CDK custom
  resource. The stack also holds `soroban-explorer-production-deploy`, which
  the explorer's tag-driven `deploy-production.yml` uses (last tags
  `production-2026.11.12-1`, `-2-CloudWatch`).
- The repository had no deploy keys.

#### Phase 2 implementation (PR #427, opened 2026-11-20)

- `.github/workflows/deploy-production.yml`: job `build` (ARM runner, the CI
  toolchain pair, `build-lambda-assets.sh`, assets passed on as a tar because
  `upload-artifact` drops the executable bit) and job `deploy` (environment
  `production`: verify the bootstraps, `cdk deploy --all --require-approval
  never`, `make flush-production-cache`, then the promote steps).
- `cicd-stack.ts`: the provider imported by ARN (decision 13), plus
  `apigateway:DELETE` on `…/restapis/<id>/stages/production/cache/data`, the
  id taken from `/prices/production/api-gateway-id` (`02mabge71l` on
  2026-11-20).
- **Promote is steps in `deploy`, not a job (emerged).** The deploy key is a
  `production` environment secret, and every job naming an environment with
  required reviewers asks for its own approval, so a separate `promote` job
  would need a second one. The steps run only when the deploy and the flush
  succeeded, which is the same guarantee step 12 asked for.
- The workflow runs on the first push to `deploy/production` that carries the
  file, so the setup below must be finished before that release.

#### Phase 2 setup (the operator, before the next release)

All from the local machine, repository root. Steps 1–2 can run any time;
steps 3–4 run after #427 merges.

1. **Deploy key.** Generated in `/dev/shm` (RAM, never written to disk), the
   private half goes to the environment secret on stdin, then both halves are
   shredded:
   ```bash
   d=$(mktemp -d /dev/shm/promote.XXXXXX)
   ssh-keygen -q -t ed25519 -N '' -C 'stellar-prices-api promote (deploy-production.yml)' -f "$d/key"
   gh repo deploy-key add "$d/key.pub" --allow-write --title 'promote (deploy-production.yml)' -R rumblefishdev/stellar-prices-api
   gh secret set PROMOTE_DEPLOY_KEY --env production -R rumblefishdev/stellar-prices-api < "$d/key"
   shred -u "$d/key" "$d/key.pub" && rmdir "$d"
   ```
   Check: `gh repo deploy-key list -R rumblefishdev/stellar-prices-api` shows
   one key with read-write; `gh secret list --env production -R rumblefishdev/stellar-prices-api`
   shows `PROMOTE_DEPLOY_KEY`.
2. **Bypass on ruleset 27628401.** The full list is sent, so the admin entry
   stays:
   ```bash
   gh api -X PUT repos/rumblefishdev/stellar-prices-api/rulesets/27628401 --input - <<'JSON'
   {"bypass_actors":[
     {"actor_id":5,"actor_type":"RepositoryRole","bypass_mode":"always"},
     {"actor_id":null,"actor_type":"DeployKey","bypass_mode":"always"}
   ]}
   JSON
   gh api repos/rumblefishdev/stellar-prices-api/rulesets/27628401 --jq '{rules:[.rules[].type], bypass_actors}'
   ```
   Expected: rules `["update"]`, two bypass actors. Ruleset 27628422 is not
   touched.
3. **`make deploy-cicd`** (after #427 merges, from `develop`), with
   `soroban-admin`. It asks once about the new IAM role; the diff must show
   one role, one policy and no OIDC provider:
   ```bash
   git checkout develop && git pull --ff-only
   AWS_PROFILE=soroban-admin make -C infra diff-cicd
   AWS_PROFILE=soroban-admin make -C infra deploy-cicd
   ```
4. **Role ARN into the environment:**
   ```bash
   gh secret set AWS_DEPLOY_ROLE_ARN --env production -R rumblefishdev/stellar-prices-api --body "$(AWS_PROFILE=soroban-readonly aws cloudformation describe-stacks --stack-name Prices-Cicd --region eu-central-1 --query "Stacks[0].Outputs[?OutputKey=='ProductionDeployRoleArn'].OutputValue" --output text)"
   ```
   Check: the value is `arn:aws:iam::750702271865:role/stellar-prices-api-production-deploy`.

First test: the next release PR `develop` → `deploy/production`. Expected:
`build` green, `deploy` waits for an approval from someone other than the
merger, then `master` = the release commit and a `prod-*` tag on it.

### Phase 3: guard and docs

13. `infra/Makefile`: `deploy-production*` refuses unless the working tree is
    clean and `HEAD` equals a freshly fetched `origin/deploy/production`.
14. `infra/README.md`: the flow, a hotfix (PR to `deploy/production`, then merged
    back into `develop`), a rollback (a revert PR on `deploy/production`;
    `master` is never force-moved), the ClickHouse schema rule, and the other
    changes that are not branch state (data repairs, the portal sync), each
    noted on the release tag.
15. Once `promote` moves `master`, review the admin bypass on ruleset
    27628401: remove it, or narrow it to a team, since it covers every
    repository admin (seven on 2026-11-19).

#### Decisions (2026-11-20, Phase 3)

Answered by the operator.

14. **The guard has no override.** No variable skips it. An urgent fix is a
    PR to `deploy/production`, then a deploy of its tip (CI or by hand).
15. **The guard also covers `sync-portal-explorer` and every
    `destroy-production*`**, not only `deploy-production*`. `diff-production`
    stays unguarded, because a release PR is diffed from `develop`.
    `deploy-cicd` / `destroy-cicd` are not guarded (not asked; the deploy
    role, not production).
16. **A release's manual steps go in the tag's GitHub Release.** An annotated
    tag's message cannot be edited, so "noted on the release tag" means the
    Release of that tag, created by the workflow and edited with
    `gh release edit` (portal upload, ClickHouse schema apply, data repair:
    what, when in UTC, who).

#### Phase 3 implementation

- **#427, second commit:** job `release` after `deploy` creates the tag's
  GitHub Release, with the run and the release PR linked and an empty
  "Manual steps" section. **Its own job (emerged)**, holding
  `contents: write`, so the job with the AWS role and the deploy key keeps
  `contents: read`; `develop` has no rules, and a write token there could
  push to it.
- **#428** (`feat/0343_deploy-guard-and-release-docs`, merge after #427):
  - `tools/scripts/check-release-commit.sh`, the first prerequisite of all 13
    guarded targets: fetches `origin/deploy/production`, refuses a dirty tree
    (untracked files count, ignored ones do not) or a `HEAD` off its tip, and
    prints `git checkout --detach origin/deploy/production`. Checked on a
    real checkout: refused before any build started.
  - **Remote and branch are constants (emerged).** A first draft read them
    from `RELEASE_REMOTE` / `RELEASE_BRANCH`, which `RELEASE_BRANCH=develop`
    would have turned into an override; removed, and a test asserts no
    variable moves the guard.
  - `tools/scripts/release-guard.test.mjs`, 10 tests: the script against
    throwaway repositories, and the Makefile wiring (guard first in every
    target, absent from `diff-production`).
  - `infra/README.md`: "Releasing to production" (the flow, the manual diff,
    hotfix, rollback, deploying by hand, non-branch changes, the promote key
    and its rotation) and the rewritten CI/CD role section.
  - `docs/runbooks/deploy-ledger-processor.md` deploys from the release tip,
    not `develop` (emerged: it contradicted the guard).

## Acceptance Criteria

- [x] `deploy/production` exists, is protected, and takes only approved PRs
      (Phase 1; #426 stayed `BLOCKED` until approved and green, 2026-11-19)
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
- A release that changes only `lore/` or docs still runs the deploy
  (decision 8, 2026-11-20).
- [[0141]] is the history behind `build-lambdas`: a deploy shipped stale Lambda
  assets from a laptop. Building in CI removes that class of error.
