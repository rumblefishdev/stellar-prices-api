---
title: "Proposal: five plans scaled linearly from CoinGecko's"
type: synthesis
status: developing
spawned_from: notes/R-coingecko-plans-and-aws-limits.md
spawns: []
tags: [portal, usage-plans, pricing]
links: []
history:
  - date: "2026-09-24"
    status: developing
    who: akot
    note: "Proposal awaiting Adam's decision on the figures"
---

# Proposal: five plans scaled linearly from CoinGecko's

## Method

Anchor: our free plan ↔ CoinGecko Demo. Each dimension gets its own factor,
because our free plan relates to Demo differently on each one:

- **Monthly quota:** 100 000 / 10 000 = **×10**
- **Rate:** 60/min (1 req/s) / 100/min = **×0.6**
- **Burst:** 5× the rate, as on the free plan (1 → 5)

Names follow CoinGecko: Basic, Analyst, Lite, Pro. AWS names follow the free
plan's pattern: `pricing-api-<tier>-<env>`. The free plan keeps its name.

## Variant A: pure linear

| Plan | Quota / month | Rate | Burst | Max reachable / month at that rate | Quota binds? |
|---|---|---|---|---|---|
| free (unchanged) | 100 000 | 1 req/s (60/min) | 5 | 2.59M | yes (25.9×) |
| Basic | 1 000 000 | 3 req/s (180/min) | 15 | 7.78M | yes (7.8×) |
| Analyst | 5 000 000 | 5 req/s (300/min) | 25 | 12.96M | yes (2.6×) |
| Lite | 20 000 000 | 5 req/s (300/min) | 25 | 12.96M | **no (0.65×)** |
| Pro | 50 000 000 | 10 req/s (600/min) | 50 | 25.92M | **no (0.52×)** |

**Problem:** Lite and Pro sell a quota that their own rate limit makes
unreachable. Running flat out for the whole month, a Lite key reaches 13M of
20M and a Pro key 26M of 50M. At CoinGecko the quota always binds (Pro:
1 000/min ≈ 43M/month vs 5M). Scaling the two dimensions by different factors
(×10 vs ×0.6) breaks that at the top of the ladder.

## Variant B (recommended): linear quotas, rates raised where the quota must stay reachable

| Plan | Quota / month | Rate | Burst | Max reachable / month | Worst-case cost / month* |
|---|---|---|---|---|---|
| free | 100 000 | 1 req/s | 5 | 2.59M | ~$1 |
| Basic | 1 000 000 | 3 req/s | 15 | 7.78M | $5–9 |
| Analyst | 5 000 000 | 5 req/s | 25 | 12.96M | $26–44 |
| Lite | 20 000 000 | **10 req/s** | **50** | 25.92M | $106–174 |
| Pro | 50 000 000 | **25 req/s** | **125** | 64.80M | $265–435 |

\* A fully drained key × $5.3–8.7 per million ([[0180]]). This is our cost,
not a price.

- The rates still rise monotonically (1 → 3 → 5 → 10 → 25), so every paid
  plan's quota is reachable.
- Capacity: 10 Pro keys at full rate = 250 req/s, which is below the
  500–900 req/s ceiling ([[0293]]). Rates, not quotas, are what threaten the
  shared backend, so the number of Pro keys we hand out should be watched.
- Enterprise is not a sixth plan. It is a per-customer plan created by hand
  (like the partner plan today), outside this ladder.

## Open for Adam

- Variant A or B, or other figures.
- Whether to show the rest of the ladder anywhere (pricing page) — not part
  of 0311.
