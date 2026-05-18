---
title: "R: Cost delta — current RDS plan vs. Hetzner CH shared-tenant refactor"
type: research
status: developing
spawned_from: ../README.md
spawns: []
tags: [cost, aws, hetzner, rds, lambda, step-6]
links:
  - "../../../../../docs/prices-api-general-overview.md"
  - "./R-be-hetzner-ch-shape.md"
  - "./R-ingest-target-mapping.md"
  - "./R-aws-hetzner-auth-network.md"
  - "./I-schema-ownership-options.md"
history:
  - date: 2026-05-18
    status: developing
    who: okarcz
    note: >
      Cost-delta math grounded in §10 (Cost Estimate) and §11
      (Infrastructure Sharing) of the design doc plus AWS list
      prices for items not directly enumerated.
---

# R: Cost delta — current RDS plan vs. Hetzner CH shared-tenant refactor

## Purpose

Step 6 of task 0044. Quantify the monthly recurring cost change
and the one-time implementation cost. Money is not the only axis
that matters — non-monetary deltas (operational complexity,
Lambda count, blast radius) get their own table.

Inputs:

- Design doc §10: post-backfill ~$117/mo total per env;
  scaled-up ~$712/mo at high traffic.
- Design doc §11.1: already-shared BE infra (Galexie, S3, VPC, NAT
  Gateway) saves ~$73/mo per env. **Not in scope of this delta
  — that saving holds for both plans.**
- Step 1: BE Hetzner box is BE-funded; specs not yet public.
- Step 2: OHLCV Rollup Lambda eliminated by MV chain.
- Step 4: AWS-side adds ~$0.40/mo Lambda egress + ~$2.40/mo
  Secrets Manager secrets across three envs.
- Step 5: schema-migration tooling is a one-shot CI Lambda; no
  recurring runtime.

---

## 1. Pricing baseline (AWS us-east-1, list prices, 2026-05-18)

| Item | Unit price | Source |
|---|---|---|
| RDS `db.t4g.micro` Single-AZ | ~$0.016/hr → ~$12/mo | Design doc §10 |
| RDS `db.t4g.small` Single-AZ | ~$0.032/hr → ~$25/mo | Design doc §10 |
| RDS `db.t4g.medium` Single-AZ | ~$0.064/hr → ~$50/mo | Design doc §6.2 scaling table |
| RDS `db.r6g.large` + Multi-AZ | +~$350/mo (vs. base) | Design doc §10 scaled-up |
| RDS read replica | +~$175/mo | Design doc §10 scaled-up |
| RDS Proxy | +~$25/mo | Design doc §10 scaled-up |
| RDS GP3 storage | ~$0.115/GB-mo | AWS list |
| RDS backup storage beyond 1× | ~$0.095/GB-mo | AWS list |
| Lambda invocation | $0.20 per 1M requests | AWS list |
| Lambda duration | $0.0000166667 per GB-second | AWS list |
| Lambda egress (internet) | $0.09/GB | AWS list |
| Secrets Manager | $0.40 per secret per month | AWS list |
| CloudWatch alarm | $0.10 per alarm per month | AWS list |

---

## 2. Per-environment monthly delta — steady state (post-backfill, low traffic)

The unit of comparison is **one environment** (dev / staging / prod).
The current §10 table lists only "Prices API" totals; assume the
same shape per environment.

### 2.1 Removed lines

| Item | Today's $/mo | Hetzner $/mo | Delta |
|---|---|---|---|
| RDS `db.t4g.micro` Single-AZ | $12.00 | $0 | **-$12.00** |
| RDS storage (20 GB GP3) | $2.30 | $0 | **-$2.30** |
| RDS automated backups (~10 GB beyond 1×) | ~$1.00 | $0 | **-$1.00** |
| **Subtotal — removed** | **$15.30** | | **-$15.30** |

### 2.2 Added lines

| Item | $/mo | Notes |
|---|---|---|
| Secrets Manager — 2 secrets (cert + key) | $0.80 | Per env. Today's plan has 1 DB-password secret; net add is ~$0.40 |
| Lambda outbound egress to Hetzner | $0.40 | ~150 MB/day × 30 × $0.09/GB |
| CloudWatch — cert NotAfter alarm | $0.10 | One alarm per env |
| Hetzner CH share | $0 to $15 | **Negotiation variable.** See §6 sensitivity |
| **Subtotal — added (free-ride scenario)** | **$1.30** | |
| **Subtotal — added ($15 cost-share)** | **$16.30** | |

### 2.3 Net per env per month (steady state)

| Cost-share scenario | Net delta |
|---|---|
| Free ride ($0 to BE) | **-$14.00 / month** |
| 10% pro-rata Hetzner share (~$12) | **-$2.00 / month** |
| Fixed $15/mo flat per env | **+$1.00 / month** |
| Fixed $20/mo flat per env | **+$6.00 / month** |

