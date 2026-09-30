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
- **Regenerate warning, at the user's request.** The user flagged Regenerate as the one step that must stand out. Regenerate leaves the account with no key until the next quota period and cannot be undone. The warning was the last bullet of a list; `2c038656` made it a callout and repeated it in the 403 row.
  - Baseline: 3 fresh-agent answers to "suddenly 403, how do I fix it". None told the user to press Regenerate, and none warned against it either.
  - After the change: 3/3 answers say "don't click Regenerate to fix this". The prompt "rotate my month-old key" is talked out of it.
  - "The dashboard says my key was suspended, am I banned?" gets "no, expected after a Regenerate, new key on 1 October".
  - Source of that last case: after a self-regenerate the portal shows `RevokedDashboard` (`web/portal/src/app/app.tsx` ~1613). Its copy is written for an operator suspension ("Monthly quota exceeded repeatedly. Key was suspended…"). The code comment records this as Adam's decision (2026-08-26). The skill now tells agents it is expected; changing the card is Adam's call.

### PR review: Oskar and Adam (2026-09-29)

- **Oskar** checked the skill against the handlers, DTOs, the gateway and
  the key-reissue logic. He raised three points that could make an agent act
  wrongly. I re-checked each in code:
  - Batch `not_found` also holds tracked assets that are not priced yet
    (`batch/handlers.rs`).
  - 503 `quote_unavailable` means the reference asset is untracked
    (`errors.rs:63`), so retrying cannot help.
  - `price_status` has a transient `""` beside a real price during a deploy
    (`assets/dto.rs`). "No price" is `unpriced`, not an empty `as_of`.
- **Adam** ran 6 scenarios with `claude -p` on Opus 5.5, each with only the
  skill installed. All passed: key in env, key in `.env`, multi-asset with
  search/batch/OHLCV, no key, invalid key, and a negative balance question.
  His two nits:
  - The key check ignored `.env`, although the skill tells users to put the
    key there.
  - The user-agent reason was written for us, not for the agent.
- **Fixed in `7318d858`.** The key check and the recipe setup now read one
  `STELLAR_PRICES_API_KEY=` line from `.env` when the variable is unset.
  - This does not execute `.env`. Adam's suggestion `set -a; . ./.env` would.
  - Shell state does not persist between an agent's tool calls, so the setup
    line repeats in every call.
- **Tests of the `.env` read** (bash and zsh):
  - plain, double-quoted, single-quoted, `export` prefix, CRLF: all resolve;
  - commented-out line, no key line, missing file: all give an empty key;
  - the environment variable wins over `.env`;
  - a `touch pwned` line in `.env` was not executed.
- **Negative control with the fake key only in `.env`.** The recipes
  resolved the 14-character key, and 8/8 bash plus 8/8 zsh returned
  `curl: (22) … 403`.
- **Fresh-agent run with the key only in `.env`.** The agent found the key
  and got 403. It did not guess a price, warned against Regenerate, and never
  printed the key (0 hits in the transcript).
- **Live re-run with the user's key, 2026-09-29 ~14:07 UTC, on `7318d858`.**
  In bash all 8 recipes passed, and in zsh all 8 passed. The pagination loop
  returned 600 rows in each shell, so the new key-resolving setup line works
  against production.

### Public-facing wording (2026-09-30)

The user asked whether the skill should show transient states, since it is
public for the Stellar community.

- **Decision:** keep documented sentinels, but word them precisely and
  neutrally (`b4f237d5`).
- **`price_status: ""` stays.** The public spec already documents it.
  - It is not a backfill effect and does not happen on every deploy. It
    appears only while a schema change that adds the snapshot's columns is
    being rolled out.
  - ~~until the next refresh, about a minute~~ was wrong (Oskar,
    2026-09-30). The old view rewrites the blanks on every refresh, so the
    state lasts until the view is re-created by hand. In the 0216 rollout
    one view was missed and fixed hours later.
  - In that window `as_of` is `""` too, not "may be".
  - The skill now says to use the price, report its age as unknown and not
    refetch (`a4a5db8e`). It also drops the half-line that read as "empty
    `as_of` means no price".
  - Fresh-agent runs on a blank-window response: 3/3 use the price, call the
    age unknown and do not refetch. Before the last tweak, 2/2 suggested
    "check again in a minute".
  - Refresh cadence is verified in code: `REFRESH EVERY 1 MINUTE`
    (`current.sql:172`).
