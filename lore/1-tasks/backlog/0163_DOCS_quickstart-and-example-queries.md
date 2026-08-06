---
id: "0163"
title: "Quickstart guide and example queries, accurate against the live API"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0124", "0157", "0161", "0162", "0164"]
tags: [layer-docs, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, documentation, developer-experience]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../../docs/scf/api-endpoints.md"
history:
  - date: 2026-08-06
    status: backlog
    who: akot
    note: >
      Epic AC 3. Two named deliverables from the agreed scope (quickstart,
      example queries) that carry the last step of self-service: turning an
      issued key into a working request.
---

# Quickstart and example queries

## Summary

The document that takes a developer from "I have a key" to "I got data back".
The epic names both the quickstart and the example queries as deliverables, and
its acceptance criterion adds the harder word: **accurate**. A quickstart that
does not run is worse than none, because it moves the failure from our docs to
their debugging.

## Context

Self-service ends where this begins. [[0162]] hands over a key; without a first
request that works on the first try, the flow the reviewer signs off on is
incomplete in the only way a developer would notice.

Most of the raw material already exists and should not be duplicated: [[0124]]
publishes the spec at `/api-docs-json` with every route, parameter bound and
error shape, [[0161]] renders it as Swagger UI, and `docs/scf/api-endpoints.md`
carries the base URL. The quickstart's job is to be the short, opinionated path
through all that — not a second copy of it.

## Implementation

**From the epic**

- A quickstart guide.
- Example queries.
- Both accurate against the live API.

**Follows from the epic, but not stated in it**

- **Lead with a single copy-pasteable `curl`** that includes the base URL and
  the `x-api-key` header and returns real data. Everything else is second.
- **State the auth rule and its failure explicitly**: every request needs a key;
  a missing key is `403`. That is the epic's own line, and it is the first
  error anyone hits.
- **Explain the limits in the same place**: 1 req/s sustained (burst 5),
  the monthly quota, what `429` means, and the reset. Add the non-obvious one —
  **a cached response still counts against the quota**, because throttling and
  quota are evaluated before the gateway cache. Nobody guesses that.
- **Warn against shipping the key in a browser bundle.** The portal audience
  includes frontend developers, and the key is a bearer credential with a quota
  attached; the honest advice is to call the API from their own backend. Saying
  it here costs one paragraph and prevents a class of support conversation.
- **Cover the parameter bounds that already exist** — `limit` is 1..200 and
  returns `400` outside that, published in the spec by [[0124]]. Examples should
  use valid values and the text should name the bound.
- **Examples worth having**: current price for an asset, OHLCV over a window,
  the batch endpoint, and `/health`. Enough to show the shapes; the spec covers
  the rest.
- **Live where the reader is** — served from [[0161]]'s distribution next to the
  portal, and reachable from the dashboard. Whether the source of truth is a
  markdown file in `docs/` rendered at build time or a page in the portal is a
  decision to record; do not maintain two copies.

## Acceptance Criteria

- [ ] Quickstart takes a reader from a fresh key to a successful response with
      one copy-paste
- [ ] Auth (`x-api-key`, `403` without it) and limits (1 req/s, monthly quota,
      `429`, cached responses still counted) all stated
- [ ] Example queries cover current price, OHLCV, batch and health, and every
      one of them was run against production before publishing
- [ ] Guidance not to embed the key in a browser bundle
- [ ] Links to Swagger UI and `/api-docs-json` rather than restating the spec
- [ ] Reachable from the portal dashboard and from the documented URL
- [ ] Single source of truth for the text — no second copy to drift
- [ ] Epic AC 3 satisfied

## Notes

- "Accurate against the live API" is verified in [[0164]], which runs these
  exact commands with a real self-service key as part of the end-to-end check.
  Keeping them mechanically runnable — no placeholders beyond the key itself —
  is what makes that possible.
- The examples double as the burst-limit argument in [[0157]]: if the quickstart
  page fires two of them in parallel, burst 1 would fail our own documentation.
