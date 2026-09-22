---
id: "0163"
title: "Quickstart guide and example queries, accurate against the live API"
type: DOCS
status: completed
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
      account-age rules, and — until [[0191]] (which absorbed 0192) ships — the honest answer to "my
      key leaked" is "stop using it and wait for the period to roll over", which
      belongs in the document rather than in a support conversation.
  - date: "2026-09-22"
    status: active
    who: stkrolikiewicz
    note: >
      Activated, with [[0233]]. Re-pointed to the world after 0194/0195: there is
      ONE base URL, the API's own hostname
      `https://prices-api.sorobanscan.rumblefish.dev` — the CloudFront domain this
      task named is gone (0184's distribution destroyed by 0195; the execute-api
      origin answers 403 since 0126). The spec's `servers` block and
      `PUBLIC_API_BASE_URL` already agree, asserted by `links.spec.ts`. Single
      source of truth: the portal's quick start page (0193), served at
      `/api/quick-start`; no markdown copy in `docs/`. Still owed here: the
      keyless exceptions named (`/health`, `/api-docs-json`), burst 5 and "a
      cached response still counts" in the limits block, the
      key-in-a-browser-bundle warning, copyable examples for price, OHLCV, batch
      and health run against production, and 0157's burst criterion.
  - date: "2026-09-22"
    status: active
    who: stkrolikiewicz
    note: >
      Implemented on `docs/0163_quickstart-accurate-against-live-api` with
      [[0233]]: "Example queries" (price by CODE:ISSUER, OHLCV 7d/1h, POST
      batch, GET /health) on the quick start, the keyless exceptions, the
      key-in-a-bundle warning, burst 5 and the measured cache behaviour in the
      limits block, the account-age half-sentence, the dashboard link's stale
      comment. One deviation from this task's text recorded under Emerged:
      the page does NOT say a cached response counts against the quota. Open:
      the production run with a free-plan key, and the two criteria that wait
      on it.
  - date: "2026-09-22"
    status: completed
    who: stkrolikiewicz
    note: >
      Shipped: PR #334 merged 13:35 UTC (`bfb7c18d`), portal bundle synced
      14:02 UTC (`index-DnFlUQMz.js`, `/api/*` invalidation completed). All
      11 criteria met; 4/4 example queries answered on production at 13:02
      CEST, after 0286's schema step. `QuickStart.live` with a free-plan key
      is the check to repeat after every API deploy. [[0164]] repeats the
      same commands as its end-to-end proof.
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

- [x] Quickstart takes a reader from a fresh key to a successful response with
      one copy-paste
- [x] How to get a key is stated before the first `curl`, including Stellar
      Discord membership, with the invite pointing at **`discord.gg/stellardev`**
      and not the `stellar_test` guild
- [x] Auth (`x-api-key`, `403` without it) and limits (1 req/s, monthly quota,
      `429`, cached responses still counted) all stated, **with `/health` and
      `/api-docs-json` named as the keyless exceptions**
- [x] Example queries cover current price, OHLCV, batch and health, and every
      one of them was run against production before publishing
- [x] Guidance not to embed the key in a browser bundle
- [x] Links to Swagger UI and `/api-docs-json` rather than restating the spec
- [x] One documented base URL, and the OpenAPI `servers` block agrees with it —
      Swagger UI's "Try it out" hits the same origin the quickstart teaches
- [x] Reachable from the portal dashboard and from the documented URL
- [x] Single source of truth for the text — no second copy to drift
- [x] Epic AC 3 satisfied (measured 2026-09-22 13:02 with a free-plan key; [[0164]] repeats it end to end)
- [x] **The example queries run without hitting the burst limit** — inherited
      from [[0157]] on 2026-08-13, when that task archived. It was 0157's last
      open criterion and 0157 could never close it: the limits were deployed and
      measured, but there was no quickstart to run. Measured groundwork it hands
      over: burst is 5, and two examples on two different routes are both cache
      misses, so the headroom is real. What is untested is a page that fires
      more than a couple in parallel, or several against the *same* route — a
      cache hit there is not rejected at any rate we could produce, so the
      sequence matters more than the count

## Notes

- "Accurate against the live API" is verified in [[0164]], which runs these
  exact commands with a real self-service key as part of the end-to-end check.
  Keeping them mechanically runnable — no placeholders beyond the key itself —
  is what makes that possible.
- The examples double as the burst-limit argument in [[0157]]: if the quickstart
  page fires two of them in parallel, burst 1 would fail our own documentation.

## Implementation Notes (2026-09-22)

