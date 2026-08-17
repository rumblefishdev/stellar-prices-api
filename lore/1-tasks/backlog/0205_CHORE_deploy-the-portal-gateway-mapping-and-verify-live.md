---
id: '0205'
title: 'Deploy the portal gateway mapping and verify it live — three deploys 0184 merged but never shipped'
type: CHORE
status: backlog
related_adr: []
related_tasks: ['0184', '0183', '0185', '0186', '0194', '0141']
tags:
  [
    layer-infra,
    priority-high,
    effort-small,
    milestone-M3,
    epic-self-service-onboarding,
    deployment,
    cloudfront,
    apigateway,
  ]
milestone: 3
links:
  - '../active/0184_FEATURE_portal-hosting-skeleton.md'
  - '../../../docs/scf/api-endpoints.md'
history:
  - date: 2026-08-14
    status: backlog
    who: akot
    note: >
      Spawned from [[0184]] on close. That task's code is merged (#209) but four
      of its properties are code-only, and reaching the committed shape needs
      three deploys rather than one — so it is real outstanding work rather than
      a checkbox, and it would have been lost in the archive.
  - date: 2026-08-17
    status: backlog
    who: akot
    note: >
      Scope grew by one item during a review of [[0185]]: the two
      `BucketDeployment`s that task introduced carry no `DependsOn`, verified
      `null` in the synthesized template. Assigned here because this task
      performs the first deploy of a real bundle, which is the only place the
      race can bite. This makes the task's "nothing here is a code change"
      premise hold with one stated exception rather than absolutely — amended in
      the summary. Also rewrote the `Cache-Control` criterion, which measured
      "the placeholder" — an object [[0185]] deleted, replaced by a real bundle
      whose two halves take different headers, so the single criterion could no
      longer be checked as written. +1 code change, +3 acceptance criteria
      (2 new, 1 split in two).
---

# Deploy the portal gateway mapping and verify it live

## Summary

[[0184]] is merged and its acceptance criteria were measured — but against the
**2026-08-13 deploy**, and four properties landed after it. Production therefore
serves an intermediate state that matches neither the branch nor the original
deploy. This task ships it and re-measures.

Nothing here is a code change **except one**, added 2026-08-17: a missing
`DependsOn` between the two `BucketDeployment`s that [[0185]] introduced. That
exception is scoped and justified below; anything beyond it means something is
wrong with [[0184]] rather than with the deploy.

## Context

Production today:

- the gateway maps `ANY /api-tokens/api/{proxy}` + `{proxy}/{sub}` with **no
  throttle** — left by the failed 2026-08-14 deploy, correct at depth 1–2 and
  `403` at depth 3
- CloudFront access logs: **off**
- `Cache-Control` on the uploaded objects: **absent**
- the trailing-slash redirect: **absent**, so `/api-tokens` answers
  `403 AccessDenied` rather than `302`

`PORTAL_ENABLED` is `false` and stays that way — opening the portal is
[[0194]]'s acceptance criterion, gated on 0189. This task changes what the
routes *are*, never whether they answer.

## Implementation

Three deploys, in order. Two, not one, because `{proxy}` and `{proxy+}` cannot
both be children of `/api-tokens/api` mid-update: a resource may have at most one
variable child and CloudFormation creates before it deletes. `cdk diff` shows
this as an unremarkable create + delete — see [[0184]]'s issue log, where getting
this wrong cost a 20-minute outage on the prefix.

**Before anything:** rebuild the api-handler bootstrap. These targets carry no
`--exclusively`, so `deploy-production-apigateway` pulls `Compute` in with them,
and `Code.fromAsset` packages whatever is in `target/lambda/` without a freshness
check — [[0141]].

```bash
cargo lambda build -p prices-api --release --arm64 --features lambda
make -C infra diff-production   # read it for REMOVALS, not additions
```

