---
id: "0351"
title: "The GitHub OIDC provider lives in Explorer-Cicd and both CI pipelines depend on it — move it to soroban-infrastructure"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0343"]
tags: [layer-infra, priority-low, effort-small, milestone-M3, ci, iam, aws, be-interop]
milestone: 3
links:
  - "../../../infra/src/lib/stacks/cicd-stack.ts"
  - https://github.com/rumblefishdev/soroban-infrastructure
  - https://github.com/rumblefishdev/soroban-block-explorer/blob/develop/infra/src/lib/stacks/cicd-stack.ts
history:
  - date: 2026-11-20
    status: backlog
    who: okarcz
    note: >
      Spawned from 0343 Phase 2. The team chose to import Explorer-Cicd's
      provider for now (option 1) and to move it to soroban-infrastructure
      later (option 3, this task).
---

# The GitHub OIDC provider lives in Explorer-Cicd and both CI pipelines depend on it — move it to soroban-infrastructure

## Summary

GitHub Actions logs in to the shared AWS account through one OIDC provider,
and an account can hold only one for GitHub. soroban-block-explorer's
`Explorer-Cicd` stack creates it; `Prices-Cicd` imports it by ARN ([[0343]]).
If `Explorer-Cicd` is deleted, or the provider is removed from its code,
CloudFormation deletes the provider and both production pipelines stop, with
no warning. This task gives the provider one shared owner:
`soroban-infrastructure`, which holds the infrastructure that belongs to none
of the projects alone.

## Context

Measured on 2026-11-20 (read-only, `soroban-readonly`):

- The account has one provider:
  `arn:aws:iam::750702271865:oidc-provider/token.actions.githubusercontent.com`,
  audience `sts.amazonaws.com`, thumbprint `6938fd4d98bab03faadb97b34396831e3780aea1`,
  created 2026-08-12.
- It is `GitHubOidcProvider7EBF861F` in `Explorer-Cicd`, a CDK custom
  resource (`Custom::AWSCDKOpenIdConnectProvider`) with two helper resources
  (a Lambda and a role).
- The explorer's `deploy-production.yml` and, after [[0343]], this repo's
  deploy workflow both log in through it.
- The ARN holds only the account and the URL, so a re-created provider has
  the same ARN and the importing stacks need no change.

Precedent: `cloudflare/state-bucket.yaml` in soroban-infrastructure was moved
out of the explorer's CDK app (explorer task 0277) without replacing the
bucket.

Decided with the team on 2026-11-20: option 1 (import as is) for 0343, this
move later. Option 2 (only `RETAIN` in the explorer) was not chosen.

## Implementation Plan

The order is fixed: the provider must be retained before it leaves the
explorer's code, or CloudFormation deletes it.

1. **soroban-block-explorer PR:** `applyRemovalPolicy(RemovalPolicy.RETAIN)`
   on the provider in `infra/src/lib/stacks/cicd-stack.ts`, then
   `make deploy-cicd`. Check that the deployed template carries
   `DeletionPolicy: Retain` on `GitHubOidcProvider7EBF861F`.
2. **soroban-block-explorer PR:** replace `new iam.OpenIdConnectProvider` with
   `iam.OpenIdConnectProvider.fromOpenIdConnectProviderArn`, then
   `make deploy-cicd`. CloudFormation drops the provider and its two helper
   resources from the stack and leaves the provider. Check that it still
   exists and that the explorer's next production deploy logs in.
3. **soroban-infrastructure PR:** a CloudFormation template
   (`AWS::IAM::OIDCProvider`, `DeletionPolicy: Retain`, today's URL, audience
   and thumbprint) brought in with an `IMPORT` change set, like
   `state-bucket.yaml`; a README section on what depends on it. First confirm
   that CloudFormation supports importing `AWS::IAM::OIDCProvider`.
4. **stellar-prices-api:** nothing to change, since `Prices-Cicd` already
   imports by ARN. Update the comment in `cicd-stack.ts` to name the new owner.

Rough size: half a day to a day of work, three PRs in two repos and three
deploys, spread over a few days by review.

## Acceptance Criteria

- [ ] The provider is defined in soroban-infrastructure, with `DeletionPolicy: Retain`
- [ ] `Explorer-Cicd` and `Prices-Cicd` only import it by ARN
- [ ] The provider kept its ARN through the move (never deleted)
- [ ] One production deploy of each project logs in through it after the move

## Notes

- If step 2 ever runs before step 1 is live, the provider is deleted. Re-create
  it with the same URL, audience and thumbprint; the ARN is the same, so both
  pipelines recover with no code change.
- Deploying `Explorer-Cicd` and the new stack needs `soroban-admin`.
