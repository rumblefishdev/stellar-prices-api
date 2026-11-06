---
id: "0340"
title: "Serve the Prices API at prices-api.sorobanscan.com, then move the portal to sorobanscan.com"
type: FEATURE
status: active
related_adr: ["0010"]
related_tasks: ["0339", "0326", "0194", "0186", "0318"]
tags: ["hosting", "portal", "domains", "priority-medium", "effort-medium"]
links:
  - https://github.com/rumblefishdev/soroban-block-explorer/blob/develop/lore/1-tasks/backlog/0659_OPS_move-public-domains-to-sorobanscan-com.md
  - https://github.com/rumblefishdev/soroban-block-explorer/blob/develop/lore/1-tasks/active/0664_OPS_redirect-old-explorer-hosts.md
history:
  - date: 2026-11-05
    status: backlog
    who: stkrolikiewicz
    note: >
      Task created. Step 6 of the explorer's domain move (its task 0659) had no
      task here. The explorer moved to sorobanscan.com on 2026-11-05, but the
      portal cannot follow while this API stays on *.rumblefish.dev.
  - date: 2026-11-05
    status: active
    who: stkrolikiewicz
    note: >
      Started: Step 1 (the second custom domain) on
      feat/0340_prices-api-on-prices-api-sorobanscan-com.
---

# Serve the Prices API at prices-api.sorobanscan.com, then move the portal to sorobanscan.com

## Summary

The Prices API gets a second host, `prices-api.sorobanscan.com`, beside
`prices-api.sorobanscan.rumblefish.dev`. Then the portal moves from
`sorobanscan.rumblefish.dev/api/prices-api/` to `sorobanscan.com/api/prices-api/`
in one window with the backend's own config. The names were agreed by the team
on 2026-11-03 (explorer task 0659, step 6).

## Status: Active

**Current state:** Steps 1–4 done (2026-11-06). The portal runs on
`https://sorobanscan.com/api/prices-api/` against `prices-api.sorobanscan.com`,
a Discord sign-in passed there, and the old portal host answers `301`. The old
API host `prices-api.sorobanscan.rumblefish.dev` keeps serving API-key clients.
Step 5's docs part is in progress on `feat/0340_docs-name-new-host`. Still
open: a key test on the new host, then the old host's fate.

- Step 5, docs (2026-11-06): the endpoint reference, the overview, the OAuth
  runbook, the skill, the portal's logo link and the comments that state the
  current hosts name `sorobanscan.com`. The skill reaches readers from
  `master` (its `copyValue`, 0318), so after the next develop→master merge.
  - Kept on the old host on purpose: dated SCF evidence and runbooks; the
    cache verification and the k6 load test, because they measure API
    Gateway and the new host puts Cloudflare in front. They need another
    target before the old host retires.
  - The Stellar catalog entry (stellar/stellar-dev-skill#141, merged) names
    `sorobanscan.rumblefish.dev/prices-api/`; it reaches the portal in one
    `301`.

- Done: Step 3 (2026-11-06, Adam). The Discord application holds the new
  redirect URI beside the old one. Checked by an authorize link that showed the
  consent screen with `https://prices-api.sorobanscan.com`.
- Done: Step 4 (2026-11-06, about 08:57–09:00 UTC). #412 merged as `8ed247d3`.
  - `redirect_uri` in `prices/production/portal-discord-oauth` → the new
    callback. The previous version `c34106e0…` is kept as `AWSPREVIOUS`.
  - `deploy-production-apigateway` and `-compute`: CORS, `API_BASE_URL`,
    `PORTAL_WEB_ORIGIN`.
  - `sync-portal-explorer`, with the bundle built against the new host.
  - Explorer: `pricesPortalOnLegacyHost: false`, release #719, Delivery from
    a laptop.
  - Checked: preflight from `sorobanscan.com` answers `204` with that origin and
    credentials; sign-in starts with the new `redirect_uri`; the callback lands
    on `https://sorobanscan.com/api/prices-api/`; a person signed in.
- Found (2026-11-06): CI on #412 pinned two copies of `apiBaseUrl`:
  `PUBLIC_API_BASE_URL` in `web/portal/src/landing/links.ts` and `servers` in
  `web/portal/public/openapi.json`. Both moved with it. The logo's `EXPLORER`
  link still names the old explorer host (it answers `301`), because nine
  `app.spec.tsx` cases pin it. Change it with Step 5.

- Done: Step 1. #411, deployed with `make deploy-production-apigateway` from
  `3dc847d7`. The stack's first deploy since 2026-09-24 also shipped 0309's
  `404` for unmapped routes. Regional target
  `d-in7vpbow2m.execute-api.eu-central-1.amazonaws.com`.
- Done: Step 2. soroban-infrastructure#9: a proxied CNAME, `prices_api_origin`.
- Decided (2026-11-05): proxied, like the rest of `sorobanscan.com` behind
  Cloudflare, with no rules of its own yet. The zone's single Free-plan rate
  limit (100 / 10 s per IP) stays the explorer API's, because a paid plan here
  allows 25 requests/s; API Gateway's usage plans keep limiting. Managed WAF
  comes after watching the traffic. An `X-Edge-Secret` origin lock comes only
  with Step 5, while the old host and `execute-api` still answer. The backend
  reads no client IP.

## Context

- **Why the portal cannot move alone.** Its session cookie is `SameSite=Lax`
  and host-only (`portal/auth/cookies.rs`). The browser sends it to the backend
  only while page and backend are same-site: today both are under
  `rumblefish.dev`. A page on `sorobanscan.com` calling
  `prices-api.sorobanscan.rumblefish.dev` is cross-site, so the cookie never
  arrives and the portal looks signed out. CORS allows one origin,
  `portalWebOrigin`; that is the smaller half.
- **Why a subdomain, not a path on `sorobanscan.com`.** Any host under
  `sorobanscan.com` is same-site with the portal. A path would route this API
  through the explorer's CloudFront and collide with the `/api/` the explorer
  host already uses (hub, portal) and this backend uses (`/api/auth/*`,
  `Path=/api/`). Decided 2026-11-05: subdomain, as agreed in 0659.
- **What exists.** The `*.sorobanscan.com` certificate in eu-central-1
  (`c937aeef…`, explorer task 0661) covers `prices-api.sorobanscan.com`, same
  AWS account. The `sorobanscan.com` zone is soroban-infrastructure's
  `cloudflare/`. The old host is a Route 53 alias to this API Gateway domain.
- **Who.** soroban-infrastructure's README names the migration Karol's. The
  Discord application and its redirect URI are Adam's alone (ADR 0010 §6,
  `docs/runbooks/portal-oauth-deploy-prep.md`).

