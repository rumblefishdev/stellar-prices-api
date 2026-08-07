---
id: "0157"
title: "Default key limits: 1 req/s + monthly quota, not the design doc's 100 req/s"
type: FEATURE
status: backlog
related_adr: ["0008"]
related_tasks: ["0121", "0156", "0158", "0160", "0163"]
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
      against them. Quota fixed at 100 000/month (top of the epic's range) and
      burst at 5; both confirmed by Adam on 2026-08-06.
  - date: 2026-08-07
    status: backlog
    who: akot
    note: >
      Added the calendar-alignment of the monthly quota (it settles [[0160]]'s
      period-boundary question), and corrected the [[0121]] sequencing note —
      the partner key's daily quota kills a sustained 100 rps run after ~100
      seconds, which the original note did not catch.
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
  a range of 50 000–100 000 calls/month; take **100 000** — ~3 300/day, ~6.7×
  CoinMarketCap's free tier, and a round number the dashboard ([[0160]]) can
  render as "used / 100 000 this month" without explanation. A per-second limit
  alone does not stop a key idling at 1 req/s from producing 2.6M
  requests/month.
- **Anything higher is manual and out of band** — someone creates a key with a
  higher quota by hand (AWS console/CLI), payment happens by bank transfer
  outside the product. No self-serve upgrade flow, no in-app billing.
- **No change** to `403`-without-key or to the usage-plan mechanism.

**Follows from the epic, but not stated in it**

- **Two usage plans on the same stage.** The epic requires 1 req/s for
  self-issued keys *and* a hand-issued higher tier; a single plan cannot hold
  both. So add `prices-<env>-selfservice-plan` (1 rps, 100 000/month) and leave
  `prices-<env>-partner-plan` exactly as it is — same logical id, same name,
  same limits — as the manual tier. Renaming it risks a CloudFormation resource
  replacement that would invalidate a working partner key. A key belongs to one
  plan per stage, so `(usagePlanId, apiKeyId)` stays unambiguous for `GetUsage`.
- **Burst 5.** The epic fixes the sustained rate and says nothing about burst.
  At burst 1 the quickstart page ([[0163]]) firing two example queries in
  parallel 429s on a key we issued ourselves, which contradicts epic AC 3.
  Token-bucket refill keeps the sustained rate at exactly 1 req/s; burst only
  allows the allowance to be spent unevenly. 1:5 is the same shape as the
  existing 100:200 partner plan and 200:400 stage throttle.
- New config in `infra/src/lib/types.ts` + `envs/production.json`:
  `selfServiceRateLimit` (1), `selfServiceBurstLimit` (5),
  `selfServiceMonthlyQuota` (100000), validated like the existing key fields
  (positive integers, burst ≥ rate) plus
  `selfServiceRateLimit <= apiKeyRateLimit` — cheap, and it catches the "100
  instead of 1" typo this whole task exists to prevent. Note that this last
  check also encodes "self-service is never faster than the manual tier", which
  is intended but will fail synth if the partner tier is ever lowered.
- **Settle the config naming scheme while adding the second tier.** The existing
  fields are already inconsistent (`apiGatewayPartnerDailyQuota` versus
  `apiKeyRateLimit` / `apiKeyBurstLimit`), and from now on every field has to say
  which plan it belongs to. Pick one shape here rather than after a third tier
  makes it expensive.
- Quota period `apigateway.Period.MONTH`. Note that **a usage plan carries
  exactly one quota** — no daily sub-cap alongside the monthly one — so a key at
  full rate spends the month's allowance in ~28 hours and then waits for the
  reset. That is only acceptable because [[0160]] caps rework at once per quota
  period; otherwise a user would simply mint a fresh key, which is exactly the
  loophole the epic's rework rule closes.
- **The monthly quota is calendar-aligned: it resets on the 1st at 00:00 UTC**,
  not on a rolling window from key creation. That is what makes [[0160]]'s
  boundary ("first of the month following the last rework") and the epic's
  "calendar month" the same date, so there is one date to render, not two.
  Cheap to confirm on the first deploy — do it rather than assume it.
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

- [ ] Self-service usage plan in CDK: 1 req/s sustained, burst 5, 100 000
      requests/month
- [ ] A key on that plan is throttled at 1 req/s and its monthly quota
      decrements; `403`-without-key behaviour unchanged
- [ ] The quickstart's example queries run without hitting the burst limit
- [ ] Partner plan and key unchanged and still working — verified against the
      deploy diff, not assumed
- [ ] Self-service usage plan id readable by [[0160]] without closing the
      Compute → ApiGateway cycle
- [ ] New config validated at synth like the existing key fields
- [ ] Manual higher-tier runbook written
- [ ] Epic AC 5 satisfied: default limits are 1 req/s + monthly quota, not the
      design doc's 100 req/s

## Notes

- **Sequencing with [[0121]] — and the partner key does not solve it either.**
  The 100 req/s load test must not run against a self-service key, or it just
  measures our own throttle. But the manual-tier key carries a **10 000/day
  quota**, which at 100 req/s is exhausted in **100 seconds** — so a sustained
  test hits `429` from the quota rather than the throttle, and reports the same
  configuration artefact by a different route. Either give the load test its own
  quota-free plan, raise the partner quota for the duration, or cap the run
  below 100 seconds and say so in the report. Belongs in [[0121]]; recorded here
  because it surfaced while sizing this task.
- Throttle and quota are evaluated *before* the response cache, so a cached
  response still counts against the caller's quota. Worth stating in the
  quickstart ([[0163]]) — it is the first thing a partner asks.
- The stage throttle (200 rps / 400 burst) stays the population-level ceiling;
  the usage plan is the per-key one, and the more restrictive of the two
  applies. It takes ~200 flat-out self-service keys to reach that ceiling.
- Verification is manual at deploy time: `infra/` has no unit tests today, so
  there is nowhere to assert the plan's shape in CI without building that
  first. Out of scope here.
