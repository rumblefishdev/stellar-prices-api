---
id: "0341"
title: "Accept a classic asset's SAC C… address as an alias — today it answers 404"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0242", "0139"]
tags: [layer-backend, api, identity, external-request, priority-medium, effort-small]
links:
  - "https://github.com/rumblefishdev/stellar-prices-api/issues/416"
  - "../../../packages/prices-api/src/identity.rs"
  - "../../../packages/prices-api/src/assets/handlers.rs"
  - "../../../packages/prices-ingest-core/src/writer.rs"
history:
  - date: "2026-11-10"
    status: backlog
    who: mkowalski
    note: >
      Created from hackathon feedback by Dominik (Inferera). External request,
      tracked in GH #416. Tell Dominik when it ships.
---

# Accept a classic asset's SAC C… address as an alias

> **External request:** suggested by **Dominik (Inferera)** after using the API
> at the hackathon. Tracked in [GH #416](https://github.com/rumblefishdev/stellar-prices-api/issues/416).
> **When this is completed, let Dominik know** and close #416.

## Summary

A Soroban caller usually knows a classic asset only by its Stellar Asset
Contract (SAC) address. Asking for e.g. USDC by its `C…` address returns
`404`; only `CODE:ISSUER` works. Resolve a SAC address to the classic asset it
wraps, so both names return the same data.

## Context

- Documented behaviour today: `assets/handlers.rs` says classic assets are named
  by `CODE:ISSUER` and their SAC addresses "as a rule answer `404`".
- The parser already accepts `C…` (`AssetIdentifier::Contract`, `identity.rs`).
- Ingest already collapses SAC legs into the classic asset and stores the SAC
  that wraps each classic asset (`prices-ingest-core/src/writer.rs`, §12.4).
- USDC already resolves by its SAC through a special case (`handlers.rs`,
  the id arm of the canonical-USDC check). This task generalises that.
- [[0242]] (SAC minted as a second identity) touches the same mapping. Check
  its state first so the alias doesn't resolve to a split identity.

## Implementation Plan

1. In the identity resolver, look up a `Contract(c)` as a SAC of a held classic
   asset first, and only then as a pure Soroban token.
2. Apply it to every route that takes `{asset_identifier}` (assets, oracles,
   history), not only `/assets/{id}`.
3. Update the OpenAPI parameter descriptions that currently promise `404`.
4. Decide whether the response echoes the requested `C…` or the canonical
   `CODE:ISSUER`, and document the choice.

## Acceptance Criteria

- [ ] `GET /assets/{SAC C…}` returns the same data as `GET /assets/{CODE:ISSUER}` for a held classic asset (incl. USDC, XLM's SAC → `native`)
- [ ] A pure Soroban token and an unknown `C…` behave as before
- [ ] OpenAPI docs no longer say SAC addresses answer `404`
- [ ] Integration test covering at least one non-USDC SAC
- [ ] Dominik (Inferera) informed; GH #416 closed
