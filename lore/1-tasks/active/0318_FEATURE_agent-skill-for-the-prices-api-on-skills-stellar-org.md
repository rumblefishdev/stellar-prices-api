---
id: "0318"
title: "Agent skill for the prices API, listed on skills.stellar.org"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0315", "0316"]
tags: [docs, marketing, agents, epic-self-service-onboarding, priority-medium, effort-small]
links:
  - "https://skills.stellar.org"
  - "https://github.com/stellar/stellar-dev-skill/blob/main/site/src/data/skills.ts"
  - "../../../skills/stellar-prices-api/SKILL.md"
history:
  - date: "2026-09-29"
    status: active
    who: stkrolikiewicz
    note: "Task created. Portal skills section deferred; hosting of the published SKILL.md undecided."
---

# Agent skill for the prices API, listed on skills.stellar.org

## Summary

Write one `SKILL.md` that teaches an AI agent to call the prices API, and list it
in the "Community skills" section of https://skills.stellar.org.

## Status: Active

**Current state:** skill being written in `skills/stellar-prices-api/SKILL.md`.

## Context

**Why: this is marketing aimed at AI agents, not at people.** An agent building
on Stellar reads `skills.stellar.org/llms.txt`; the card description and the
skill's frontmatter `description` are what it sees when it decides whether to
use us. Three consequences:

1. The descriptions are the ad copy. They carry the words a "give me the price
   of…" prompt contains: price, USD, XLM, OHLCV/candles, Soroban tokens, SDEX,
   AMM, oracle.
2. The skill has to beat "I'll compute it from Horizon". SDF's own `data` skill
   points agents at raw RPC/Horizon trades. Ours says when to use us instead:
   VWAP aggregated across SDEX, Soroswap, Aquarius and Phoenix, USD ready-made,
   OHLCV history, Reflector/Band oracle comparison, 100-asset batch.
3. The funnel (agent → human → portal → Discord → key) must be smooth, and the
   result measurable: portal link carries `?ref=stellar-skill` (CloudFront
   logs), curl recipes send `User-Agent: stellar-prices-skill/1` (X-Ray trace
   summaries), business metric is new `discord-*-key` keys before vs after.

How Stellar accepts community skills: a PR to `stellar/stellar-dev-skill` adding
an entry `{title, description, pathLabel, copyValue}` to `ECOSYSTEM_CARDS` in
`site/src/data/skills.ts`. `copyValue` is the raw markdown URL an agent
fetches; CI (`check:ecosystem-links`) only rejects `github.com/.../blob/` URLs.

## Implementation Plan

### Step 1: `skills/stellar-prices-api/SKILL.md`

One file (the "use without installing" mode fetches a single URL). The
`skills/<name>/SKILL.md` path is what `npx skills add owner/repo` discovers, and
it works for every hosting option below. Content: key onboarding first, when to
use us over Horizon, base URL + `x-api-key`, endpoint table with the live spec
as source of truth, asset identifiers, curl runbook, reading the response
(`as_of`, `price_status`), errors and plan limits.

Do not copy stale text: `web/portal/src/landing/Faq.tsx:69-76` (says free is the
only plan), `packages/prices-api/README.md` (`/production/v1`, 15 s cache).

### Step 2: Verify

- Every curl in the skill against production (user's key): 200 and the fields
  described; the same call without a key gives 403.
- Endpoint table vs `jq -r '.paths|keys[]' web/portal/public/openapi.json`.
- Fresh-agent runs: no key → asks for one and links the portal; with key →
  `/v1/assets/native/price` with `as_of`; "daily USDC candles, 30 days" →
  `timeframe=30d&granularity=1d`; card-description "sales" test against
  `llms.txt`.

### Step 3: PR to `develop`

### Step 4: PR to `stellar/stellar-dev-skill` (after the hosting decision)

Draft entry:

```ts
{
  title: "Stellar Prices API",
  description: "Get USD and XLM prices for any classic or Soroban asset without computing them from Horizon trades: VWAP aggregated across SDEX, Soroswap, Aquarius and Phoenix, OHLCV candles, Reflector/Band oracle comparison and 100-asset batch lookups. Free API key via Discord.",
  pathLabel: "rumblefishdev/stellar-prices-api",
  copyValue: "<depends on hosting>",
}
```

Optional second placement: a mention in SDF's `data` or `standards` skill
(separate PR; SDF may decline).

## Open questions

- **Where `copyValue` points.** Raw GitHub on `master` (needs a develop→master
  merge first; `npx skills add` works), raw GitHub on `develop` (live on merge),
  or a static file on the portal (`web/portal/public`, manual portal deploy).
  Only the portal option gives a fetch count, from CloudFront logs.

## Acceptance Criteria

- [ ] `skills/stellar-prices-api/SKILL.md` merged to `develop`
- [ ] Every curl in it verified against production
- [ ] Hosting decided and `copyValue` resolves to raw markdown
- [ ] Entry merged into `stellar/stellar-dev-skill` and visible on skills.stellar.org

## Notes

- Portal "Skills" section: deferred by the user (2026-09-29).
- Found in passing: portal FAQ still says free is the only plan; prices-api
  README uses `/production/v1`.
