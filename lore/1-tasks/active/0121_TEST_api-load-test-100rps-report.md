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
      list, non-200 counted as
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
  - date: 2026-08-28
    status: active
    who: stkrolikiewicz
    note: >
      Addressed all ten findings from okarcz's PR #234 review. Three of them
      shared one theme — the harness could report the SLO as met without
      being able to know it: no `checks` threshold (k6 checks do not affect
      the exit code, so the body validation was decorative), the
      authoritative README command still pinning `-e ASSET=native` (which
      measures the gateway cache), and a sequential `setup()` probe that
      aborts the run before a measured request in the wide pool the README
      recommends. Also: non-404 probe failures now abort naming the status
      instead of reading a throttled key as dead assets, `dropped_iterations`
      scoped to `phase:main`, `WARMUP=0` drops the phase (`0s` was rejected
      at config validation — verified against the old script), warmup at full
      rate, `SharedArray`, the wide-regime threshold restated as
      `pool >> RATE x TTL`, and the `X-Request-Id` header removed along with
      the claim of a ClickHouse `log_comment` join that does not exist.
      Branch rebased onto develop first (it was 217 commits behind). Smoke
      run on prod: 2/20 assets dropped on 404 (USDC per [[0178]], RON), all
      four thresholds evaluated, p95 55.68 ms in `phase:main` against 670 ms
      unscoped. The `X-Request-Id` claim is also removed from the note below.
  - date: 2026-09-02
    status: active
    who: stkrolikiewicz
    note: >
      Pre-flight pass — everything that does not need the BE window is done, so
      the window is spent measuring rather than debugging. Report skeleton
      published at `docs/prices-api-load-test-100rps.md` (method, environment
      and pre-flight final; every number `TBD`). Wide pool built: prod lists
      **3543** assets, `setup()` drops **39** on 404 and names them, **3504**
      under test — 3.5x the `RATE x TTL` margin, so the wide regime is viable
      without hand-curation. Gitignored as a prod snapshot, not a fixture.
      **One bug found that would have killed the run inside the window:** the
      README's pool generator emitted `XLM:` for native XLM — the listing
      returns it with *both* `contract_address` and `issuer_address` empty — and
      the API answers that **400**, not 404, so `setup()`'s deliberate
      abort-on-non-404 would have aborted the whole run pointing at the API
      rather than at the pool file. Generator fixed and the trap documented.
      Also measured, not guessed: setup on 3543 assets takes **59 s** at
      `PROBE_BATCH=25` (the default 10 extrapolates to ~150 s against a 180 s
      timeout — too little margin), `ASSETS` resolves relative to the *script*
      not the shell cwd, `--summary-trend-stats` is required for the p50/p99 the
      AC asks for (k6 defaults stop at p95), and in `--summary-export` JSON a
      threshold of `false` means *passed* — so the report cites the exit code.
      Script re-validated end-to-end on the full wide pool: all four thresholds
      evaluated and passed. **New blocker for two AC items:** cold-start
      incidence and ClickHouse-side query time live in CloudWatch in the prod
      account, which is not reachable from the profiles available here —
      `rumblefish-admin`/`rumblefish-readonly` (045028348791) return zero
      APIGateway/Lambda/CloudWatch resources in eu-central-1. The load run
      itself is unaffected (public endpoint + API key).
  - date: 2026-09-03
    status: active
    who: stkrolikiewicz
    note: >
      **Regimes 1 and 2 run; the AC PASSES.** Regime 2 (the AC scenario)
      sustained 99.85 req/s for 5 min at **p95 47.09 ms** against the 200 ms
      bar with **0 errors in 30 001 requests**; k6 exit 0 on all four
      thresholds. Regime 1 (single hot asset): p95 47.75 ms, 0 errors. Both are
      also under the Tranche 3 100 ms bar at p95 — but both are
      cache-dominated, so that is not yet an answer for T3, and p99 already
      exceeds 100 ms (175.95 / 107.53 ms). Report filled in at
      `docs/prices-api-load-test-100rps.md`, raw k6 exports archived under
      `docs/loadtest-results/`.
      Regimes 1 and 2 were run **without** a BE window and did not need one:
      together ~1200 ClickHouse queries over eleven minutes, under 2 req/s.
      Only regime 3 (~36 500 queries at a sustained 100 req/s) needs
      coordination; window requested, awaiting BE.
      **The endpoint moved under us.** [[0126]] retired the `execute-api` URL
      in `a635439`, merged 2026-09-02 14:56 CEST — 28 minutes after the
      pre-flight smoke. The first attempt today aborted at setup with
      `native → 403`, which API Gateway returns byte-identically for an unknown
      key and for no key at all, so it reads as "bad key" rather than "dead
      endpoint" — the same indistinguishability [[0255]] is open on. Fixed by
      pointing at `https://prices-api.sorobanscan.rumblefish.dev`; README and
      report both updated. Plan membership re-verified empirically since the
      console is unreachable: 60 requests at ~52 req/s, all 200, zero 429,
      which rules out the 1 req/s free plan.
      Two findings worth carrying: (1) regime 1 dropped 6 iterations, **all in
      warmup, none in main** — the first live case of Oskar's `phase:main`
      scoping mattering; unscoped it would have reported "did not sustain
      100 req/s". (2) The 20-asset pool ran as **18**: `setup()` dropped `AUD`
      and `EQL` on 404, a different pair from the pre-flight's `USDC`/`RON`, so
      the unservable set **drifts day to day** — a data-freshness property, not
      a fixed defect list. Relevant to [[0178]] and [[0210]].
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

