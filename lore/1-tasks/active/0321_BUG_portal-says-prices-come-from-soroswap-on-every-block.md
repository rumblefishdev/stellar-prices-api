---
id: "0321"
title: "Portal says prices come straight from Soroswap and update on every block — the product reads five venues and prices USD hourly"
type: BUG
status: active
related_adr: []
related_tasks: ["0318", "0316", "0193"]
tags: [layer-frontend, portal, copy, epic-self-service-onboarding, priority-medium, effort-small]
links:
  - "../../../web/portal/src/landing/Faq.tsx"
  - "../../../web/portal/src/landing/Features.tsx"
history:
  - date: "2026-09-29"
    status: backlog
    who: stkrolikiewicz
    note: >
      Found by the 0318 claim-by-claim audit of the agent skill; the same
      sentence also sits in the Features section.
  - date: "2026-10-01"
    status: active
    who: stkrolikiewicz
    note: >
      Activated for the Milestone 3 video, which shows the portal; both
      sentences are still in the live bundle (checked 2026-10-01).
---

# Portal says prices come straight from Soroswap and update on every block

## Summary

Two public portal texts describe a data source and a cadence the API does not
have. Rewrite both to match the API's own spec.

## Context

- `web/portal/src/landing/Faq.tsx:81` ("How often are prices updated?"):
  "Prices come straight from Soroswap liquidity pools and are updated on every
  block."
- `web/portal/src/landing/Features.tsx:47`: "Sourced directly from Soroswap
  liquidity pools, updated on every block."

What the API actually does, per the published spec
(`packages/prices-api/src/openapi/descriptions.rs`, `PriceResponse`):

- **Venues.** Prices come from five venues: `sources` is keyed `sdex`,
  `soroswap`, `aquarius`, `phoenix` and `sushiswap`. USDC is priced from a
  Reflector rate (`method: oracle`).
- **Refresh.** Snapshots refresh every minute (`updated_at`).
- **USD pricing.** USD values come from an hourly pass, so `price_status:
  carried` for up to about an hour is the ordinary state of an active asset.
  "Every block" (~5 s ledgers) overstates freshness by two orders of magnitude.

The skill (`skills/stellar-prices-api/SKILL.md`, 0318) already states this
correctly. Keep the two in agreement.

## Implementation Plan

1. Rewrite both sentences from the spec's wording. The venue list, "refreshed
   every minute" and "USD values computed hourly" are the facts to keep.
2. Grep the portal for any other "Soroswap" / "every block" claim. On
   2026-09-29 only these two existed.
3. Deploy with the next portal release. 0316 also edits portal files, so batch
   the two if they are close.

## Acceptance Criteria

- [ ] Neither text names a single venue as the source, nor claims per-block updates
- [ ] Wording matches the spec (five venues, per-minute refresh, hourly USD)
- [ ] Portal deployed and the live page checked
