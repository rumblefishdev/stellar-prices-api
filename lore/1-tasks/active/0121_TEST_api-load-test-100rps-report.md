---
id: "0121"
title: "Load test — 100 req/s sustained for 5 min on GET /assets/{id}/price, p95 <200ms, error rate <0.1%"
type: TEST
status: active
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
  - date: 2026-08-19
    status: active
    who: stkrolikiewicz
    note: >
      Promoted to active — starting the load-test pass. Uses the 20-asset
      list fixed by [[0120]] (PR #226) for the spread scenario.
  - date: 2026-08-20
    status: active
    who: stkrolikiewicz
    note: >
      Script and plan done; the run itself still needs a BE window. Plan
      `prices-production-loadtest-plan` (`i12bsj`, 150 req/s / burst 300 /
      1M mies.) + key issued and **registered** in
      `docs/runbooks/manual-api-key-tier.md` — note it sits under the stage
      ceiling (200/400), so unlike soroban-block-explorer's harness we need
      no infra flag to lift a throttle. `price_load.js` extended rather than
      rewritten: warmup phase excluded from thresholds (Lambda cold starts
      would otherwise pollute p95 over 30k samples), asset pool from 0120's
      list, `X-Request-Id` for a CH `log_comment` join, non-200 counted as
      failure, and `dropped_iterations` as a threshold so a run that failed
      to sustain the rate cannot be reported as one that did.
      **Three measured findings shaping the method:** (1) there is NO
      `X-Cache` header, so hit/miss percentiles cannot be tagged per
      request — and the gateway cache key is the PATH ONLY, so pool size is
      the only lever on hit rate; over 300 s an asset misses at most 30
      times, making 1/20/1000+ assets the ~0 %/2 %/100 % miss regimes.
      Report each number with its regime. (2) Canonical USDC answers 404
      ([[0178]]), which puts a permanent 5 % floor under the error rate with
      the 20-asset pool — the AC's 0.1 % would be unreachable for a reason
      that has nothing to do with load. `setup()` now probes the pool, drops
      unservable assets and names them, so the exclusion lands in the report
      instead of vanishing. (3) Smoke run against prod (3 req/s, 19 assets,
      mostly cache misses): p95 **83 ms**, zero failures — an early signal
      that the 200 ms AC is comfortable, though not a substitute for the
      5-minute run.
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
- Use a dedicated API key with its own usage plan. **Updated by [[0157]]:** there
  is now exactly one CDK-managed plan, `pricing-api-free-production`, at **1 req/s
  with a 100 000/month quota**. Both limits make it unusable here — the rate alone
  means the run measures our own throttle rather than the system, and
  100 req/s × 5 min = **30,000 requests** would spend nearly a third of the
  month's allowance per run. A separate plan sized for the run is mandatory, not
  optional; `docs/runbooks/manual-api-key-tier.md` is the procedure.
- Publish a report: test plan, environment, raw numbers, graphs, and an explicit
  statement of whether the AC passed.

## Acceptance Criteria

- [ ] k6 (or Locust) script committed to the repo and runnable from the README
- [ ] Dedicated load-test API key / usage plan provisioned, sized above 100 req/s
      — the run does **not** use a key on `pricing-api-free-production` (1 req/s,
      100 000/month), which would measure our own throttle rather than the system
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
