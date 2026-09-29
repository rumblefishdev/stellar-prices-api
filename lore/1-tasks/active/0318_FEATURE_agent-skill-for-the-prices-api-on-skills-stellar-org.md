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

**Current state:** skill in PR #368 (to `develop`), audited and verified live
in bash and zsh. Open: hosting decision, then the Stellar PR.

## Verification log (2026-09-29)

These checks are **proven**:

- **Keyless calls against production.** `/health` and `/api-docs-json` answer 200. `/v1/assets/native/price` without a key answers 403 `Forbidden`. `Authorization: Bearer` also answers 403. The portal link `https://sorobanscan.rumblefish.dev/api/?ref=stellar-skill` answers 200.
- **Skill matches the live spec.** All 9 paths in the live spec appear in the skill. Every enum (granularity, timeframe, sort, type, base_currency, order) and every query parameter name matches.
- **Oracle list.** The only oracle in code is `reflector`. "Band" in the first draft of the card was a false grep hit (the word "band" in `change_7d_pct`), so it was removed. `sources` also lists `sushiswap`.
- **Fresh-agent evals.** Runs used `claude -p --model sonnet` from a scratch directory outside the repo, so no repo context leaked in.
  - "Sales" test, control: the unmodified `skills.stellar.org/llms.txt` with the question "USD price of a Soroban token + 7-day chart". 0/2 runs used a prices API. One went to Horizon/RPC with its own candle indexer; the other went to the Soroswap SDK plus self-sampling.
  - "Sales" test, with our card line appended: 2/2 runs chose the Stellar Prices API skill first.
  - No key: the first run was inconclusive, because the eval's sandbox blocked `$VAR` expansion before the agent's behaviour showed. After adding an explicit key-check command to the skill, the rerun with Bash allowed went 2/2. Each run's only command was the check; no API call was made. Each answer gave the portal link, the Discord requirements and the export step.
  - Candles: `timeframe=30d&granularity=1d` for "daily USDC candles, 30 days".
- **Negative control for the live runbook script.** With a fake key, all 8 recipes FAIL and the pagination loop returns 0 rows. The script checks curl `--fail-with-body` with `pipefail`, so `jq` reshaping an error body cannot hide a failure.

- **Live run with the user's key, 2026-09-29 ~10:27 UTC.** All 8 recipes returned 200. They ran verbatim from SKILL.md, key read without echo. Results:
  - XLM: `price_status: carried`, `as_of` 11 minutes behind `updated_at`. This is the ordinary state the skill describes.
  - USDC: `method: oracle`, empty `sources`.
  - `AQUA` search resolves the issuer.
  - Soroban top-10 returns SolvBTC via aquarius.
  - The 7-day hourly and date-range daily (XLM-based) OHLCV both return candles.
  - Batch: `not_found: []`.
  - Oracles: `reflector` only.
  - The pagination loop, capped at 3 pages, returned 600 rows, so the `--data-urlencode` cursor round-trips.

### Second pass: claim-by-claim audit (2026-09-29)

At the user's request, an independent agent checked every claim in SKILL.md
against the live spec, the handlers, the gateway stack and the portal. I re-checked
the key findings myself (FAQ line 102, `auth/mod.rs`, a zsh repro, the gateway
responses, and the ohlcv spec for `XLM` and 503). All of them were fixed in
`ec72a313`:

- **Key flow was wrong.**
  - The first sign-in issues the key automatically; the dashboard then offers **Copy key**. "Get my API key" appears only on a revoked dashboard.
  - **Regenerate** deactivates the key within about 30 s and issues nothing until the next quota period. The skill had said "replacing it".
- **429 has two bodies.**
  - `Too Many Requests` is the 1 req/s throttle.
  - `Limit Exceeded` is AWS's default body for a spent monthly quota; only THROTTLED is customised. Retrying the second one does not help.
- **The pagination loop broke silently in zsh.** `${cursor:+...}` is not word-split there, so the loop stopped after page 1. The earlier 600-row result ran under bash only.
  - The loop now builds an args array.
  - Recipes use `-sS --fail-with-body`, so an HTTP error shows as `curl: (22) … 403` instead of `jq` printing nulls.
- **Imprecise response details, corrected:**
  - `"0"` sentinels differ per field. `vwap_24h` is `"0"` for USDC even when it is priced.
  - Candle price fields can be `null`.
  - `base_currency=XLM` returns only the asset's trades against XLM, not a conversion.
  - The 5000-candle 400 applies only with an explicit `granularity`.
  - `start`/`end` semantics.
  - 500 `db_error` and 503 `quote_unavailable`.
  - Per-route cache TTLs: batch is not cached.
  - Field names on `/assets/{id}` and `/backfill/status`.
  - `/oracles` carries no traded price.
- **Horizon/RPC section.** It now states what each covers, and reads as complementary because SDF reviews it. Both facts were checked in the Stellar docs:
  - `/trade_aggregations` is per asset pair, from classic trades.
  - RPC's default retention is 120960 ledgers, about 7 days.
- **Negative control.** The runbook script now runs every recipe in both bash and zsh. With a fake key, all recipes and both loops FAIL, each with a visible `curl: (22) … 403`.
- **Live re-run with the user's key, 2026-09-29 ~12:37 UTC, on the audited recipes (`62430cc9`).** In bash all 8 recipes passed, and in zsh all 8 passed. The pagination loop returned 600 rows from 3 pages in each shell, so the zsh cursor fix holds against production.

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
   VWAP aggregated across SDEX, Soroswap, Aquarius, Phoenix and SushiSwap, USD ready-made,
   OHLCV history, Reflector oracle readings, 100-asset batch.
3. The funnel (agent → human → portal → Discord → key) must be smooth, and the
   result measurable: portal link carries `?utm_source=stellar-skill`
   (CloudFront logs record the query string, proven with a `ref=` test hit at
   2026-09-29 10:07:22 UTC; GA from 0316 reads `utm_source` with no setup), curl recipes send `User-Agent: stellar-prices-skill/1` (X-Ray trace
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
(`as_of`, `price_status`), errors and per-key limits.

**Usage plans stay out of the skill** (user, 2026-09-29): they are not public.
The skill states only what the portal FAQ publishes per key: 100k requests a
month, 1 request/second, reset on the 1st, and "get in touch" for more.

Do not copy stale text: `packages/prices-api/README.md` (`/production/v1`, 15 s
cache).

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
  description: "Get USD and XLM prices for any classic or Soroban asset without computing them from Horizon trades: VWAP aggregated across SDEX, Soroswap, Aquarius, Phoenix and SushiSwap, OHLCV candles, Reflector oracle readings and 100-asset batch lookups. Free API key via Discord.",
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

- Portal "Skills" section: split out to backlog task 0320 (2026-09-29).
- Found in passing: `web/portal/src/landing/Faq.tsx:81` says prices "come
  straight from Soroswap liquidity pools and are updated on every block". The
  product uses five venues and an hourly USD pass. `Features.tsx:47` repeats
  it. Filed as backlog task 0321.
- Found in passing: prices-api README uses `/production/v1`. (The portal FAQ's
  "free is the only plan" was listed here as stale; it is consistent with plans
  not being public, so it is not.)