## Implementation Plan

### Step 1: A second custom domain (CDK)

`prices-api.sorobanscan.com` on the same API, as the explorer's 0662 did: an
optional domain and its certificate ARN, the regional target as an output. The
old domain keeps serving.

### Step 2: The record (soroban-infrastructure)

`prices-api.sorobanscan.com` → that regional target, proxied (see Decided). The
Free plan has one rate-limit rule for the whole zone, already the explorer API's.

### Step 3: Discord

Adam adds `https://prices-api.sorobanscan.com/api/auth/callback` as a second
redirect URI, beside the current one. Matching is treated as character-exact
(runbook §1).

### Step 4: Switch, in one window

- Here: `apiBaseUrl` and `portalWebOrigin` (`https://sorobanscan.com`) in
  `infra/envs/production.json`, and the `redirect_uri` in the Discord OAuth
  secret (runbook §2); ApiGateway and Compute. Then the portal bundle, built
  against the new API host.
- Explorer: `pricesPortalOnLegacyHost: false` (Delivery), so its old host sends
  the portal to `sorobanscan.com` with a `301`, like everything else.
- Signed-in users sign in again once: the cookie is host-only.

### Step 5: The old API host

Keep it serving for API-key clients for a while, with docs, the skill
(skills.stellar.org, [[0318]]) and examples pointing at the new host. Then a
`308` or retirement. API Gateway cannot redirect by itself; decide when we get
there.

## Acceptance Criteria

- [ ] `https://prices-api.sorobanscan.com/v1/…` answers like the old host,
      with the same API keys.
- [x] A Discord sign-in round-trip completes on
      `https://sorobanscan.com/api/prices-api/`, done by a person on production
      (2026-11-06).
- [x] `https://sorobanscan.rumblefish.dev/api/prices-api/…` answers `301` to
      the same path on `sorobanscan.com` (explorer flag off, 2026-11-06).
- [x] Docs, the skill and the API examples name the new host (2026-11-06;
      dated evidence kept as submitted).
- [x] **Docs updated** — README, `docs/runbooks/portal-oauth-deploy-prep.md`,
      `infra/README.md` (no host left in it).

## Notes

- Explorer side of the same window: soroban-block-explorer task 0664
  (`pricesPortalOnLegacyHost`), umbrella 0659.
- Dated SCF evidence stays as submitted, as in [[0326]] and [[0339]].
