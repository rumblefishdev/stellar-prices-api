---
id: "0348"
title: "Answer the API hub's dashboard contract: account, reveal, revoke, issue"
type: FEATURE
status: active
related_adr: ["0010"]
related_tasks: ["0340", "0186", "0187", "0188", "0189"]
tags: ["portal", "hub", "priority-high", "effort-medium"]
links:
  - https://github.com/rumblefishdev/sorobanscan-api-portal/blob/main/docs/superpowers/specs/2026-11-17-shared-sign-in-design.md
  - https://github.com/rumblefishdev/sorobanscan-api-portal/blob/main/docs/superpowers/plans/2026-11-18-prices-hub-contract-plan.md
history:
  - date: 2026-11-18
    status: active
    who: akot
    note: >
      Task created and started. Stage 2 of the shared sign-in: the hub's
      dashboard at sorobanscan.com/api/dashboard shows the Prices tab through
      the hub's backend, which calls these four routes server-to-server.
---

# Answer the API hub's dashboard contract: account, reveal, revoke, issue

## Summary

`https://sorobanscan.com/api` becomes one hub for every API, with one Discord
sign-in and one dashboard with a tab per product (`rumblefishdev/sorobanscan-api-portal`).
The hub's backend (BFF) holds the session and calls each product server-to-server
with a 60-second HMAC-signed token. Prices answers four calls under
`/api/hub/v1/`: `GET account`, `POST key/reveal`, `POST key/revoke`,
`POST key/issue`, reusing its own key, cap and usage logic. Today's portal routes
keep working unchanged until the hub takes over (cutover step 3 of the design).

## Status: Active

**Current state:** Task created; nothing built yet.

## Context

- Design: `docs/superpowers/specs/2026-11-17-shared-sign-in-design.md` in
  `sorobanscan-api-portal` ("The dashboard contract", decisions 11–16; decision
  15 is the token format with a test vector both sides pin).
- Plan: `docs/superpowers/plans/2026-11-18-prices-hub-contract-plan.md` there.
- A change from today's portal (akot, 2026-11-17, design decision 4): no key is
  issued on sign-in. An account without a key gets one only from "Get API Key"
  in the hub's Prices tab, which reaches `key/issue` with `act=issue`.

## Implementation Plan

1. Extract the portal's key and usage services from the handlers, no behaviour
   change (PR 1).
2. Verify the hub's token (`v1.<payload>.<mac>`, HMAC-SHA256, `current` or
   `previous` secret from Secrets Manager `sorobanscan-hub/production/prices-token`).
3. The four routes, inside the `PORTAL_ENABLED` gate, outside the portal's CORS.
4. Infra (`PORTAL_HUB_SECRET_NAME`, read on that one secret), runbook, ADR 0010
   amendment (PR 2). Deploy is akot's.

## Acceptance Criteria

- [ ] Given the design's test vector, when Prices verifies it, then it is accepted
      inside its lifetime and refused after it
- [ ] Given a token signed with another secret, malformed, expired or living
      longer than 60 s, when any route is called, then 401 and no AWS call
- [ ] Given a `read` token on `key/revoke` or `key/issue`, then 403 and no AWS call
- [ ] Given no or a bad hub secret, then the contract answers 503
      `hub_unconfigured` and the portal and `/api/config` still work
- [ ] Given an account with no key, when `key/issue` is called with an `issue`
      token, then it gets a key under Prices' rules (one active, month cap after
      a revoke, account-age backstop)
- [ ] Given today's portal routes, then their behaviour and tests are unchanged
