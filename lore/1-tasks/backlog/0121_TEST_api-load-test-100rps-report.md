---
id: "0121"
title: "Load test — 100 req/s sustained for 5 min on GET /assets/{id}/price, p95 <200ms, error rate <0.1%"
type: TEST
status: backlog
related_adr: ["0007", "0008"]
related_tasks: ["0047", "0120", "0122", "0128"]
tags: [layer-backend, layer-infra, priority-high, effort-medium, milestone-M2, api, performance, load-test, acceptance]
milestone: 2
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Owns Tranche 2
      acceptance criterion 2. Note the two different latency bars in the
      design doc — §6 targets p95 <100ms, but the T2 AC says <200ms; T3 AC 5
      tightens back to <100ms.
---

# Load test — 100 req/s on `GET /assets/{id}/price`

## Summary

Tranche 2 AC 2: *"Load test (k6 or Locust, script provided): 100 req/s sustained
for 5 minutes on `GET /assets/{id}/price` → p95 latency <200ms, error rate
<0.1%."*

Deliverable is a **script plus a report**, both citable by a reviewer.

## Context

**Two latency bars, deliberately.** §6 states the design target as *"<100ms p95
API response time"*, Tranche 2 AC 2 asks for **<200ms**, and Tranche 3 AC 5
tightens to **<100ms at 100 req/s**. The T2 bar is the looser one; do not treat
a 150ms p95 as a failure at this milestone, but do record the number so the M3
gap is known early rather than discovered in Tranche 3.

**Why `/price` is the chosen endpoint.** Task 0040 deliberately shipped `/price`
as a cheap point lookup against `current_prices`, and 0072 kept the expensive
derivations producer-side *specifically* to protect this SLO. The load test is
the check on that decision.

**What actually dominates.** The cross-cloud hop AWS → Hetzner is ~80–130ms RTT
(§6). A cache **miss** therefore spends most of the 200ms budget on one network
round trip. Consequences worth designing for:

- The result depends heavily on **cache hit rate**, so the test must report hit
  and miss percentiles separately. A p95 that is really "95% cache hits" says
  nothing about the data path.
- Hammering a single asset measures the API Gateway cache. Spreading across the
  20-asset set from [[0120]] measures the real path. **Do both**, and report
  both.
- Warm vs cold Lambda containers matter (connection reuse amortises the mTLS
  handshake, §5.2). Report cold-start incidence.

**⚠️ Shared-cluster caution.** The read path lands on BE's **shared** Hetzner
ClickHouse box. 100 req/s of cache misses is real load on infrastructure another
team depends on. Coordinate the window with BE, keep the run to the specified 5
minutes, and stop if BE-side alarms fire. This is the live question task 0047
was opened to answer.

## Implementation

- Write the load script (k6 preferred — single binary, scriptable thresholds,
  clean JSON/HTML output). Commit it to the repo so "script provided" is
  literally true.
- Scenarios:
  1. **Single hot asset**, 100 req/s × 5 min — upper bound, cache-dominated.
  2. **Spread across the 20-asset set**, 100 req/s × 5 min — the AC scenario.
  3. Short ramp to find the knee, kept well under the stage throttle
     (`apiGatewayThrottleRate: 200`, burst 400 in `envs/production.json`) so
     the test measures latency, not 429s.
- Instrument: p50/p90/p95/p99, error rate by status class, `X-Cache` hit ratio,
  Lambda duration + cold starts, ClickHouse query time, and API Gateway 4xx/5xx.
- Use a dedicated API key with its own usage plan so the test cannot exhaust the
  partner key's daily quota (`apiGatewayPartnerDailyQuota: 10000` — note that
  100 req/s × 5 min = **30,000 requests**, which *exceeds* that quota; a
  separate key or a raised quota is mandatory, not optional).
- Publish a report: test plan, environment, raw numbers, graphs, and an explicit
  statement of whether the AC passed.

## Acceptance Criteria

- [ ] k6 (or Locust) script committed to the repo and runnable from the README
- [ ] Dedicated load-test API key / usage plan provisioned — the partner key's
      10,000/day quota is **not** used and is not exhausted by the run
- [ ] 100 req/s sustained for 5 minutes on `GET /assets/{id}/price` completes
- [ ] p95 < 200ms and error rate < 0.1% on the 20-asset spread scenario
- [ ] Percentiles reported separately for cache hits and cache misses
- [ ] Cold-start incidence and ClickHouse-side query time recorded
- [ ] Report published under `docs/` with a pass/fail verdict, citable by
      [[0128]]
- [ ] Distance to the Tranche 3 bar (p95 <100ms) stated explicitly, with a
      recommendation if the gap is material
- [ ] BE notified before the run; no BE-side alarm fired, or the incident is
      recorded

## Notes

- If p95 fails only on cache misses, the levers in order of cost are: raise
  per-endpoint TTL (§6 / [[0122]]), Lambda provisioned concurrency (§10 lists it
  at ~+$45/mo), or move a hot column producer-side. Record which was chosen.
- A RED result here is also a signal for task **0047** (cross-tenant throughput
  on the shared box) and, in the extreme, for ADR 0007's sidecar-CH fallback.
