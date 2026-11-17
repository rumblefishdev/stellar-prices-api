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
      Returned to backlog, unassigned; handed over to another team member.
      No code written. Finding: the alias already exists on develop (0242 D5,
      PR #390), but develop is 163 commits ahead of master and it is unclear
      what production runs. See "Handover findings".
  - date: "2026-11-10"
    status: active
    who: mkowalski
    note: "Activated for implementation."
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

## Handover findings (2026-11-10)

**The alias is already implemented on `develop`, so most likely no new code is
needed.** Check this before planning work:

- [[0242]] decision D5, commit `3de11181`, merged to `develop` in PR #390 on
  2026-11-02: `resolve_sac_alias(es)` in
  `packages/prices-api/src/assets/queries_ch.rs` maps a SAC `C…` address to its
  classic asset (XLM's SAC → `native`). Single-asset, history, oracle and batch
  handlers call it. The OpenAPI text says SAC addresses are "accepted as an
  alias". `tests/endpoints_it.rs` covers XLM and XCR (incl. a pre-heal duplicate).
- **None of it is on `master`.** `develop` is 163 commits / 20 PRs ahead of
  `master` (host move 0340, stellar-xdr 30 bump 0335, pricing fixes 0329/0331/0332, …).
- **It is unclear what production actually runs** (confirmed with the team), so
  it's also unclear why the hackathon user got a 404.

Suggested next steps: find out what production runs, call
`GET /assets/{XLM SAC}` against it, and then either ship via a develop → master
release or close this task as done by 0242. The Implementation Plan below was
written before this was known and is likely obsolete.

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