Branch `docs/0163_quickstart-accurate-against-live-api`, shared with [[0233]].
The single source of truth is the portal's quick start page
(`web/portal/src/quickstart/QuickStart.tsx`, route `/api/quick-start`, task
0193); nothing in `docs/` duplicates it.

- **Example queries** — a new section after Endpoints: `EXAMPLES` holds four
  curl snippets (a credit asset's price by CODE:ISSUER with the full USDC
  issuer, `/ohlcv?timeframe=7d&granularity=1h` for native, `POST /prices/batch`
  with two identifiers, `GET /health` with no key). Listed in `SNIPPET_TABLES`
  (the view/text tie) and in `EXAMPLE_PATHS` (each snippet's URL held against
  its template, and the template against the spec). The lede says to run them
  one after another inside a free key's burst of five.
- **Authentication** names `/health` and `/api-docs-json` as the keyless
  routes and gains a "Keep the key on your side" card (bearer credential with
  a quota; call from your own backend).
- **Prerequisites**: "A Discord account created moments ago is turned away
  for a short while" — no number, so ADR 0010's SSM value can move without
  making the page false.
- **Rate limits**: "up to 5 at once" in the rate figure (`FREE_PLAN_BURST`, a
  literal — the config probe reports the per-second rate only) and a paragraph
  on the token bucket and the cache (see Emerged 1).
- **`QuickStart.live.spec.tsx`**: imports `EXAMPLES`, parses each snippet's
  copy text as the curl it is and sends it, with `PRICES_API_KEY` in place of
  the placeholder; holds the 200 body to the fields the spec marks required
  (read from `public/openapi.json`, so neither the requests nor the field
  lists are a second copy). Skipped without the key, so CI never runs it.
  [[0164]] repeats the same commands with a self-service key.
- The 500 row's advice no longer names a status page: none exists ([[0301]]
  looked for one), and the sentence was a promise the page could not keep.
- **`app.tsx`**: the "View quick start" comment claimed `QUICKSTART` still
  pointed at the OpenAPI document; it has been the route since 0193.
- Tests: `app.spec.tsx` rail count 10 → 11; `QuickStart.spec.tsx` +12 cases
  (documented paths, example URLs). Portal: 231 tests, typecheck, lint green.

## Design Decisions

### Emerged

1. **The page does not say a cached response counts against the quota.**
   This task's Implementation section asserts it ("throttling and quota are
   evaluated before the gateway cache"). Task 0157's production measurement
   (2026-08-13) found the opposite ordering — the stage cache is in front of
   the throttle, a cache hit is never rejected — and could not settle whether a
   hit spends a bucket token or a quota unit. The page states only what was
   measured; that bullet of this task is superseded by this note.
2. **Four examples, not one per endpoint.** Price, OHLCV, batch and health are
   what the task names; the Endpoints section already unfolds an example
   response per route, and a curl per route would be the reference again.
3. **Burst as a literal, not a probe field.** Adding `burst` to the portal's
   config endpoint is backend work for one number that has not moved since
   0157; a comment names its source in `infra/envs/production.json`.
4. **The production run is a test gated on the key, not a CI job**: a
   free-plan key belongs to a person, and a key in a workflow or a transcript
   is the incident 0298 just cleaned up after. A first cut was a standalone
   script with the four requests written out by hand — a second copy of the
   page that nothing compared; the test runs the copy text itself instead.

**Production run, 2026-09-22 12:16 CEST (the key holder, free-plan key):
3 of 4.** `price`, `batch` and `health` answered 200 with every required
field. `ohlcv` (`?timeframe=7d&granularity=1h`) answered **500** — the
api-handler logged ClickHouse `Code: 47, Unknown expression identifier
pf_trade_count` in the `price_ohlcv_1h` read. The Lambda deployed on
2026-09-18 08:11 UTC carries [[0286]]'s phase-1 read path (merged 09-16,
`8ddc2fa9`) while the CH schema step of that rollout has not run, so the
whole `/ohlcv` route has answered 500 since that deploy. It went unseen:
API Gateway counted 0 server errors on 09-19…09-21 and 1 on 09-22 (this
one) — nobody called the route — and the api-handler had no error alarm
until [[0249]]'s stack went out today. The page is right; production is
not. Not recorded on 0286: its owner started the phase-1 rollout the same
morning, and the schema step went out before this note could.

**Re-run 13:02 CEST, after 0286's schema step (21 `pf_*` columns on the
seven `price_ohlcv_*` tables): 4 of 4.** `ohlcv` answers 200 with every
required field. The route had been down from the 09-18 deploy to that step.

Every criterion is met; what remains is the PR and the portal bundle going
out. `PRICES_API_KEY=… npx vitest run QuickStart.live` in `web/portal` is the
check to repeat after any API deploy.
