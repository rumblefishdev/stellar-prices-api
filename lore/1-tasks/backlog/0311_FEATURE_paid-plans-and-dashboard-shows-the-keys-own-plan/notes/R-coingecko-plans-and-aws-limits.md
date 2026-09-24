---
title: "CoinGecko API plans, our AWS limits and what the portal reads today"
type: research
status: mature
spawns:
  - notes/S-proposed-plan-tiers.md
tags: [portal, usage-plans, pricing]
links:
  - https://www.coingecko.com/en/api/pricing
history:
  - date: "2026-09-24"
    status: mature
    who: akot
    note: "Collected while scoping 0311"
---

# CoinGecko API plans, our AWS limits and what the portal reads today

## CoinGecko (read 2026-09-24, https://www.coingecko.com/en/api/pricing)

| Plan | $/month (monthly billing) | Credits / month | Rate / min | API keys |
|---|---|---|---|---|
| Demo (free) | 0 | 10 000 | 100 | 1 |
| Basic | 35 | 100 000 | 300 | 5 |
| Analyst | 129 | 500 000 | 500 | 10 |
| Lite | 499 | 2 000 000 | 500 | 10 |
| Pro | 999 | 5 000 000 (8M/10M/15M selectable, unpriced) | 1 000 | 10 |
| Enterprise | inquire | custom | custom | custom |

- 1 call = 1 credit. Credits reset on the 1st of each calendar month
  whatever the billing cycle. Unused credits do not roll over. Overage is
  $0.0005/call on paid plans.
- The page contradicts itself in its FAQ ("300 to 2,500 calls/min", a
  "Pro+" plan). Those look like leftovers from an older lineup; the table
  above is the plan grid itself.
- Five priced plans (Demo + 4 paid) plus Enterprise (custom). Enterprise is a
  per-customer deal, not a plan in the grid.

## Our AWS account (production, eu-central-1, 2026-09-24)

- Account throttle: 10 000 req/s, burst 5 000.
- Service quotas: 300 usage plans, 10 000 API keys, 10 usage plans per key,
  20 stage throttles per plan.
- A key can be on **one plan per API stage**, so moving it means
  `delete-usage-plan-key` then `create-usage-plan-key`.
- Existing plans:

| id | name | rate / burst | quota | API / stage |
|---|---|---|---|---|
| `71t9im` | `pricing-api-free-production` | 1 / 5 | 100 000 MONTH | `02mabge71l/production` |
| `i12bsj` | `prices-production-loadtest-plan` | 150 / 300 | 1 000 000 MONTH | `02mabge71l/production` |
| `q7sd40` | `production-partner-plan` | 50 / 100 | 10 000 DAY | `6l9k06w4pl/production` (other API) |

## Backend capacity and cost

- [[0293]]: the shared ClickHouse box has a ceiling between **500 and ~900 req/s**.
  Every paid plan's rate has to be read against that shared ceiling, not
  against the 10 000 account limit.
- [[0180]] `R-all-in-per-call-cost.md`: all-in cost is 1.4–2.3× the
  gateway-only $0.38 per 100 000 calls, i.e. about **$5.3–8.7 per million**
  calls. The upper bound for a fully drained key is quota × that.

## What the dashboard reads today (develop 1c0bc434)

| Field | Source |
|---|---|
| used / remaining | `GetUsage` with `usage_plan_id = free_plan_id` (`portal/keys/gateway.rs:689`) |
| limit | `used + remaining` ([[0188]]); no call reports the quota |
| reset date | calendar-month rule in code |
| per-second | env `PORTAL_RATE_LIMIT` = `pricingApiFreePlanRateLimit` (`compute-stack.ts:875`), front fallback `1` (`app.tsx:3659`) |
| per-minute | per-second × 60 in the page |

IAM allows only `GET /usageplans/{freePlanId}/usage` (`api-gateway-stack.ts:966`).

**Measured (read-only):** loadtest key `gc22sbmwa2`:
- `GetUsage` on the free plan → `"items": {}`, so the dashboard would show
  "nothing recorded yet" and `1 req/s`;
- `GetUsage` on its own plan → `[0, 1000000]`;
- `GetUsagePlans --key-id gc22sbmwa2` → exactly its plan, with throttle and
  quota. One call gives the dashboard everything it needs.
