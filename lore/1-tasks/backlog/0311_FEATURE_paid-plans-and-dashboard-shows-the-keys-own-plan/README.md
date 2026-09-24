---
id: "0311"
title: "Five usage plans (free + Basic/Analyst/Lite/Pro), and the dashboard states the limits and usage of the key's own plan"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0157", "0180", "0187", "0188", "0191", "0193", "0293", "0307"]
tags: ["portal", "self-onboarding", "api-gateway", "usage-plans", "effort-medium", "priority-medium"]
links:
  - https://www.coingecko.com/en/api/pricing
history:
  - date: "2026-09-24"
    status: backlog
    who: akot
    note: >
      Created. We will hand single users keys with higher limits. Today a key
      moved off the free plan gets the new limits from the gateway, but the
      dashboard shows "nothing recorded yet" and 1 req/s (measured on the
      loadtest key). Plans proposed from CoinGecko's lineup, scaled linearly.
  - date: "2026-09-24"
    status: backlog
    who: akot
    note: >
      Plan figures decided: variant B (Basic 1M/3 rps, Analyst 5M/5 rps,
      Lite 20M/10 rps, Pro 50M/25 rps; burst 5x rate).
---

# Five usage plans, and the dashboard states the key's own plan

## Summary

We will give individual users keys with higher limits. The shape follows
CoinGecko: **five plans**, the existing free one plus **Basic, Analyst, Lite
and Pro**. An operator moves a user's key to a paid plan in AWS. The dashboard
must then show **that plan's** limits and **the usage counted against it**.
Today it cannot: every figure on it is tied to the free plan.

## Context — current state

Details and measurements: [notes/R-coingecko-plans-and-aws-limits.md](notes/R-coingecko-plans-and-aws-limits.md).

- Monthly Usage reads `GetUsage` **on the free plan only**
  (`portal/keys/gateway.rs:689`), and IAM allows only that plan's `/usage`
  (`api-gateway-stack.ts:966`). `limit` is reconstructed as `used + remaining`
  ([[0188]]), and the reset is a calendar-month rule in code.
