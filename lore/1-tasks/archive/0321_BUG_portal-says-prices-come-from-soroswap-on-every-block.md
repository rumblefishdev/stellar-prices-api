---
id: "0321"
title: "Portal says prices come straight from Soroswap and update on every block — the product reads five venues and prices USD hourly"
type: BUG
status: completed
related_adr: []
related_tasks: ["0318", "0316", "0193", "0294"]
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
  - date: "2026-10-01"
    status: completed
    who: stkrolikiewicz
    note: >
      Closed. Three PRs, all merged: #378 (FAQ, Live Prices card, hero, DEX
      use case), #379 (Liquidity Data card to Volume Data) and #380 (the
      remaining claims the API does not back). Portal synced 2026-10-01
      12:49 UTC; the live bundle index-C8BkfZaW.js carries every new text and
      none of the old. 269 portal tests pass. The FAQ answers on plans and
      quota stay as they are, by the operator's decision.
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

- [x] Neither text names a single venue as the source, nor claims per-block updates
- [x] Wording matches the spec (five venues, per-minute refresh, hourly USD)
- [x] Portal deployed and the live page checked

## Implementation Notes

- **#378** (`8885d0d9`): the FAQ answer to "How often are prices updated?",
  the Live Prices card, the hero subtitle and the DEX Aggregators use case.
- **#379** (`a2e2493a`): the Liquidity Data card becomes Volume Data.
- **#380** (`fcf5c654`): the remaining claims the API does not back, in
  `UseCases.tsx`, `Features.tsx`, `FairAccess.tsx`, `Documentation.tsx` and
  `DeveloperDashboard.tsx`.
- Deployed with `make -C infra sync-portal-explorer` on 2026-10-01 at 12:49
  UTC (CloudFront invalidation `I4WV8G1AXLPIB177Y2A28IVFUZ`). The live bundle
  `index-C8BkfZaW.js` was checked for every new sentence and every old one.

## Design Decisions

### From Plan

1. **Wording from the spec and the skill.** Five venues, a snapshot every
   minute, USD values hourly. `price_usd` is the last close and `vwap_24h` the
   VWAP across venues, so the Live Prices card names both.

### Emerged

2. **The hero and the DEX Aggregators use case are included.** Step 2's grep
   found "Powered by Soroswap infrastructure" and "liquidity depth across
   Soroswap pools". Both sentences also claimed liquidity data, so they were
   rewritten whole.
3. **Liquidity Data became Volume Data** (#379). The card promised pool
   reserves, depth and liquidity metrics; no route, no response field and no
   line of the design document carries any of them. The new card describes the
   volume data that exists. Per-venue quotes carry a price and a volume, not a
   VWAP, so the card places the VWAP across venues.
4. **A pass over every landing text** (#380) removed further claims: values
   "in any other asset", series for "any Stellar asset pair", "high-frequency",
   "real-time", "arbitrage detection", "no throwaway signups" (the gate allows
   a five-minute-old account), "Every request requires an API key" (the
   OpenAPI document and the health route are anonymous) and "SDK" (none
   exists).
5. **The FAQ answers on plans and quota are unchanged**, by the operator's
   decision, although four higher usage plans exist since 2026-09-24 (0311).

## Issues Encountered

- **#378 was merged before its second commit reached it.** GitHub had not yet
  attached the Volume Data commit to the PR when it was merged, so that commit
  went in as #379.
