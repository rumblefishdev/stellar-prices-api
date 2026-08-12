---
title: "All-in per-call cost, and whether ADR 0010's proportionality argument survives it"
type: research
status: mature
spawned_from: notes/Q-which-undocumented-behaviours-hold.md
spawns: []
tags: [aws, cost, lambda, clickhouse, adr-0010]
links:
  - "../../../../2-adrs/0010_discord-account-model-and-abuse-barrier.md"
  - "../../../archive/0156_RESEARCH_self-service-auth-assumptions/notes/R-abuse-mitigation-options-costed.md"
history:
  - date: 2026-08-12
    status: seed
    who: akot
    note: "Created empty; this is arithmetic over existing CloudWatch metrics, not a measurement — no Discord prereq"
  - date: 2026-08-12
    status: mature
    who: akot
    note: >
      Computed. All-in is 1.4–2.3× ADR 0010's $0.38, so the proportionality
      argument survives. Three corrections fall out: $3.50/M is the us-east-1
      rate (Frankfurt is $3.70), Lambda duration is a marginal term of the same
      size as gateway rather than a rounding error, and ClickHouse is ~€1.10/mo.
      Bounded by there being no representative traffic — 16 invocations in 90
      days, essentially all cold starts.
---

# All-in per-call cost

Covers item 9. **Not a spike** — no Discord app, no scratch guild, no scratch
usage plan. It is arithmetic over metrics the deployed `prices-api` already
emits, which is why it can run first, before any of the setup lands.

> **Status: computed 2026-08-12.** Verdict at the bottom.

## What is wrong with the current figure

Every cost figure in [[0156]] is **gateway-only**: $3.50/million requests +
$0.09/GB transfer. Compute, ClickHouse query time and storage reads are
unpriced — they were simply not in scope there.

ADR 0010 then declines paid abuse mitigations (CAPTCHA, email verification) on
a proportionality argument: a fully-drained key costs us about **$0.38**, which
is too small to justify the friction. That number is gateway-only.

**If backend cost dominates by 10×, the exposure is ~$4/key and the argument
needs revisiting.** Not necessarily reversing — but it stops being obvious, and
an ADR resting on an obviously-true number should not quietly rest on a
possibly-false one.

## How to compute it

Per request, from CloudWatch over a representative window (state the window):

| Component | Source | Rate |
|---|---|---|
| Gateway request | AWS pricing | $3.50 / million |
| Gateway transfer | response size × | $0.09 / GB |
| Lambda duration | `Duration` p50/p95 × memory | GB-second price |
| Lambda requests | invocation count | $0.20 / million |
| ClickHouse query time | query duration / read bytes | shared Hetzner box — see below |
| CloudWatch logs | ingest per invocation | $0.50 / GB |

**ClickHouse is not usage-priced.** It is a fixed monthly box (ADR 0007), so
per-call cost there is an *allocation*, not a bill. A marginal-cost reading puts
it at zero, which is misleading in one direction; dividing the whole box cost by
our request volume is misleading in the other, because **`ch-prod-01` is shared
with the BE team** — see `docs/runbooks/0136-coarse-rollup-merge-recovery.md:33`.
Allocate our share (query time is the honest divisor, ingestion and backfill
dominate the box) and state the basis explicitly next to the number.

Use the **monthly quota** as the multiplier for a fully-drained key, so the
result is directly comparable to ADR 0010's $0.38.

## Result — measured 2026-08-12

### Inputs, and where each came from

| Input | Value | Source |
|---|---|---|
| Monthly quota | 100,000 req | `infra/envs/production.json` |
| Lambda memory | 512 MB | `production.json` → `apiHandler.memoryMb` |
| Duration | avg 349 ms, p50 396 ms, p95 496 ms | CloudWatch, `prices-production-api-handler`, 90 d |
| **Sample size** | **n = 16 invocations / 90 days** | — see the caveat below |
| API GW request | **$3.70 / million** | AWS Pricing API, `EUC1-ApiGatewayRequest` |
| Lambda request | $0.20 / million | `EUC1-Request` |
| Lambda compute | $0.0000166667 / GB-s | `EUC1-Lambda-GB-Second` tier 1 |
| Data transfer out | $0.09 / GB | standard |
| Dedicated cache 0.5 GB | **$0.020 / hour = $14.60 / month** | `EUC1-ApiGatewayCacheUsage:0.5GB` |

