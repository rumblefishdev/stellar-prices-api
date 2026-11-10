---
id: "0342"
title: "NAV / fundamental price for tokenised RWAs next to the market price — USDY trades below NAV, BENJI and WisdomTree come back unpriced"
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [layer-backend, api, oracles, rwa, external-request, priority-medium, effort-large]
links:
  - "https://github.com/rumblefishdev/stellar-prices-api/issues/417"
  - "../../../packages/prices-ingest-core/src/writer.rs"
  - "../../../packages/prices-api/src/oracles/handlers.rs"
history:
  - date: "2026-11-10"
    status: backlog
    who: mkowalski
    note: >
      Created from hackathon feedback by Dominik (Inferera). External request,
      tracked in GH #417. Tell Dominik when it ships.
---

# NAV / fundamental price for tokenised RWAs

> **External request:** suggested by **Dominik (Inferera)** after using the API
> at the hackathon. Tracked in [GH #417](https://github.com/rumblefishdev/stellar-prices-api/issues/417).
> **When this is completed, let Dominik know** and close #417.

## Summary

For a tokenised fund, the trade price isn't the valuation. USDY trades 0.7–2%
below its NAV. Funds that are minted and redeemed through the issuer and rarely
or never trade (BENJI, WisdomTree) get no price at all. Add a NAV / "fundamental"
price, sourced from on-chain feeds such as RedStone, as a separate field next to
the market price. The API could then value fund shares, not only traded tokens.

## Context

- Our prices come from trades, so an asset with no trades gets no price.
- RedStone is already ingested, but its samples carry no asset and are stored
  under the sentinel id `0` (`prices-ingest-core/src/writer.rs`, `oracle_row`).
  Nothing links a feed to a token yet.
- Adding the field changes the public response shape, so it needs an ADR.

## Implementation Plan

1. **Research (gate):** list which NAV feeds RedStone (and other providers)
   actually publish *on Stellar* for USDY, BENJI and WisdomTree tokens: contract,
   feed id, update cadence, and how the value is defined (NAV per share vs. a
   price). If no feeds exist on Stellar, decide whether to stop or use issuer data.
2. Map each feed to its Stellar asset identity.
3. ADR: field name (`nav_usd` / `fundamental_price_usd`), source, timestamp and
   staleness semantics, and wording that it is issuer-reported, not a market price.
4. Ingest + store per asset; expose it on the asset endpoint(s). Assets with a
   NAV but no trades must return a response instead of `404`.

## Acceptance Criteria

- [ ] Research note recording which NAV feeds exist on Stellar and for which assets
- [ ] ADR accepted for the new field
- [ ] USDY returns both market price and NAV
- [ ] BENJI / WisdomTree return a NAV even with no trades
- [ ] Docs explain market price vs. NAV
- [ ] Dominik (Inferera) informed; GH #417 closed