1. Comment out the `portalProxy` block and `portalSettings` in
   `api-gateway-stack.ts`, then `make -C infra deploy-production-apigateway`.
   Deletes the live `{proxy}` and `{proxy}/{sub}`. **The portal prefix answers
   `403` for the length of the gap** — which is why this happens before [[0185]]
   has a bundle that calls it. The edit is local and must not be committed.
2. Restore the file, `make -C infra deploy-production-apigateway`. Creates
   `{proxy+}`, its three verbs and the per-verb throttle.
3. Apply the `DependsOn` fix below, then `make -C infra deploy-production-portal`.
   Access logs, upload `Cache-Control`, the redirect function. Upload and
   invalidation happen inside the same `cdk deploy`.

### The one code change: order the two bucket deployments

[[0185]] split [[0184]]'s single `BucketDeployment` in two — content-hashed
`assets/*` at a year and `immutable`, the unhashed `index.html` at
`max-age=0, must-revalidate` — and neither carries a `DependsOn`. Verified
against the synthesized template on 2026-08-17: both custom resources have
`DependsOn: null`, so CloudFormation may run them in either order.

If the entry document lands first there is a window where a fresh `index.html`
references chunk names that are not in the bucket yet. The bucket grants
`s3:GetObject` and not `s3:ListBucket`, so a missing key reads as
`403 AccessDenied` — the app 403s on its own JavaScript and renders a blank
page. Worse, the invalidation hangs off the entry-document deployment, so that
`index.html` is the one CloudFront starts serving.

```ts
// in portal-hosting-stack.ts — assets must be in place before the document
// that points at them, and before the invalidation that publishes it.
entryDocumentDeployment.node.addDependency(assetsDeployment);
```

**Why here and not in [[0185]]:** the race needs two deployments to exist, which
is [[0185]]'s change, but it can only *bite* on a deploy — and this task performs
the first deploy of a real bundle. Fixing it in [[0185]] would have been fine
too; it is here because that is where it was caught, and shipping it separately
would mean deploying the known-racy shape once on purpose.

This is a one-line ordering change with no effect on the synthesized resources
themselves, so it does not reopen [[0185]]'s acceptance criteria.

Then delete the two "ahead of the deploy" notes — one in
`docs/scf/api-endpoints.md`, one in [[0184]]'s record — and mark [[0184]]'s
deferred criteria.

## Acceptance Criteria

- [ ] `/api-tokens` returns `302` to `/api-tokens/`, not `403 AccessDenied`
- [ ] `/api-tokens/index.html` carries
      `Cache-Control: public, max-age=0, must-revalidate` — the entry document
      must revalidate, or a visitor keeps booting a stale app from a URL that
      still resolves
- [ ] Everything under `/api-tokens/assets/` carries
      `Cache-Control: public, max-age=31536000, immutable` — safe only because
      those names are content-hashed, so a new build is a new URL
- [ ] `/api-tokens/api/a/b/c` returns an empty `404` — greedy matches any depth
      again, so the depth-3 `403` is gone
- [ ] The deployed stage carries a throttle entry per verb (`GET`, `POST`,
      `DELETE`) at 10 req/s burst 40, with caching off
- [ ] CloudFront access logs are landing in the log bucket, without cookies
- [ ] `/api-tokens/api/config` still answers `200 {"enabled":false}` with
      `no-store`, and every other path under the prefix still answers an empty
      `404` — the flag is untouched by this task
- [ ] `/health`, `/api-docs-json` and `/v1/assets` (keyless → `403`) unchanged
      throughout — the data routes must not notice this happening
- [ ] Both "ahead of the deploy" notes deleted
- [ ] The synthesized `PortalHosting` template shows the entry-document
      deployment depending on the asset deployment — neither is `DependsOn: null`
      any more
- [ ] On a cold cache after the deploy, `/api-tokens/` loads and every asset it
      references returns `200`; nothing under `/api-tokens/assets/` answers
      `403 AccessDenied`
