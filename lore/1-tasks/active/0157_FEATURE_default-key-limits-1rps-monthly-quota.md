---
id: "0157"
title: "Default key limits: 1 req/s + monthly quota, not the design doc's 100 req/s"
type: FEATURE
status: active
related_adr: ["0008", "0010"]
related_tasks: ["0121", "0156", "0158", "0160", "0163", "0171"]
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
      single-date property is design intent until [[0171]] #7 measures it.
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
      `selfServicePlan*`; six were planned, the one-plan outcome halved it.
      Key rotated in the same change (construct-id change → remove + add, not
      `ApiKey.Name` replacement) after its value was exposed by
      `get-usage-plan-keys` during the usage check.
      Verified by `tsc`, `cdk synth` and `cdk diff` against the live stack: plan
      updates in place under logical id `ApiUsagePlanDBBE8AB1`, key removed and
      re-added under a new logical id, SSM parameter added. Not yet deployed —
      four ACs stay open until then.
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
  | `apiKeyRateLimit` | `selfServicePlanRateLimit` (1) |
  | `apiKeyBurstLimit` | `selfServicePlanBurstLimit` (5) |
  | `apiGatewayPartnerDailyQuota` | `selfServicePlanMonthlyQuota` (100000) |

  Three renames, no additions — the one-plan outcome collapsed the six fields
  this table originally listed into three. The reason to rename here rather than
  later: `apiKeyRateLimit` reads as a global property of API keys, when it is one
  plan's rate.

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
  `SelfServiceApiKey`), which changes the logical id — so CloudFormation sees
  `ApiPartnerApiKey8034B29D` removed and `ApiSelfServiceApiKey90815276` added,
  two unrelated resources, and `Name`'s replacement semantics never come into
  play. The outcome is identical (new value, old key destroyed); the CFN verb is
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
  assume it" — was the right one and is now tracked as [[0171]] #7. Until it is
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
      decrements; `403`-without-key behaviour unchanged *(needs deploy)*
- [ ] The quickstart's example queries run without hitting the burst limit
      *(needs deploy; [[0163]] not written yet)*
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
- [ ] Epic AC 5 satisfied: default limits are 1 req/s + monthly quota, not the
      design doc's 100 req/s *(code done; closes on deploy)*
- [ ] `docs/scf/milestone-1-evidence.md:795` states 100 req/s as a delivered
      property — must be reconciled with the new limit before this task closes

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

7. **`docs/runbooks/manual-api-key-tier.md` states the drift trade-off
   explicitly.** A hand-made plan does not appear in `cdk diff` and nobody reviews
   it, so the runbook carries a registry table and makes recording the resource a
   required step rather than an afterthought.

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