**At steady state the recurring delta is small in absolute
terms** (-$14 to +$6 per env, per month). The cost-share
negotiation determines the sign but not the magnitude. **The
monetary case at steady state is roughly a wash.**

---

## 3. Per-environment monthly delta — at scale (high traffic)

This is where the math diverges. §10's scaled-up table adds these
RDS-only items as traffic grows:

| Item (at scale) | Today's $/mo | Hetzner $/mo | Delta |
|---|---|---|---|
| Upgrade to `db.r6g.large` + Multi-AZ | $350 | $0 | **-$350** |
| Add read replica | $175 | $0 | **-$175** |
| Add RDS Proxy | $25 | $0 | **-$25** |
| **Subtotal — RDS scale-up removed** | **$550** | | **-$550** |
| Hetzner cost-share scaling | n/a | ~$0–$30/mo step | (mostly absorbed) |
| **Net at scale** | | | **-$520 to -$550 / month** |

**The scaled-up scenario is where the refactor pays.** When
prices-api hits sustained CPU pressure or needs HA, the RDS
upgrade path is steep; Hetzner CH absorbs the same load on
BE's already-funded box.

This is also where the cost-share conversation can break down
gracefully — if BE asks for chargeback at scale, both sides
have leverage to land on a fair number because BE is operating
a cluster that benefits prices-api too.

---

## 4. Per-environment monthly delta — backfill window

§10 backfill table: ~$30 one-time AWS cost (RDS upgrade to
`db.t4g.small` during push windows). Per ADR 0001/0005 the
backfill is workstation-local, so neither plan touches Fargate
for it.

| Item | Today | Hetzner |
|---|---|---|
| RDS upgrade during pushes | ~$30 | $0 |
| Workstation electricity / ISP | operator-paid | operator-paid |
| Hetzner cost-share during pushes | n/a | $0 (negligible additional load) |

**Net backfill delta: -$30 one-time.** Trivial.

---

## 5. Across three environments

Multiply §2.3 by three for dev + staging + prod:

| Cost-share scenario | Steady-state delta (3 envs) |
|---|---|
| Free ride | **-$42 / month** = **-$504 / year** |
| 10% pro-rata (~$12/env) | **-$6 / month** = **-$72 / year** |
| Fixed $15/mo flat per env | **+$3 / month** = **+$36 / year** |
| Fixed $20/mo flat per env | **+$18 / month** = **+$216 / year** |

And at scale across all three:

| Cost-share scenario | At-scale delta (3 envs) |
|---|---|
| Free ride | **-$1,650 / month** = **-$19,800 / year** |
| Even with $30/mo flat per env | **-$1,560 / month** = **-$18,720 / year** |

The scaled scenario is also where dev/staging do NOT scale up
the RDS; only prod does. So the **realistic at-scale annual
delta is dominated by the production environment** and the
2-env savings on steady-state mostly stay flat:

| Realistic at-scale (prod scaled, dev+staging steady) | $/year |
|---|---|
| Free ride | **-$6,936** (=$528/mo prod + $14×2/mo non-prod, ×12) |
| 10% pro-rata | **-$6,816** (small flat $12/env applied) |

---

## 6. Cost-share sensitivity analysis

The Hetzner box is BE-funded; prices-api's incremental cost on
that box is what's negotiable. The math above bracketed $0–$30/mo
per env. Concretely:

- **Storage share.** Step 2 sized prices-api at ~32 GB/year per
  CH-B model vs. BE's ~800 GB pubnet backfill. **~4%** by volume.
- **Row-write share.** Prices-api writes ~5–10 OHLCV rows per
  ledger vs. BE's ~hundreds of event rows per ledger. **~2–5%**
  by row count.
- **CPU share.** Read load is small (API queries vs. BE's
  internal indexer); writes are bounded by Lambda concurrency
  caps. **~5–10%** estimated.

**Fair-share band: 5–10% of Hetzner cost.** If the Hetzner
dedicated server is in the typical $60–$150/mo range, prices-api's
fair share is **$3–$15/mo per env**, summing to **$9–$45/mo
across three envs**.

Three negotiation outcomes are realistic:

1. **Free ride.** BE views the shared infra as cost-amortising
   for them (matches the §11.1 framing for Galexie/S3/VPC/NAT).
   Saves prices-api $42–$504/yr (3 envs steady state) and
   $6,936/yr at production scale.
2. **Pro-rata fair share ($9–$45/mo across 3 envs).** Honest
   accounting. Saves the at-scale figure largely intact.
3. **Per-env flat fee ($15–$20/mo).** Easier to administer.
   Roughly cost-neutral at steady state; still saves at scale.

All three keep the refactor cheaper than the current RDS plan
once prices-api hits any scale-up trigger.

---

## 7. One-time implementation cost (engineering)

Quantified in engineer-weeks, since dollar-cost depends on rate
which is out of scope for this note.

