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
  - date: "2026-08-14"
    status: backlog
    who: akot
    note: >
      Spawned from [[0184]] on close. That task's code is merged (#209) but four
      of its properties are code-only, and reaching the committed shape needs
      three deploys rather than one — so it is real outstanding work rather than
      a checkbox, and it would have been lost in the archive.
  - date: "2026-08-17"
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
  - date: "2026-08-17"
    status: backlog
    who: akot
    note: >
      Gave the `DependsOn` fix back to [[0185]] the same day, on a second review
      pass: that task introduces both `BucketDeployment`s, the fix is one line in
      a file it already edits, and this task sits in `backlog/` — so any deploy
      between the two would have worn the race the reassignment was meant to
      manage. Verified present on [[0185]]'s branch. This is a pure
      re-measurement task again; what is left of the item is a **precondition**
      (deploy a tree that includes [[0185]]) plus one thing to check live —
      [[0185]]'s decision 13, on whether `/api-tokens/` can hold a cache entry at
      all. −1 code change.
---

# Deploy the portal gateway mapping and verify it live

## Summary

[[0184]] is merged and its acceptance criteria were measured — but against the
**2026-08-13 deploy**, and four properties landed after it. Production therefore
serves an intermediate state that matches neither the branch nor the original
deploy. This task ships it and re-measures.

Nothing here is a code change. The one exception this task briefly carried — a
missing `DependsOn` between the two `BucketDeployment`s — went back to [[0185]]
the same day and is already on that branch; anything beyond re-measurement means
something is wrong with [[0184]] rather than with the deploy.

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
3. `make -C infra deploy-production-portal`. Access logs, upload
   `Cache-Control`, the redirect function. Upload and invalidation happen inside
   the same `cdk deploy`. **Deploy [[0185]]'s branch, not an older one** — see
   the note below.

### Deploy the bucket-deployment ordering fix with the bundle

This task briefly owned a one-line fix and gave it back. [[0185]] split
[[0184]]'s single `BucketDeployment` in two — content-hashed `assets/*` at a
year and `immutable`, the unhashed `index.html` at `max-age=0, must-revalidate`
— and on 2026-08-17 neither carried a `DependsOn`: both custom resources read
`DependsOn: null` in the synthesized template, so CloudFormation ran them
concurrently. If the entry document landed first, CloudFront served a fresh
`index.html` referencing chunk names not yet in the bucket, and the bucket
grants `s3:GetObject` without `s3:ListBucket`, so the miss came back as
`403 AccessDenied` — the app failing on its own JavaScript, with the
invalidation firing inside the same window.

**[[0185]] took it back** and now carries
`portalBundle.node.addDependency(portalBundleAssets)` (its decision 14): that
task introduces both deployments, the fix is one line in a file it already
edits, and this task sitting in `backlog/` meant any deploy in between would
have worn the race. Verified on that branch —
`DependsOn: [PortalBundleAssetsAwsCliLayer…, PortalBundleAssetsCustomResource…]`.

Nothing to apply here. What remains is a **precondition**: the first real-bundle
deploy must run from a tree that includes [[0185]], or it reintroduces the race
it was written to avoid.

One thing to check while measuring: [[0185]]'s decision 13 keeps `/api-tokens/`
in `distributionPaths` on the belief that `DirectoryIndexFn`'s VIEWER_REQUEST
rewrite happens ahead of the cache lookup, which would mean that path never
holds an entry. If the live deploy confirms it, drop the path; it is free either
way, so this is tidiness, not a fix.

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
