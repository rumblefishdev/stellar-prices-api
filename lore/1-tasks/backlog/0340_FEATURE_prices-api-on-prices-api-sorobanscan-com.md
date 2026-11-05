---
id: "0340"
title: "Serve the Prices API at prices-api.sorobanscan.com, then move the portal to sorobanscan.com"
type: FEATURE
status: backlog
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
---

# Serve the Prices API at prices-api.sorobanscan.com, then move the portal to sorobanscan.com

## Summary

The Prices API gets a second host, `prices-api.sorobanscan.com`, beside
`prices-api.sorobanscan.rumblefish.dev`. Then the portal moves from
`sorobanscan.rumblefish.dev/api/prices-api/` to `sorobanscan.com/api/prices-api/`
in one window with the backend's own config. The names were agreed by the team
on 2026-11-03 (explorer task 0659, step 6).

## Status: Backlog

**Current state:** nothing started. The explorer serves on `sorobanscan.com`
since 2026-11-05 and keeps the portal on its old host with
`pricesPortalOnLegacyHost`: the new host answers `302` to the old one (explorer
task 0664).

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

`prices-api.sorobanscan.com` → that regional target. Decide first: DNS-only, as
the API is today (API Gateway straight, no Cloudflare), or proxied with an
`X-Edge-Secret` lock like the explorer API. The Free plan has one rate-limit
rule for the whole zone, already the explorer API's.

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
- [ ] A Discord sign-in round-trip completes on
      `https://sorobanscan.com/api/prices-api/`, done by a person on production.
- [ ] `https://sorobanscan.rumblefish.dev/api/prices-api/…` answers `301` to
      the same path on `sorobanscan.com` (explorer flag off).
- [ ] Docs, the skill and the API examples name the new host.
- [ ] **Docs updated** — README, `docs/runbooks/portal-oauth-deploy-prep.md`,
      `infra/README.md`.

## Notes

- Explorer side of the same window: soroban-block-explorer task 0664
  (`pricesPortalOnLegacyHost`), umbrella 0659.
- Dated SCF evidence stays as submitted, as in [[0326]] and [[0339]].