| Workstream | Estimate |
|---|---|
| Schema design + migration files (Step 2 + 5 output → DDL) | 3–5 days |
| Migration applier (~200-line Rust binary, per step 5 §3.4) | 2–3 days |
| Retarget Ledger Processor Lambda (sqlx → clickhouse crate; mTLS reqwest::Identity) | 1 week |
| Retarget Current Price Updater + Oracle Fetcher + Asset Discovery + Cleanup Worker | 1 week |
| Delete OHLCV Rollup Lambda + replace with MV chain in init.sql | 1–2 days |
| mTLS plumbing (Secrets Manager wiring, OnceCell pattern, NotAfter alarm) | 2–3 days |
| CDK changes (RDS out, no VPC, Lambda env vars, IAM scope for Secrets Manager) | 3–5 days |
| Integration test rewrite (Postgres docker → CH docker container) | 1 week |
| Coordination with BE (cert issuance, DB carve, fan-out shape, Caddy capacity) | 1 week (mostly waiting; not engineering-time) |
| Cutover + observability + runbook | 1 week |
| **Total engineering, end-to-end** | **~4–5 person-weeks** |

If implementation lands during the steady-state band of §2.3, the
payback period on monetary terms alone is "never under free ride
+ realistic engineer rates" — i.e. **the monetary case at
steady state does not justify the engineering investment by
itself**. The justification has to come from §3 (at-scale
savings), §8 (non-monetary deltas), or both.

---

## 8. Non-monetary deltas

These don't show up in §10 but they're real.

### 8.1 Removed operational surface

| Removed | Annual ops cost saved (rough) |
|---|---|
| Production Postgres database | ~5–10 person-hours / quarter (patching, parameter tuning, slow-query investigation) |
| VPC + subnets + route tables | one-time setup eliminated; reduces ongoing CDK churn |
| Security groups for Lambda→RDS | one-time |
| RDS Proxy when added | ~2 person-hours / quarter saved |
| Lambda OHLCV Rollup | ~1 person-hour / quarter (cron-job monitoring) |

### 8.2 Added operational surface

| Added | Annual ops cost added (rough) |
|---|---|
| mTLS client cert lifecycle | ~2 person-hours / year (annual rotation + alarm setup) |
| Migration applier maintenance | ~1 person-day / year |
| CH-specific debugging skill | one-time learning curve; ~1 person-week to bootstrap |
| Cross-team coordination during initial setup | ~1 person-week (largely one-time) |

### 8.3 Net non-monetary

**Strictly less day-to-day operational surface in steady state.**
The added items are mostly one-time (learning curve, setup); the
removed items are recurring. Cert rotation is the only meaningful
new recurring task and it's ~2 hours/year.

The Lambda count drops by 1 (Rollup eliminated). The Postgres
codepath in the Lambdas (sqlx queries, connection pooling)
disappears; replaced by ~50 lines of `clickhouse` crate usage.
Net code surface is roughly even, possibly smaller.

---

## 9. Risk-adjusted view

Bullet points the eventual ADR should weigh:

- **Best case** (free ride + scale-up materialises): -$7k/yr in
  AWS bill + reduced ops surface. Implementation pays back in
  first year of production scale.
- **Likely case** (pro-rata share + moderate scale): roughly
  cost-neutral at steady state, -$5k+/yr at scale. Implementation
  pays back in 12–18 months.
- **Worst case** (BE asks for $20/env flat + prices-api never
  scales beyond steady state): +$216/yr in AWS bill + 4–5 weeks
  engineering with no monetary payback. Non-monetary wins still
  apply (less operational surface, simpler topology).

The **expected-value calculus favors the refactor** under any
reasonable cost-share scenario as long as the team believes
prices-api will eventually hit scale-up triggers. If the team's
position is "we'll always be a single `db.t4g.micro`", the
refactor is harder to justify on cost alone — and the right
answer might be to defer.

---

## 10. Open questions surfaced by step 6 (forwarded to README)

25. **Cost-share number with BE.** Free ride vs. pro-rata vs.
    fixed fee. Material to the recurring-cost math. *Action:*
    cross-team conversation; carry a fair-share proposal of 5–10%.
26. **Hetzner box monthly cost.** BE has not published. Needed
    to ground the pro-rata math. *Action:* ask BE.
27. **Production scale-up timeline.** If prices-api is unlikely
    to hit scale-up triggers in year 1, the refactor's monetary
    case weakens. *Action:* product / traffic projection.
28. **Engineering capacity for 4–5 person-weeks.** Implementation
    is non-trivial. *Action:* sequencing decision against
    competing roadmap.

---

## 11. What step 6 does NOT cover

- The actual ADR + scoping for the implementation tasks if the
  recommendation is "go" — step 7's synthesis.
- Sensitivity to BE schedule (when does Hetzner CH land in
  production?) — step 7 should bound this.
- Risk-weighted comparison versus alternatives like "stay on
  RDS but reduce instance class" — the framing of the
  refactor as the only available cost lever is itself a
  recommendation-level decision.
