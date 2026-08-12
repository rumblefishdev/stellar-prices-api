---
id: "0157"
title: "Default key limits: 1 req/s + monthly quota, not the design doc's 100 req/s"
type: FEATURE
status: active
related_adr: ["0008", "0010"]
related_tasks: ["0121", "0156", "0158", "0160", "0163", "0180"]
tags: [layer-infra, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, api-gateway, usage-plan, throttling, cost]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
  - "../../../infra/envs/production.json"
history:
  - date: 2026-08-06
    status: backlog
    who: akot
    note: >
      Epic AC 5, verbatim as the task title. Independently shippable ahead of
      the portal — the limits must exist before any key can be self-issued
      against them. Quota fixed at 100 000/month (our choice; the epic's
      "50,000–100,000" is an e.g., not a bound) and
      burst at 5; both confirmed by Adam on 2026-08-06.
  - date: 2026-08-07
    status: backlog
    who: akot
    note: >
      Added the calendar-alignment of the monthly quota (it settles [[0160]]'s
      period-boundary question), and corrected the [[0121]] sequencing note —
      the partner key's daily quota kills a sustained 100 rps run after ~100
      seconds, which the original note did not catch.
  - date: 2026-08-07
    status: backlog
    who: akot
    note: >
      Config naming settled: `<plan>Plan*` for usage-plan settings, `apiGateway*`
      for gateway/stage settings. Three renames alongside the three new fields,
      done here because this is the task that introduces the second tier and
      makes `apiKeyRateLimit` actively misleading.
  - date: 2026-08-10
    status: active
    who: akot
    note: >
      Activated. Branched from `docs/0156_self-service-auth-assumptions` rather
      than `develop` because [[0156]] (PR #187, in review) rewrites this task's
      quota-reset section: the calendar alignment is unverified, so the
      single-date property is design intent until [[0180]] #7 measures it.
      Building from `develop` would implement the superseded spec. The status
      move is committed on the feature branch, not pushed to `develop`, to
      avoid a modify/delete conflict against PR #187 which edits this file.
  - date: 2026-08-10
    status: active
    who: akot
    note: >
      Implemented as ONE usage plan renamed in place, not two — the specified
      two-plan design rested on two claims that did not survive checking:
      `UsagePlanName` is "no interruption" (no replacement risk), and the partner
      key served 14 requests in 30 days, all our own verification curls. The epic
      never asked for a second plan. Full reasoning in Design Decisions #3-#6.
      Changed: `types.ts`, `api-gateway-stack.ts`, `envs/production.json`,
      `infra/README.md` SSM contract, plus a new
      `docs/runbooks/manual-api-key-tier.md`. Three config fields renamed to
      `pricingApiFreePlan*`; six were planned, the one-plan outcome halved it.
      Key rotated in the same change (construct-id change → remove + add, not
      `ApiKey.Name` replacement) after its value was exposed by
      `get-usage-plan-keys` during the usage check.
      Verified by `tsc`, `cdk synth` and `cdk diff` against the live stack: plan
      updates in place under logical id `ApiUsagePlanDBBE8AB1`, key removed and
      re-added under a new logical id, SSM parameter added. Not yet deployed —
      four ACs stay open until then.
  - date: 2026-08-12
    status: completed
    who: akot
    note: >
      Review round on PR #195, eight findings fixed: plan family renamed to
      `pricing-api-free`; the runbook's `items[0]` key lookup replaced with a
      printed candidate table (key names are not unique, and `nameQuery` has no
      documented matching semantics); profile re-export plus a guard on the
      literal string `None`; two stale §6 throttling tables; the load-test
      criteria; the grant-facing milestone evidence; the Makefile rotation
      warning; and a self-contradictory comment. Merged to develop as 2de35d8
      via PR #187 — PR #195 was merged into that branch rather than into develop,
      so 0156 and 0157 landed as one squash commit.
  - date: 2026-08-12
    status: completed
    who: akot
    note: >
      Completed with three acceptance criteria open, noted here so the archive is
      not read as full verification. All three assert behaviour of the deployed
      system and none has been observed: a key throttled at 1 req/s with a
      decrementing monthly quota, the quickstart's parallel queries clearing the
      burst limit (0163 unwritten), and epic AC 5. The code is merged, not
      deployed — nothing has changed in AWS, and the production key rotates on
      the first deploy that carries this. Calendar alignment of the quota reset
      also stays unmeasured; it is design intent here and 0180 #7 owns it.
  - date: 2026-08-12
    status: active
    who: akot
    note: >
      Reopened out of the archive to run the deploy, at Adam's request. The
      entry above archived this with acceptance criteria that close only
      against a deployed system, so the archive was recording merged code as a
      finished task — reopening is the honest place for the deploy and its
      verification to live rather than a follow-up carrying no context of its
      own. Scope of the reopen is the deploy of `Prices-production-ApiGateway`
      and the observation of the open ACs; the key rotation was acknowledged
      and accepted by Adam in advance.
  - date: 2026-08-12
    status: active
    who: akot
    note: >
      Deployed to production, ApiGateway stack only, with an explicit
      `--exclusively` — the Makefile target lacks it and would have pulled
      ComputeStack in behind eleven 10-byte placeholder bootstraps. Epic AC 5
      closes: the plan reads 1 req/s, burst 5, 100 000/month in AWS.
      Throttling and the unchanged `403` are measured; the quota half of its
      AC is not, so that box stays open on the quota alone. The measurements
      also put the stage cache in front of the throttle, contradicting the
      ordering this task had recorded as an inference. Full write-up under
      Deploy. Stays active: the quota reading is still open, and the seven
      `utoipa` 429 descriptions remain undeployed pending a ComputeStack
      deploy with real binaries.
---

# Default key limits: 1 req/s + monthly quota

## Summary

The deployed usage plan gives an API key **100 req/s** (`apiKeyRateLimit`) and a
10 000/day quota — a shape designed for one hand-issued partner key. The epic
overrides that number for self-issued keys: **1 req/s per key (60 req/min) plus
a monthly request quota**, because an unreviewed, no-approval key must not be
able to consume the capacity the whole system is load-tested against.

The epic is explicit that this is a **rate-limit-only change**: it does not
affect the `403`-without-key behaviour or the usage-plan mechanism already
agreed.

**As built, this task goes past that framing — deliberately, and the Summary
should not be read as covering it.** Besides the limits it renames the plan to
`pricing-api-free-production`, renames three config fields, publishes a new SSM
parameter, and **rotates the production API key**. The rotation is not epic
compliance: it is scope this task added after the old key's value was exposed by
`get-usage-plan-keys` during the usage check (Design Decision 5). Its
consequence is the one thing in here that touches someone outside the repo — on
deploy, every holder of `prices-production-partner-key` is cut off. Usage over
the preceding 30 days was 14 requests across 4 days, all our own verification
curls, which is why that is acceptable rather than a blocker.

**One holder that argument cannot see, checked separately** *(2026-08-12, from
review of [[PR 195]])*. `GetUsage` proves nothing about a holder who has not
called in 30 days, and this repo has two documents — `docs/scf/milestone-1-evidence.md`
and `docs/scf/milestone-1-video-scenario.md` — that hand an SCF reviewer a curl
with `x-api-key` and tell them to run it. If the key's value had ever gone out
with them, the reviewer would be exactly such a holder and the rotation would
break a grant deliverable on deploy. **It never did** — confirmed by Adam
2026-08-12; the evidence docs carry `$KEY` as a placeholder the reader exports
themselves, never a value. So the blast radius is what the usage figure says it
is. Worth writing down because the question is re-askable from the docs alone,
and the answer is not derivable from them.

## Context

`api-gateway-stack.ts` creates one `UsagePlan` (`prices-<env>-partner-plan`)
with `rateLimit: 100`, `burstLimit: 200` and a 10 000/day quota, one key
(`prices-<env>-partner-key`), under a stage throttle of 200 rps / 400 burst.
That is coherent for a key we hand out deliberately; it is not a default for
keys anybody can mint by signing in with Discord.

The epic's case against 100 req/s: it equals the sustained load the *entire*
system is load-tested against, so one self-issued key can take all of it; ten
such keys saturate the 1000 req/s global burst ceiling; API Gateway bills per
request regardless of cache hits, so a key held at 100 req/s is ~259M
requests/month ≈ $900/month from a single no-approval signup; and comparable
public APIs sit far lower — CoinGecko free registered ≈ 0.5 req/s,
CoinMarketCap free bursting to ~0.83 req/s but really bounded by 15 000
calls/month.

Two of the epic's supporting premises do not map onto this deployment and
should not be repeated verbatim in a review: the `db.t4g.micro` RDS point (we
read from ClickHouse on Hetzner over mTLS), and the $900 figure, which assumes
no quota — the plan as deployed already caps a key at 10 000/day. The
conclusion stands on the remaining arguments.

## Implementation

**From the epic**

- **Default free-tier limit for self-issued keys: 1 req/s per key** (60
  req/min) — roughly 2× CoinGecko's free registered tier.
- **A monthly request quota alongside the per-second throttle.** The epic gives
  "(e.g. 50,000–100,000 calls/month)" — an **illustration, not a range to pick
  from**, so the number below is our decision rather than epic compliance, and
  nothing in the epic sets a floor of 50 000. Take **100 000** — ~3 300/day, ~6.7×
  CoinMarketCap's free tier, and a round number the dashboard ([[0160]]) can
  render as "used / 100 000 this month" without explanation. A per-second limit
  alone does not stop a key idling at 1 req/s from producing 2.6M
  requests/month.
- **Anything higher is manual and out of band** — someone creates a key with a
  higher quota by hand (AWS console/CLI), payment happens by bank transfer
  outside the product. No self-serve upgrade flow, no in-app billing.
- **No change** to `403`-without-key or to the usage-plan mechanism.

**Follows from the epic, but not stated in it**

- ~~**Two usage plans on the same stage.**~~ **Superseded 2026-08-10 — see
  Design Decisions below.** The original reasoning was: the epic requires 1 req/s
  for self-issued keys *and* a hand-issued higher tier, a single plan cannot hold
  both, so add `prices-<env>-selfservice-plan` and leave `prices-<env>-partner-plan`
  untouched as the manual tier — because renaming it "risks a CloudFormation
  resource replacement that would invalidate a working partner key".

  **Both halves of that turned out to be wrong.** `UsagePlanName` is
  *Update requires: No interruption* (every property of
  `AWS::ApiGateway::UsagePlan` is), so there is no replacement risk; and the
  partner key served 14 requests in 30 days — all our own verification curls — so
  there was no working partner to invalidate. The epic never asks for a second
  plan either; it calls the manual tier "nothing to build here".

  **Implemented instead: one plan, renamed in place.** A key still belongs to one
  plan per stage, so `(usagePlanId, apiKeyId)` stays unambiguous for `GetUsage`.
- **Burst 5.** The epic fixes the sustained rate and says nothing about burst.
  At burst 1 the quickstart page ([[0163]]) firing two example queries in
  parallel 429s on a key we issued ourselves, which contradicts epic AC 3.
  Token-bucket refill keeps the sustained rate at exactly 1 req/s; burst only
  allows the allowance to be spent unevenly. 1:5 is a wider ratio than the
  1:2 the plan carried before (100:200) and than the stage throttle's 200:400 —
  deliberately so, because at a sustained rate of 1 a 1:2 bucket refills too
  slowly to absorb even a pair of parallel requests.
- **Config naming — settled 2026-08-07. Scope first, then property:**
  `apiGateway*` for anything belonging to the gateway or the stage,
  `<plan>Plan*` for anything belonging to a specific usage plan.

  | Today | After |
  | --- | --- |
  | `apiGatewayThrottleRate` / `apiGatewayThrottleBurst` | unchanged — stage-level, not a plan |
  | `apiGatewayCacheEnabled`, `apiBaseUrl` | unchanged |
  | `apiKeyRateLimit` | `pricingApiFreePlanRateLimit` (1) |
  | `apiKeyBurstLimit` | `pricingApiFreePlanBurstLimit` (5) |
  | `apiGatewayPartnerDailyQuota` | `pricingApiFreePlanMonthlyQuota` (100000) |

  Three renames, no additions — the one-plan outcome collapsed the six fields
  this table originally listed into three. The reason to rename here rather than
  later: `apiKeyRateLimit` reads as a global property of API keys, when it is one
  plan's rate.

  **Corrected 2026-08-12, from review of [[PR 195]].** The right-hand column read
  `selfServicePlan*` until Adam noticed it did not match the plan. The rule says
  `<plan>Plan*`, and `<plan>` has to be the plan's **actual name** or the rule
  buys nothing — the fields were named after "self-service", which is how you
  *obtain* a key, while the plan they configure is called `pricing-api-free`. One
  plan therefore carried two names, split down the middle: `pricing-api-free` on
  everything AWS sees (`UsagePlanName`, the SSM parameter and its construct id),
  `selfService*` on everything the repo sees (config fields, key construct, key
  name).

  Worth recording that the rename that *was* reasoned about got a table, a date
  and a rationale, while `pricing-api-free` itself arrived with none of the three
  — no epic mention, no Design Decision, no row here. It is now the settled
  family name by Adam's decision (2026-08-12); "self-service" stays in prose for
  the concept — a key anybody can mint by signing in — and never as a resource or
  field name. Everything the plan owns now reads the same way:

  | | |
  | --- | --- |
  | Usage plan | `pricing-api-free-<env>` (construct `UsagePlan`, logical id unchanged) |
  | SSM parameter | `/prices/<env>/pricing-api-free-plan-id` (construct `PricingApiFreePlanIdParam`) |
  | Config fields | `pricingApiFreePlanRateLimit` / `…BurstLimit` / `…MonthlyQuota` |
  | API key | `pricing-api-free-<env>-key` (construct `PricingApiFreeApiKey`) |

  The key was the one real decision in that list rather than a mechanical
  follow-through, because it is the only entry whose name could reasonably have
  come from its *function* instead of its plan — it is ours, the one verification
  curls use, and after [[0160]] it will be one of many keys on this plan and the
  only one CloudFormation manages or that has no owning row in [[0158]]'s
  registry. Adam chose the family name; that distinction is therefore carried by
  a comment at the construct instead, so nobody later goes looking for its
  registry row. Free to do here either way: this task already destroys and
  recreates that key, so the second rename rides along on a rotation that was
  already paid for.

  The quota period stays in the name (`MonthlyQuota`) on purpose: a usage plan
  carries exactly one quota, so encoding the unit makes it impossible to misread.

- **Renaming a config key is not renaming a resource.** CloudFormation sees only
  the *values*; the key exists solely for our own typed config, so `tsc` flags
  every missed usage and no resource is touched.

  Renaming the *plan* turned out to be equally safe — `UsagePlanName` is
  *No interruption* — so the caution originally recorded here was unnecessary.

  `AWS::ApiGateway::ApiKey.Name` *is* replacement, but **that is not the
  mechanism that rotated the key here** *(corrected 2026-08-11 from the synthesized
  template)*. The construct id changed too (`PartnerApiKey` →
  `PricingApiFreeApiKey`), which changes the logical id — so CloudFormation sees
  `ApiPartnerApiKey8034B29D` removed and `ApiPricingApiFreeApiKey04E49C4B` added,
  two unrelated resources, and `Name`'s replacement semantics never come into
  play. *(The construct id was `SelfServiceApiKey` until 2026-08-12; the naming
  correction below changed which logical id appears on the right-hand side, not
  what CloudFormation does — it is one remove + one add either way, so the
  deploy impact is unchanged.)* The outcome is identical (new value, old key destroyed); the CFN verb is
  remove+add, not Replacement. Both paths delete the old resource only in the
  post-success cleanup phase, so neither opens a window where a valid key is
  rejected.

  **Blast radius — corrected 2026-08-11.** The list below was written about the
  *config-field rename* and then read as the whole change's footprint, and the
  claim "no consumers outside the repo" is what let seven published OpenAPI
  descriptions go unnoticed. The full set actually touched:
  `types.ts`, `api-gateway-stack.ts`, `compute-stack.ts`, `envs/production.json`,
  `infra/README.md`, the new `docs/runbooks/manual-api-key-tier.md`,
  `docs/prices-api-general-overview.md`,
  `docs/database-schema/database-schema-overview.md`,
  `packages/prices-api/loadtest/README.md`, [[0121]], and in Rust:
  seven `utoipa` 429 descriptions (`assets/`, `batch/`, `backfill/`,
  `oracles/handlers.rs`) plus `config.rs` and `auth/mod.rs`. The 429 descriptions
  **are** a consumer outside the repo — they ship in the OpenAPI document served
  at `GET /api-docs-json`.

  Original note, scoped to the config rename: `types.ts`, `api-gateway-stack.ts`,
  `envs/production.json`, plus the SSM contract table in `infra/README.md`.
  `cicd.json` does not carry these fields and there are no consumers outside the
  repo. [[0121]] mentions the old names in prose and should be updated with them;
  archived tasks keep the old names as historical record.

- Validation follows the existing pattern (positive integers, burst ≥ rate).
  The originally-planned cross-check `selfServicePlanRateLimit <=
  partnerPlanRateLimit` does not exist in the one-plan outcome — and would not
  have caught the "100 instead of 1" typo anyway (100 ≤ 100 passes). Replaced by
  a check derived from the epic's own argument; see Design Decisions.
- Quota period `apigateway.Period.MONTH`. Note that **a usage plan carries
  exactly one quota** — no daily sub-cap alongside the monthly one — so a key at
  full rate spends the month's allowance in ~28 hours and then waits for the
  reset. That is only acceptable because [[0160]] caps rework at once per quota
  period; otherwise a user would simply mint a fresh key, which is exactly the
  loophole the epic's rework rule closes.
- **The monthly quota is assumed calendar-aligned — resetting on the 1st at
  00:00 UTC** rather than on a rolling window from key creation. That is what
  makes [[0160]]'s boundary ("first of the month following the last rework") and
  the epic's "calendar month" the same date, so there is one date to render, not
  two.
  **This is unverified, and [[0156]] established that AWS does not document it
  at all** (2026-08-10). The only statement anywhere is an example caption,
  *"creates a usage plan that resets at the beginning of the month"* — no
  timezone, no instant, and nothing on whether `WEEK`/`MONTH` are calendar- or
  creation-aligned. Note also that `offset` is a **request count** (*"The number
  of requests subtracted from the given limit in the initial time period"*), not
  a way to shift the reset day, so it cannot be used to force alignment.
  The instinct already recorded here — "confirm on the first deploy rather than
  assume it" — was the right one and is now tracked as [[0180]] #7. Until it is
  measured, treat the single-date property as a **design intent**, and make sure
  the wording in [[0160]] and [[0163]] presents the rework boundary as our rule
  rather than as AWS behaviour.
- **The quota binds far harder than the rate.** At 1 req/s a key could produce
  ~2.6M requests/month; the quota stops it at 100 000. So the operative limit a
  user meets is the quota, and the per-second throttle is what stops them
  reaching it in an afternoon. Say it that way round in [[0163]].
- Publish the new plan's id as an SSM parameter alongside `ApiGatewayIdParam`.
  The backend that issues keys ([[0160]]) lives in `ComputeStack`, which is a
  *dependency* of `ApiGatewayStack`, so it cannot read the plan object directly
  — the same cycle [[0124]] hit with `apiBaseUrl`.
- Write the manual-tier runbook in `docs/runbooks/`: which plan a hand-made key
  attaches to, and that payment is handled outside the product. Nothing to
  build, but it should be written down once.

## Acceptance Criteria

- [x] Self-service usage plan in CDK: 1 req/s sustained, burst 5, 100 000
      requests/month
- [ ] A key on that plan is throttled at 1 req/s and its monthly quota
      decrements; `403`-without-key behaviour unchanged. **Two of three parts
      met on deploy 2026-08-12** — throttling and `403` are measured (see
      Deploy below); the quota half is not, and `[ ]` stays for it alone.
      `GetUsage` still reported `[0, 100000]` after 80 `200`s, which is most
      likely reporting lag rather than a broken counter, but "most likely" is
      not an observation. Re-read it once the reporting window has passed.
- [ ] The quickstart's example queries run without hitting the burst limit
      *(needs [[0163]], not written yet)*. The deploy measurement supports it
      without closing it: two example queries hit two different routes, so both
      are cache misses, and burst 5 covers them with room. What the measurement
      cannot cover is a quickstart nobody has written.
- [ ] ~~Partner plan and key unchanged~~ — **superseded, not met.** This AC
      assumed the two-plan design. As built the plan is renamed and re-limited in
      place and the key rotates; both deliberate, both verified against the live
      `cdk diff` rather than assumed. Nothing has changed in AWS yet — the
      rotation happens *on deploy*, and at that moment every holder of
      `prices-production-partner-key` is cut off.
- [x] Self-service usage plan id readable by [[0160]] without closing the
      Compute → ApiGateway cycle — published as
      `/prices/{env}/pricing-api-free-plan-id`, recorded in the `infra/README.md`
      SSM contract table
- [x] New config validated at synth like the existing key fields
- [x] Config fields renamed; `apiGateway*` reserved for gateway- and stage-level
      settings. ~~Synth produces the same template as before the rename~~ — moot
      under the one-plan outcome: the template *must* change. What was verified
      instead is that the usage plan's logical id (`ApiUsagePlanDBBE8AB1`) is
      unchanged, so CloudFormation updates the deployed plan in place rather than
      creating a second one.
- [x] Manual higher-tier runbook written — `docs/runbooks/manual-api-key-tier.md`
- [x] Epic AC 5 satisfied: default limits are 1 req/s + monthly quota, not the
      design doc's 100 req/s — **deployed 2026-08-12**; `get-usage-plans`
      reports `pricing-api-free-production` at `rate 1.0 / burst 5 /
      100000 MONTH`
- [x] `docs/scf/milestone-1-evidence.md:795` states 100 req/s as a delivered
      property — reconciled 2026-08-12 with a dated *Superseded* note rather than
      a rewrite. The paragraph records the configuration the milestone was
      evidenced against; editing the number in place would turn a grant-facing
      record of what was true into a statement of what is true now. What the
      paragraph is offered as evidence of — gateway-enforced key auth — is
      untouched by this task.
- [x] The §6 performance tables state the new per-key limit. **Found in review,
      2026-08-12** ([[PR 195]]): the 429 sweep was described as complete, but the
      row `Request throttling (100/s per API key, 1000/s global burst)` survived
      in *two* places — `docs/prices-api-general-overview.md:1096` and
      `docs/database-schema/database-schema-overview.md:1800`. The second is the
      worse of the two: this task had already rewritten that file's "Read rate"
      row to `≤1 req/s per key`, and that row cites **§8.2 as its source** — the
      very table still saying 100/s. The doc contradicted itself in a file the
      task had edited. Both rows now also drop the "1000/s global burst" figure,
      which Design Decision 5b retired.
      The pointer at `prices-api-general-overview.md:164` is removed rather than
      redirected: it named §2.1/§7, but line 164 *is* in §2.1 (so it pointed at
      itself) and §7 states no numeric limit. With §6 corrected there is no
      longer a section for it to point at.

## Deploy — 2026-08-12

Deployed by Adam from `develop`, `Prices-production-ApiGateway` only. CI never
deploys in this repo, so this was a manual `cdk deploy` from a workstation.

**Three things the deploy path itself taught us, none of them about limits.**

1. **The Makefile target for this stack would have taken production down.**
   `make deploy-production-apigateway` runs `cdk deploy Prices-production-ApiGateway`
   with no `--exclusively`, and CDK deploys dependency stacks — it announced
   `Including dependency stacks: Prices-production-Compute`. Every one of the
   eleven bootstraps under `target/lambda/` was the same 10-byte file,
   `#!/bin/sh\n`, so that deploy would have replaced the ledger processor, the
   api handler and every worker with a shell stub. Deployed with an explicit
   `--exclusively` instead. Written up on [[0141]], which owns the footgun but
   was scoped to `deploy-production-compute` and did not cover a *scoped deploy
   of an unrelated stack* as the delivery vector.

2. **[[0124]] rode along, unnoticed until the diff.** The stack also carried the
   `/api-docs-json` route, its two Lambda permissions and a 3600s stage-cache
   entry — merged long ago, never deployed. It is live now and returns `200`.
   The document it serves comes from the *currently deployed* handler, so it has
   no `servers` block (`API_BASE_URL` is in the undeployed ComputeStack) and
   **no `429` responses at all** — the deployed binary predates the sweep
   entirely. Worth stating precisely, because the pre-deploy worry was that it
   would publish `100 req/s` and contradict the new plan. It does not: it omits
   the limit rather than misstating it.

3. The key rotation went as the diff said: old key destroyed, new key created,
   usage plan updated in place under its original logical id. Nothing was
   removed that the diff had not named.

**Measurements.** All against `/v1/assets` on the production stage.

| What | Result |
| --- | --- |
| `403` without a key | `403` — unchanged |
| 60 requests, same path, 0.6s | 60 × `200` (~100 req/s) |
| 30 requests, unique query string each | 30 × `429` |
| 8 requests, unique, spaced 3s | 8 × `200` |
| `GetUsage` after 80 × `200` | `[0, 100000]` |

**The throttle is enforced, and the stage cache is in front of it.** A cache
miss meets the 1 req/s bucket immediately; a cache hit does not get rejected at
all, at any rate we could produce. This is the ordering question the Notes
section flagged as *our inference* — "that the quota decrements before the cache
lookup is our inference … AWS's documented throttling order names the usage
plan, stage, account and Regional limits and never mentions the cache". The
inference was wrong in its practical consequence, and the caution about not
presenting it as AWS behaviour was right.

**What these numbers do not settle**, and should not be written up as if they
did:

- **Whether a cache hit consumes a bucket token.** The 30 × `429` came straight
  after 60 cache hits, and a full bucket should have let ~5 through — which
  points at cache hits draining tokens while never being rejected themselves.
  But those 30 were fired in one parallel burst against a throttle AWS documents
  as best-effort, so the shortfall has a second candidate explanation and the
  test cannot separate them.
- **Whether the quota decrements at all.** `GetUsage` reported zero use after 80
  `200`s. Reporting lag is the ordinary explanation and almost certainly the
  right one; it is still unobserved either way.

Both belong with [[0180]], which owns the undocumented-behaviour measurements —
recorded here rather than there because 0180's file is mid-conversion to a
directory on its own branch, and editing it from here would land the same
modify/delete conflict this task took a branch off `develop` to avoid on
2026-08-10.

## Design Decisions

### From Plan

1. **1 req/s, burst 5, 100 000/month.** Rate straight from the epic; quota at the
   the top of the epic's illustrative "e.g. 50,000–100,000" — our choice, not a
   bound the epic set, and [[0010]] Alternative 6 treats it as a live cost lever
   (~$0.19/key-month at 50k, ~$0.09 at 25k), so it is revisitable downward
   without contradicting anything; burst 5 so the quickstart's parallel
   example queries do not 429. All three confirmed by Adam 2026-08-06.

2. **Plan id published via SSM** rather than read from the plan object —
   `ComputeStack` is a dependency of `ApiGatewayStack`, so [[0160]] cannot reach
   the construct without closing the cycle. Same shape as [[0124]]'s `apiBaseUrl`.

### Emerged

3. **One usage plan, renamed in place — not two plans.** The task specified adding
   a second plan and leaving `partner-plan` untouched. Three findings overturned
   that:

   - **The plan was never a tier decision.** It was created 2026-05-21 by task
     0011, a CDK bootstrap task, described there as "Usage plan + API key *pattern
     wired*". Its numbers arrived in 0040, copied from the design doc's §2.1/§7.
     The epic postdates it by two and a half months (2026-08-06).
   - **The epic does not ask for a second plan.** It says "rate-limit-only change
     … does not affect the usage-plan mechanism already agreed", and calls the
     manual tier "nothing to build here — explicitly a future problem".
   - **Nobody was using the partner key.** `GetUsage` over 30 days: **14 requests
     across 4 days**, max 4/day — the shape of our own verification curls.

   Had the usage been non-trivial, the two-plan design would have stood: silently
   dropping a live holder from 100 to 1 req/s is not a side effect to accept.

4. **`UsagePlanName` is not a replacement risk.** The task asserted renaming the
   plan "risks a CloudFormation resource replacement". Every property of
   `AWS::ApiGateway::UsagePlan` — `ApiStages`, `Description`, `Quota`, `Tags`,
   `Throttle`, `UsagePlanName` — is *Update requires: No interruption*. Confirmed
   in practice: `cdk diff` shows the plan updated in place under its original
   logical id.

5. **Key rotated as part of the same change** — through CDK rather than through
   manual CLI steps that would drift from the stack. Mechanically it rotates via
   the construct-id change (remove + add), not via `ApiKey.Name` replacement; see
   the correction in Implementation. Prompted by the old key's
   value being exposed while checking its usage — it was read with
   `get-usage-plan-keys`, which returns the plaintext `value` with no way to
   suppress it. Use `get-api-keys` without `--include-values` for that question.

5b. **The stage throttle is per-method, not an aggregate stage pool** *(found
   2026-08-11, after the code was written)*. AWS: *"Per-API, per-stage throttling
   limits are applied at the API method level for a stage."* So
   `apiGatewayThrottleRate: 200` grants **each method its own** 200 req/s bucket;
   there is no stage-wide pool that callers draw down together. The only genuinely
   shared ceiling above the usage plan is the account limit — 10 000 RPS / 5 000
   burst in `eu-central-1`.

   Three things in this task were built on the aggregate reading and had to be
   rewritten: the justification for the `/api-docs-json` method throttle (an
   anonymous loop there **cannot** 429 a key holder on `/v1/...` — the throttle
   earns its place by bounding that route's own cost instead), the
   `apiGatewayThrottleRate` doc comment, and the derivation under DD #6. The
   epic's "10 keys saturate the 1000 req/s global burst ceiling" premise
   (Context, above) does not survive this either — it should be read as a third
   rejected premise alongside the `db.t4g.micro` and $900 ones. The epic's
   conclusion is unaffected: 100 req/s per self-issued key is still wrong, just
   not for that reason.

6. **Validation is a proportionality ratio, not a capacity calculation** *(recast
   2026-08-11 — see 5b; the original text below described a stage "ceiling" that
   keys "fit under", which per-method semantics do not support)*. The planned
   check (`selfServicePlanRateLimit <= partnerPlanRateLimit`) has no counterpart
   with one plan, and would not have caught the "100 instead of 1" typo it was
   meant to catch (100 ≤ 100 passes). Replaced with: a plan limit may be at most
   **one tenth of the stage's default per-method limit**, applied to rate *and*
   burst. At 200/400 that permits 1/5 and rejects 100.

   The ratio is a judgement call, not arithmetic derived from anything. Its real
   job is narrow and worth stating plainly: stop the design doc's 100 req/s being
   reinstated by typo. A bare `<=` against the stage value would not — 200 ≥ 100
   passes. The earlier framing ("10 keys must fit under the ceiling") was
   retired by 5b; it also transplanted the epic's arithmetic, which was against
   the 1000 req/s global burst, not against `apiGatewayThrottleRate`.

   Three consequences, all intended: the pre-existing
   `apiGatewayThrottleRate >= apiKeyRateLimit` check (justified as "so a single
   key can reach its per-key SLA") was dropped, since self-service carries no SLA;
   the plan limits are now coupled to the stage defaults, so a future tier above
   one tenth of them fails synth by design; and because a plan limit must be ≥ 1,
   the coupling gives `apiGatewayThrottleRate` an implied floor of 10 — not
   binding at 200, but a constraint on anyone dialling the stage throttle down.

   **That third consequence is now a check of its own** *(2026-08-12, from review
   of [[PR 195]])*. Leaving the floor implicit meant it surfaced as an
   instruction nobody could follow: an operator clamping `apiGatewayThrottleRate`
   to 5 to shed load during an incident got `pricingApiFreePlanRateLimit (1) …
   so the maximum is 0`, while the positive-integer check rejects anything below
   1 — synth failing with two rules that contradict each other and no way out.
   The ratio was never the thing that was wrong there; the stage value was. So
   the floor is asserted directly (`apiGatewayThrottleRate`/`Burst` ≥ 10, naming
   why), and the ratio loop's stage-side guard is raised from `>= 1` to the same
   floor. Below 10 exactly one error now fires, against the field that actually
   has to change. The noise-avoidance rationale is unchanged — this is the same
   principle the `>= 1` guards already encoded, applied to a case that had been
   documented in a comment instead of enforced.

7. **`docs/runbooks/manual-api-key-tier.md` states the drift trade-off
   explicitly.** A hand-made plan does not appear in `cdk diff` and nobody reviews
   it, so the runbook carries a registry table and makes recording the resource a
   required step rather than an afterthought.

8. **Nothing in the runbook may resolve a key by taking the first match**
   *(2026-08-12, from review of [[PR 195]])*. The recovery snippet read
   `get-api-keys --name-query … --query 'items[0].id'`, and rotation re-ran the
   issue step under the *same* key name — so during the overlap the customer holds
   two keys called the same thing and `items[0]` picks one of them for reasons
   nobody can state. The failure that matters is not cosmetic: a customer stops
   paying mid-rotation, the operator recovers `KEY_ID` by name and runs "Suspend
   without destroying", and disables the **new** key while the old one keeps
   serving.

   Two facts the repo had already established, in tasks nobody thought to
   re-read while writing this runbook:

   - **[[0158]]: an API key name is not unique.** AWS enforces uniqueness on key
     *values* only; `name` is optional and duplicable, and AWS will not maintain
     the invariant for us.
   - **[[0156]]: `--name-query` has no documented matching semantics.** The whole
     of AWS's description is *"The name of queried API keys."* — not exact, not a
     prefix, nothing. The review that found this finding called it "a prefix
     match, which widens it further"; that is the claim 0156 retired on
     2026-08-10, and its correction makes the finding *stronger* rather than
     weaker — the set `items[0]` indexes into is not merely wider than expected,
     it is undefined.

   The fix is not new invention: [[0160]] hit the identical problem on the
   automated path and settled on client-side exact matching, commented so it is
   not later deleted as redundant. The runbook was already doing that for the
   *plan* (`items[?name=='…']`) two lines above the key lookup that wasn't — so
   the change extends an idiom already in the file. Three parts: lookups list
   candidates with `starts_with` + `createdDate` and make the operator choose;
   step 2 timestamps the key name so two keys never collide in the first place;
   the registry gains a **Key name** column and becomes one row per key rather
   than per plan.

   The suffix was rejected earlier in the review discussion on the grounds
   that it "widens the prefix match" — wrong order of operations, and worth
   recording because the reasoning rested on the same undocumented behaviour it
   was trying to avoid. Once lookups match client-side, the suffix is safe:
   `name=='…-key'` does not match `…-key-20260812T142317Z`, and the explicit
   `starts_with` shows both with their timestamps.

   **The suffix is to the second, not to the day** *(corrected 2026-08-12, later
   the same review)*. It was written `date -u +%Y%m%d` first, and both this entry
   and the runbook then claimed two keys "never" collide — which day-granularity
   does not deliver. The case it fails is not a corner: the likeliest rotation is
   not a scheduled one but a leak, rotated within the hour, and rotated again
   when the first replacement goes to the wrong inbox. That is the same day, so a
   day suffix reproduces exactly the duplicate-name state the suffix exists to
   prevent — and it would do so at the moment the operator is least able to
   reason carefully. `%Y%m%dT%H%M%SZ` costs one format string and makes the
   absolute claim true.

   Also written down while here, as cheap insurance rather than a finding: the
   `discord-` prefix is reserved for [[0160]]'s keys and manual keys must never
   use it. [[0164]] already tests prefix collisions *within* the self-service
   namespace (snowflakes are 17–19 digits, so one user id can prefix another);
   nothing tested across the two namespaces because nobody had declared them
   disjoint.

9. **The runbook's second half re-exports the AWS profile, and guards the one
   remaining `None`** *(2026-08-12, same review)*. "Change or revoke" states in its
   own first sentence that it runs in a shell with none of step 1's variables, then
   re-exported only `CUSTOMER` — so against a different default profile every
   lookup in it quietly targets the wrong account.

   Worth recording *why this was fixed second*. Taken on its own the finding is
   low: nothing mutates in the wrong account, because `--usage-plan-id None` is
   rejected. It was deferred once on that basis. What changed is Design Decision
   8: replacing the key lookup with a printed table removed one of the two silent
   paths outright — a wrong profile now shows up as an empty table, which an
   operator sees. That left `PLAN_ID` as the only place a failure could still pass
   itself off as a value, so the guard stopped being a patch over a snippet that
   hid failures and became the smaller job of bringing one path up to the standard
   the other already met.

   The AWS CLI detail is the reusable part, and it is verified rather than
   asserted: `--output text` prints the literal four-character string `None` for an
   empty result, so `[ -z "$VAR" ]` does **not** catch it. Expect it again in
   [[0160]]'s operational procedures. The guard warns rather than exits — the
   snippets are pasted into an interactive shell, where `return` is invalid and
   `exit` would close the operator's terminal.

   **It also unsets the variable** *(added 2026-08-12, later the same review)*.
   Warning alone left `PLAN_ID="None"` live in the shell, and a warning is exactly
   what gets scrolled past — the printed key table immediately below it is what
   the operator is looking at. Unsetting keeps the interactive-shell constraint
   above intact while moving the failure to the point of use: the next command
   that needs a plan id fails on an empty argument instead of asking AWS about a
   plan named `None` several snippets later. This is the same principle as
   Design Decision 8 — a failure must not be able to present itself as a value.

   Also corrected here: the note explaining the new lookup still described
   `--name-query` as "a cheap server-side prefilter", which the rewritten snippet
   no longer uses at all.

10. **The rotation hazard is written into `infra/Makefile`, because that is the
    only place that outlives this task** *(2026-08-12, from review of [[PR 195]])*.
    `make deploy-production` runs `cdk deploy --all --require-approval broadening`,
    which prompts on IAM and security-group widening and **not** on deletion or
    replacement — so the deploy that rotates this key destroys it without asking.

    The hazard itself was well documented, in the PR description and in this file.
    Neither survives: one disappears on merge, the other is archived. The comment
    at the `PricingApiFreeApiKey` construct warns whoever edits *those two lines*,
    which is not the person this catches — that person is running a routine deploy
    of an unrelated stack and never opens `api-gateway-stack.ts`.

    So the note sits above `deploy-production` and asks for `make diff-production`
    read for removals rather than skimmed for additions. Deliberately not phrased
    around this key: after deploy the rotation is done, and what remains true is
    the general one — an approval mode that is silent on deletion means the diff
    is the only place a destroyed resource ever announces itself.

## Notes

- **Sequencing with [[0121]] — the one-plan outcome makes this sharper, not
  softer.** *(Rewritten 2026-08-11; the original version reasoned about a
  manual-tier key with a 10 000/day quota. Neither exists — the two-plan design
  was dropped and the daily quota became monthly.)* After this task there is
  exactly one CDK-managed plan, at 1 req/s / 100 000 per month, and **no key in
  the account can sustain 100 req/s**. So the load test cannot simply pick a
  different existing key: a run against `pricing-api-free-production` measures our
  own throttle, and at 30 000 requests per 5-minute run it also spends nearly a
  third of the month's allowance. 0121 must provision a plan for the run —
  `docs/runbooks/manual-api-key-tier.md` is the procedure — and state in the
  report which plan the key was on. Applied to [[0121]] and to
  `packages/prices-api/loadtest/README.md` on 2026-08-11.
- **A cached response still costs the caller a request** — worth stating in the
  quickstart ([[0163]]), it is the first thing a partner asks. Two halves, with
  different standing *(separated 2026-08-11; this bullet previously asserted both
  as fact)*: the **billing** half is documented — API Gateway charges per call
  received, hit or miss, and the cache is billed separately by the hour. That the
  **quota** decrements before the cache lookup is **our inference** — AWS's
  documented throttling order names the usage plan, stage, account and Regional
  limits and never mentions the cache. Say it in [[0163]] with that split, the
  way the runbook now does; do not present the ordering as AWS behaviour.
- ~~The stage throttle (200 rps / 400 burst) stays the population-level ceiling;
  the usage plan is the per-key one, and the more restrictive of the two
  applies. It takes ~200 flat-out self-service keys to reach that ceiling.~~
  **Wrong — struck 2026-08-11, see Design Decision 5b.** The stage value is a
  *per-method default*, not a population pool, so no number of keys collectively
  "reaches" it. What survives: the more restrictive of stage-method and plan
  applies, and the real population ceiling is the account limit (10 000 RPS /
  5 000 burst in `eu-central-1`).
- Verification is manual at deploy time: `infra/` has no unit tests today, so
  there is nowhere to assert the plan's shape in CI without building that
  first. Out of scope here.
