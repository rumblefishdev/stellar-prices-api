---
id: "0163"
title: "Quickstart guide and example queries, accurate against the live API"
type: DOCS
status: backlog
related_adr: ["0010"]
related_tasks: ["0124", "0156", "0157", "0161", "0162", "0164", "0179", "0184", "0187", "0189", "0192", "0193", "0195"]
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
  - date: 2026-08-07
    status: backlog
    who: akot
    note: >
      Corrected the auth rule: the epic's blanket "every request requires a key"
      is already untrue of the deployed API, and this task both stated the rule
      and used a keyless route as an example. Also pinned to [[0161]]'s
      `/api-tokens/` prefix.
  - date: 2026-08-10
    status: backlog
    who: akot
    note: >
      ADR 0010 changes step one: getting a key now requires Stellar Discord
      membership and a minimum account age. The quickstart must say so before
      the first `curl`, without hard-coding the threshold, which is an SSM
      value expected to be tuned. The invite it prints is always the real
      server, never the `stellar_test` guild used during development.
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Re-pointed after the epic was re-sliced into vertical increments. The
      hand-over this document waits on is now [[0187]] (a key on screen), not
      [[0162]]; its hosting is [[0184]] and its Swagger UI [[0195]]. Two
      additions to step one: the prerequisites are [[0189]]'s membership and
      account-age rules, and — until [[0192]] ships — the honest answer to "my
      key leaked" is "stop using it and wait for the period to roll over", which
      belongs in the document rather than in a support conversation.
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
- **State how to get a key, including its two prerequisites — added 2026-08-10
  by ADR 0010.** Getting a key is no longer "sign in with Discord": the account
  must also be a **member of the Stellar Developers Discord**
  (`discord.gg/stellardev`) and **older than a minimum age**. A quickstart whose
  very first step silently fails for a reader is the exact inaccuracy AC 3
  exists to prevent, and this is the most likely way it now happens.
  Keep it to two lines above the `curl` — the reader wants data, not policy —
  and link the portal rather than restating the rules.
  **The age requirement is currently 5 minutes** (ADR 0010, matching Stellar's
  own server setting) — small enough that it deserves at most a half-sentence,
  or nothing at all. Do not write "at least N days old": that is wrong, and it
  would make the API sound far more gated than it is. If it is mentioned, word
  it so a later change to the SSM value does not make this page false.
  **The invite link in this document is always `discord.gg/stellardev` — the
  real Stellar Developers server — never the `stellar_test` guild**, even while
  development runs against the test guild ([[0179]]). This page is read by
  outside developers, so a test-guild link here would send them somewhere they
  cannot join and would ship a private artefact into public documentation. This
  page can therefore be written before the SSM flip, but it describes the world
  after it.
- **There are now two working base URLs — document exactly one.** [[0161]] puts
  CloudFront in front of the API, so `/v1/*` answers both on the distribution's
  domain and on the raw API Gateway invoke URL. The documented one is the
  **CloudFront domain**: it is the address in the Tranche 3 submission, it shares
  a hostname with the portal and the docs, and it keeps the `/production` stage
  segment — an implementation detail — out of partners' code.

  This has to reach the spec, not just this page. [[0124]] stamps `API_BASE_URL`
  into the OpenAPI `servers` block, which is the URL Swagger UI's "Try it out"
  calls. If that still points at the invoke URL while the quickstart teaches the
  CloudFront domain, our own documentation demonstrates a different origin from
  the one we publish — and it fails silently, because both work.
- **State the auth rule and its failure explicitly**: data routes need a key; a
  missing key is `403`. It is the first error anyone hits.
  **Name the exceptions, because the epic does not.** The epic says "every API
  request requires a key", which is already untrue of the deployed API:
  `/health` is a keyless mock and `/api-docs-json` is deliberately anonymous
  ([[0124]]). Both appear in the examples below, so a quickstart that repeats
  the epic verbatim would contradict its own commands — exactly the kind of
  inaccuracy AC 3 exists to prevent.
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
- [ ] How to get a key is stated before the first `curl`, including Stellar
      Discord membership, with the invite pointing at **`discord.gg/stellardev`**
      and not the `stellar_test` guild
- [ ] Auth (`x-api-key`, `403` without it) and limits (1 req/s, monthly quota,
      `429`, cached responses still counted) all stated, **with `/health` and
      `/api-docs-json` named as the keyless exceptions**
- [ ] Example queries cover current price, OHLCV, batch and health, and every
      one of them was run against production before publishing
- [ ] Guidance not to embed the key in a browser bundle
- [ ] Links to Swagger UI and `/api-docs-json` rather than restating the spec
- [ ] One documented base URL, and the OpenAPI `servers` block agrees with it —
      Swagger UI's "Try it out" hits the same origin the quickstart teaches
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