- Rate Limit is env `PORTAL_RATE_LIMIT` (the free plan's rate, `compute-stack.ts:875`),
  with a front fallback of `1` ([[0193]]). Per-minute is ×60 in the page.
- **Measured on production:** the loadtest key (`gc22sbmwa2`, on its own plan)
  gets `"items": {}` from `GetUsage` on the free plan, so the dashboard shows
  "nothing recorded yet" and `1 req/s`. `GetUsagePlans --key-id` returns its
  real plan with throttle and quota.
- Issue flow: `attach_to_free_plan` (`keys/mod.rs:1225`) treats `Conflict` as
  "already on plan", so a key moved to a paid plan is not pulled back by
  issue/reveal.

## Plans

Proposal and reasoning: [notes/S-proposed-plan-tiers.md](notes/S-proposed-plan-tiers.md).
**Variant B — decided by Adam on 2026-09-24.** Quota is CoinGecko ×10;
rate is CoinGecko ×0.6, raised for Lite/Pro so that their quota stays
reachable; burst is 5× the rate.

| Plan | AWS name | Quota / month | Rate | Burst |
|---|---|---|---|---|
| free | `pricing-api-free-<env>` (unchanged) | 100 000 | 1 req/s | 5 |
| Basic | `pricing-api-basic-<env>` | 1 000 000 | 3 req/s | 15 |
| Analyst | `pricing-api-analyst-<env>` | 5 000 000 | 5 req/s | 25 |
| Lite | `pricing-api-lite-<env>` | 20 000 000 | 10 req/s | 50 |
| Pro | `pricing-api-pro-<env>` | 50 000 000 | 25 req/s | 125 |

> These figures go into the per-env CDK config. The dashboard code does not
> depend on them: it reads whatever the key's plan says.

## Implementation Plan

### Step 1: Infra — four plans in CDK
- Per-env config for the paid tiers in `infra/src/lib/types.ts` (name, rate,
  burst, monthly quota), beside the existing `pricingApiFreePlan*`. Add a
  `UsagePlan` per tier on the same stage in `api-gateway-stack.ts`. The free
  plan's construct id `UsagePlan` stays, so the deployed plan is not replaced.
- All quotas `period: MONTH`, `offset: 0`, like the free plan.
- IAM for the portal Lambda: `apigateway:GET` on `/usageplans` (the
  `keyId`-filtered list cannot be scoped to one plan) and on
  `/usageplans/*/usage`. Both are read-only. This widens [[0188]] decision 1
  deliberately; record it.
- Pass the API id and stage to the Lambda so the backend can pick the plan
  for **this** stage (the loadtest plan and a partner plan share the account).

### Step 2: Backend — which plan the key is on, then usage on that plan
- `Gateway::plan_of(key_id)`: `GetUsagePlans` with `keyId`, keep the plans
  whose `apiStages` contain our API + stage, and return `PlanInfo { id, name,
  tier, rate_limit, burst_limit, quota_limit, quota_period }`. The tier comes
  from the name (`pricing-api-<tier>-<env>`). Any other plan (loadtest,
  hand-made Enterprise) reports as `custom` with its name.
- `usage_of(plan_id, key_id, …)`: `GetUsage` on the key's plan, not on
  `free_plan_id`.
- `/api/usage` (`portal/usage/mod.rs`) gains `plan { tier, name,
  rate_limit_per_second, burst_limit, quota_limit, quota_period }`. `limit`
  comes from `quota_limit`, and `used + remaining` becomes a cross-check only.
  Reset/period come from `quota.period` + `offset`. MONTH is supported; any
  other period is reported, not guessed.
- Distinct states: a key on **no plan** for this stage (the [[0187]] "issued
  but dead" state), and a plan **without** quota or throttle (unlimited). No
  zeros, no "nothing recorded yet" for either.
- Cache the plan inside the same `UsageCache` entry (60 s TTL). A plan change
  shows up within a minute. No cache invalidation from outside.

### Step 3: Frontend — show the plan the key is on
- `PortalUsage` (`web/portal/src/api/portal.ts`) mirrors the new `plan`.
- Rate Limit card: per-second = `usage.plan.rate_limit_per_second`,
  per-minute = ×60 (the look stays as it is, Adam 2026-09-23). `/config`
  stays the source only for the no-key state and the landing page.
- Monthly Usage: limit and reset date from the plan. Plan name visible on the
  dashboard (e.g. "Basic plan" next to the card title).
- "Need higher limits? Contact us…": see Open questions, and [[0307]].

### Step 4: Operator runbook — upgrading a user
1. Key name from the Discord id: `discord-<id>-key` (`naming.rs:60`); exact
   match (`nameQuery` is a prefix match):
   `aws apigateway get-api-keys --name-query "discord-$ID-key" --query "items[?name=='discord-$ID-key' && enabled].id"`.
2. `delete-usage-plan-key --usage-plan-id <free> --key-id <K>`, then
   `create-usage-plan-key --usage-plan-id <tier> --key-id <K> --key-type API_KEY`.
   The key answers `403` for the seconds in between; its value does not change.
3. Verify: `get-usage-plans --key-id <K>` → exactly the target plan.
4. The new plan counts from zero (usage is per (plan, key) pair).
5. **Warn the user:** a rework ([[0191]]) mints a new key on the free plan.

### Step 5: Verification
- Unit: plan selection by stage (several plans, other API, none), reset per
  period, tier from name, response shape. Portal specs for each tier + no
  plan + custom.
- Dev: move a test key free → Basic → Pro → free. After each step, the
  gateway throttles at the plan's rate and the dashboard shows the plan's
  figures within 60 s.

## Acceptance Criteria

- [ ] Four paid plans exist in CDK with per-env config; the free plan is
      untouched (no replacement in the CFN diff).
- [ ] `/api/usage` reports the key's actual plan (tier, rate, burst, quota,
      period) and the usage counted on that plan.
- [ ] A key moved to any tier: both cards show that tier's figures within
      60 s. Free keys look as they do today.
- [ ] No-plan and unlimited/custom-plan keys render stated, distinct states.
- [ ] IAM widened only to `GET /usageplans` and `GET /usageplans/*/usage`.
- [ ] Runbook for upgrading a user (Step 4) in the wiki or ops docs.

## Open questions

- **"Contact us for commercial plans"** on a paid plan: hide it, or change it
  to "Need more? Contact us"? Related to [[0307]] (the contact leads nowhere).
- **Plan name on the page:** where and how. Figma has no frame for it.

## Out of scope

- A rework ([[0191]]) keeping the plan: the new key lands on free. Adam
  2026-09-23: not now. Once paid keys are issued, this is a real downgrade path.
- Billing, a pricing page, self-service upgrade, a plan registry in DynamoDB.
- Enterprise: per-customer plans made by hand, reported as `custom`.