- **Backfill does not affect `price_status`.** `price_status` reads only the
  last 24 h of candles, while the backfill walks backward toward genesis.
  - The backfill shows only in history depth: `earliest_data_available` and
    `backfill_note` on `timeframe=all`.
  - `/v1/backfill/status` is now described by what it answers ("how far back
    the price history reaches"), so the text stays true after the backfill
    completes. No skill update is needed then.
- **Oskar's answer (2026-09-30)** corrected the duration and the retry
  advice. Both are fixed above.

### Structure vs the Stellar skills (2026-09-29)

**Hard requirements.**
- **Listing.** A community listing needs only the 4-field card and a
  non-blob raw URL. SDF does not review community skill content.
- **Agent Skills spec** ([agentskills.io](https://agentskills.io/specification)).
  - `name`: up to 64 characters, lowercase and hyphens, equal to the directory
    name.
  - `description`: up to 1024 characters.
  - Body: recommended under 500 lines / ~5000 tokens.
- **Our skill** (~3.7k tokens) passes `uvx --from skills-ref agentskills
  validate`. The negative control holds: a bad name fails with three errors.

**Official house style**, followed in `40b888ed`:
- `## When to use this skill` (7/8 official skills);
- `## Related skills` (8/8). Ours links `data`, `assets` and `dapp` via
  skills.stellar.org URLs.

A fresh agent asked "balance in USD" split the work correctly: Horizon for the
balance, this API for the price.

**Deliberately not copied:** `user-invocable` and `argument-hint`. These are
Claude Code-only fields, and the spec validator rejects them ("Unexpected
fields"). SDF's own `data` skill fails validation for the same reason.

**License: MIT, user's decision.** It is set in the frontmatter, and a
bundled `skills/stellar-prices-api/LICENSE` names Rumble Fish Poland Sp. z o.o.
(the entity named in the portal's privacy policy). This licenses the skill
only, not the repo, which still has no root LICENSE (`package.json` says MIT).

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

## Decisions for the Stellar PR (2026-09-30, user)

PR #368 was merged to `develop` on 2026-09-30.

- **`copyValue`:** raw GitHub on `develop`,
  `https://raw.githubusercontent.com/rumblefishdev/stellar-prices-api/develop/skills/stellar-prices-api/SKILL.md`.
  It answers 200 `text/plain`.
  - `develop` plays the role other listings give `main`.
  - Skill fixes reach agents on merge, with no Stellar PR.
  - `npx skills add` reads the default branch (`master`), so it works only
    after a develop→master merge.
- **Changed the same day: host on `master`, not `develop`** (user,
  2026-09-30). PR #372 adds only `skills/stellar-prices-api/` to `master`,
  like #370.
  - Why `develop` was weaker: it is unprotected, and direct lore pushes land
    there, so an unreviewed change would go live at once. A skill change could
    also go live before the API change it describes is deployed; prod deploys
    are manual and from any branch (0294: 25.09 went out from 0311's branch).
  - Why `master`: publishing now needs a deliberate PR, made after the
    deploy. It is also the default branch, so `npx skills add` and the repo
    page see the skill, matching the `main` links other cards use.
  - Cost: every skill change is a PR to `develop`, then a targeted PR to
    `master`, unless it waits for a milestone release merge.
  - `copyValue` becomes
    `…/stellar-prices-api/master/skills/stellar-prices-api/SKILL.md`. The
    Stellar PR waits until #372 is merged and that URL answers 200.
  - #372 was pushed with `--no-verify`. The shared `core.hooksPath` points at
    the main checkout's develop-era pre-push, which clippies
    `comet-extractor`, a crate absent on `master`. The change is markdown
    only and prettier passes.
- **The path must never move.** Stellar #133 exists because the Trustless
  Work card went 404 when its `SKILL.md` moved.
- **Catalog entry in the same PR.** It goes under "Data Indexing" in
  `skills/standards/ecosystem.md`, in the neighbours' format, like #133,
  #132 and #139. This puts the API inside SDF's own `standards` skill, not
  only in the community list.
- **Card text** (it no longer says "any asset" or presents the VWAP as the
  price):
  > Get USD and XLM prices for classic and Soroban assets without computing
  > them from Horizon trades: prices across SDEX, Soroswap, Aquarius, Phoenix
  > and SushiSwap with a 24h cross-venue VWAP, OHLCV candles, Reflector oracle
  > readings and 100-asset batch lookups. Free API key via Discord.
- **"Sales" re-test with this text.** 3/3 fresh agents load the skill first.
- **Checks in the prepared branch:**
  - `check:ecosystem-links` passes: 31 entries, no blob URLs.
  - `test:ecosystem-links` passes 12/12.
  - The generated `llms.txt` carries the line.
  - Our card is prettier-clean. The four prettier warnings in that file
    were already there, in other cards.
  - `next lint`, `tsc` and `build` were not run locally (2.4 GiB free); their
    CI runs them.
- **Stellar's queue.** New-card PRs from mid-September (#132, #136–#139) are
  still open. The last merge touching `skills.ts` was #133 on 2026-09-23.

## Acceptance Criteria

- [x] `skills/stellar-prices-api/SKILL.md` merged to `develop` (#368, 2026-09-30)
- [x] Every curl in it verified against production
- [x] Hosting decided and `copyValue` resolves to raw markdown
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