### Per call, and per fully-drained key

Response size is not measured (no representative traffic to measure it on).
ADR 0010's $0.38 implies ~3.5 KB per response — $0.35 requests + ~$0.03
transfer — so the first three rows hold that constant and vary only duration.

| Scenario | $/call | $/drained key | vs ADR's $0.38 |
|---|---|---|---|
| p50 396 ms, 3.5 KB | $7.50e-6 | **$0.75** | 2.0× |
| p95 496 ms, 3.5 KB | $8.33e-6 | $0.83 | 2.2× |
| warm 150 ms (assumed), 3.5 KB | $5.45e-6 | $0.55 | 1.4× |
| p50 396 ms, 20 KB | $8.92e-6 | $0.89 | 2.3× |

Breakdown at the p50/3.5 KB row: gateway $3.70e-6 · Lambda request $0.20e-6 ·
Lambda duration $3.30e-6 · transfer $0.30e-6.

### Fixed costs — do not scale with keys, and dwarf the marginal ones

| Item | Cost | Note |
|---|---|---|
| API GW dedicated cache, 0.5 GB | **$14.60 / month** | `apiGatewayCacheEnabled: true`, `CACHE_CLUSTER_SIZE = '0.5'` |
| ClickHouse allocation | **~€1.10 / month** | ~1 % of an AX102 at ~€110/mo — `docs/prices-api-hetzner-storage-estimate.md:46` |

The cache alone equals **~19 fully-drained keys per month**. We pay it whether
anyone signs up or not.

## Verdict for ADR 0010

**The proportionality argument survives, comfortably.** All-in is **1.4×–2.3×**
the $0.38 figure, not the 10× that would have made it worth revisiting. Worst
case in the table is $0.89 per fully-drained key.

Three corrections and one reframing fall out of this:

1. **$3.50/million is the us-east-1 rate.** We are in eu-central-1, where it is
   **$3.70**. [[0156]]'s figures carry the wrong region's price — a ~6 % error,
   immaterial to the conclusion but wrong in a document others will cite.
2. **Lambda duration, not gateway, is the largest marginal component** at the
   measured p50 — $3.30e-6 against $3.70e-6, near parity, and the whole reason
   all-in lands at 2× rather than 1.1×. "Gateway-only" was not a conservative
   simplification; it happened to omit a term of similar size.
3. **ClickHouse is negligible in dollars** — ~€1.10/month, on a box whose tier is
   driven by BE's data, not ours.
4. **The real exposure from a drained key is not money, it is capacity on a
   shared box.** `ch-prod-01` is shared with the BE team and the dollar cost of
   query load there is ~0 because the box is fixed-price. A dollar-denominated
   proportionality argument is therefore measuring the wrong risk — it is right
   about the money and silent about contention. Worth a sentence in ADR 0010.

## Caveat that bounds all of the above

**There is no representative production traffic: 16 invocations in 90 days, 11
in the last 30.** Those are verification curls, spread far enough apart that
essentially every one is a cold start. So the measured duration is an **upper
bound** for steady-state warm traffic — which is the safe direction for this
argument (it overstates cost, and cost is still small), but it is not a
steady-state number.

The conclusion is robust to it regardless: across the whole plausible duration
range, 150 ms to 496 ms, the drained-key cost moves only $0.55 → $0.83. Nothing
in that range threatens the ADR.

**Optional refinement, not needed for the verdict:** ~200 GETs at 1 req/s
against `v1/assets/{id}/ohlcv` with the CFN-managed key would give a real warm
p50 and a real response size. It costs ~0.2 % of one month's quota and queries
the shared `ch-prod-01`, so it is a decision, not a default.