- [x] k6 (or Locust) script committed to the repo and runnable from the README
- [x] Dedicated load-test API key / usage plan provisioned, sized above 100 req/s
      — the run does **not** use a key on `pricing-api-free-production` (1 req/s,
      100 000/month), which would measure our own throttle rather than the system
- [x] 100 req/s sustained for 5 minutes on `GET /assets/{id}/price` completes
      — 99.87 and 99.85 req/s achieved on regimes 1 and 2, zero
      `dropped_iterations{phase:main}`
- [x] p95 < 200ms and error rate < 0.1% on the 20-asset spread scenario
      — **p95 47.09 ms, 0 errors in 30 001 requests**, k6 exit 0
- [ ] Percentiles reported separately for cache hits and cache misses
      — hits measured (regimes 1 and 2); **misses need regime 3**, pending the
      BE window. There is no `X-Cache` header, so this cannot be split within a
      single run
- [ ] Cold-start incidence and ClickHouse-side query time recorded
      — blocked on prod-account CloudWatch access, not on the run
- [x] Report published under `docs/` with a pass/fail verdict, citable by
      [[0128]] — `docs/prices-api-load-test-100rps.md`, verdict **PASS**, raw
      k6 exports archived under `docs/loadtest-results/`
- [x] Distance to the Tranche 3 bar (p95 <100ms) stated explicitly, with a
      recommendation if the gap is material — no gap at p95 on the measured
      regimes (47.09 ms vs the 100 ms bar), qualified as cache-dominated; p99
      already exceeds 100 ms
- [ ] BE notified before the run; no BE-side alarm fired, or the incident is
      recorded — window requested 2026-09-03, awaiting reply. Regimes 1 and 2
      did not need one (~1 200 queries total, under 2 req/s)

## Notes

- If p95 fails only on cache misses, the levers in order of cost are: raise
  per-endpoint TTL (§6 / [[0122]]), Lambda provisioned concurrency (§10 lists it
  at ~+$45/mo), or move a hot column producer-side. Record which was chosen.
- A RED result here is also a signal for task **0047** (cross-tenant throughput
  on the shared box) and, in the extreme, for ADR 0007's sidecar-CH fallback.
