# Load test — `GET /assets/{id}/price` at 100 req/s

> **STATUS: DRAFT — the measured run has not happened yet.** Everything marked
> `TBD` waits on a BE-coordinated window (see _Why a window is needed_). The
> method, environment and pre-flight sections below are measured and final;
> filling this in on run day should be numbers, not prose.

Task [0121](../lore/1-tasks/active/0121_TEST_api-load-test-100rps-report.md) ·
Tranche 2 AC 2 · script: [`packages/prices-api/loadtest/`](../packages/prices-api/loadtest/README.md)

## The acceptance criterion

> _"Load test (k6 or Locust, script provided): 100 req/s sustained for 5 minutes
> on `GET /assets/{id}/price` → p95 latency <200ms, error rate <0.1%."_

Tranche 3 AC 5 later tightens the same measurement to **p95 <100 ms**, and §6 of
the design doc states the standing target as <100 ms. This report records the
distance to that bar as well as the pass/fail against the 200 ms one.

## Verdict

**TBD** — one line, `PASS` or `FAIL`, against p95 <200 ms and error rate <0.1 %
on the AC (20-asset) scenario, citing the k6 process exit code.

## Environment

|               |                                                                                                 |
| ------------- | ----------------------------------------------------------------------------------------------- |
| Endpoint      | `https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production`                          |
| Region        | `eu-central-1` · stage throttle 200 req/s, burst 400                                            |
| Gateway cache | enabled, TTL 10 s on `/price`, **key is the path only**                                         |
| Usage plan    | `prices-production-loadtest-plan` (`i12bsj`) — 150 req/s, burst 300, 1 M/month                  |
| API key       | `prices-production-loadtest-key-20260819T114230Z` ([registry](runbooks/manual-api-key-tier.md)) |
| Generator     | k6 v2.2.0, darwin/arm64, single host — TBD confirm on run day                                   |
| Run date      | TBD                                                                                             |

The plan sits under the stage ceiling, so 100 req/s needs no CDK change. **This
run does not use `pricing-api-free-production`** (1 req/s, 100 000/month), which
would have measured our own throttle rather than the system.

## Method — three regimes, and why the number is meaningless without one

There is **no `X-Cache` header** on this API, so hit and miss cannot be tagged
per request. The gateway caches on the **path only**, so no query parameter busts
it and the _pool size is the only lever on hit rate_. Over a 300 s run an asset
can miss at most 30 times, which fixes the arithmetic and forces three separate
runs rather than one number:

| #   | regime | pool                                                                                                     | max misses of 30 000 | what its p95 measures                  |
| --- | ------ | -------------------------------------------------------------------------------------------------------- | -------------------- | -------------------------------------- |
| 1   | cache  | 1 (`native`)                                                                                             | ~30 (0.1 %)          | the API Gateway cache                  |
| 2   | **AC** | 20 (conformance list, [0120](../lore/1-tasks/blocked/0120_TEST_endpoint-conformance-20-major-assets.md)) | ~600 (2 %)           | the AC scenario, still cache-dominated |
| 3   | wide   | 3504                                                                                                     | 30 000 (100 %)       | the real data path, worst case         |

Regime 3 needs `pool ≫ RATE × TTL`. Selection is deterministic round-robin, so a
pool of exactly 1000 at 100 req/s against the 10 s TTL returns to each asset on
the expiry boundary and measures a coin-flip; 3504 gives 3.5× margin.

Uniform sampling never warms a hot key the way real traffic does, so regime 3 is
a **worst case, not a typical one**, and is reported as such.

Each run: 30 s warmup at full rate (excluded from thresholds, so Lambda cold
starts do not pollute p95 over 30 000 samples), then 5 min measured as
`phase:main`.

## Pre-flight — completed 2026-09-02

Done ahead of the window so it does not consume it:

- **Asset pool built and verified.** Production listed **3543** assets; a seeded
  200-asset random sample returned `200` on `/price` for every id. `setup()`
  then probed all 3543 and dropped **39** on 404 (no price row), naming each,
  leaving **3504** under test.
- **Pool generator bug found and fixed.** The listing returns native XLM with
  _both_ `contract_address` and `issuer_address` empty, so the documented
  `code:issuer` expression emitted `XLM:` — which the API answers **400**, not 404. `setup()` aborts the run on any non-404 (deliberately: a throttled key
  must not read as dead assets), so the wide regime would have failed at setup,
  inside the window, with a status pointing at the API rather than the pool file.
- **Script validated end-to-end** on the wide pool: all four thresholds
  (`http_req_duration`, `http_req_failed`, `checks`, `dropped_iterations`, all
  scoped to `phase:main`) evaluated and passed.
- **Setup timing measured.** 59 s at `PROBE_BATCH=25` against the 180 s
  `SETUP_TIMEOUT`; the default batch of 10 extrapolates to ~150 s, too little
  margin for a coordinated window.
- **Reporting flags fixed.** `--summary-trend-stats` is required for p50/p99 —
  k6's defaults stop at p95. In `--summary-export` JSON a threshold entry of
  `false` means _not breached_, i.e. passed; the **process exit code** is cited
  instead, since it cannot be read backwards.

## Results

All figures from `phase:main` only. `TBD` until the run.

### Latency

| regime                 | p50 | p90 | p95     | p99 | max | requests |
| ---------------------- | --- | --- | ------- | --- | --- | -------- |
| 1 — cache (1 asset)    | TBD | TBD | TBD     | TBD | TBD | TBD      |
| 2 — **AC (20 assets)** | TBD | TBD | **TBD** | TBD | TBD | TBD      |
| 3 — wide (3504 assets) | TBD | TBD | TBD     | TBD | TBD | TBD      |

Regimes 1 and 3 bracket the cache-hit and cache-miss percentiles that the AC asks
to be reported separately; regime 2 is the mixture the AC actually specifies.

### Rate sustained and errors

| regime | offered | achieved | `dropped_iterations` | non-200 | error rate | exit |
| ------ | ------- | -------- | -------------------- | ------- | ---------- | ---- |
| 1      | 100/s   | TBD      | TBD                  | TBD     | TBD        | TBD  |
| 2      | 100/s   | TBD      | TBD                  | TBD     | TBD        | TBD  |
| 3      | 100/s   | TBD      | TBD                  | TBD     | TBD        | TBD  |

`dropped_iterations` is a gate, not a statistic: any drop means k6 could not hold
the offered rate, so the run did **not** sustain 100 req/s and its p95 is not the
AC's number.

**Error-rate argument.** With 30 000 requests and zero errors, the rule of three
puts the 95 % upper bound at 3/30 000 = **0.01 %** — a 10× margin under the
0.1 % AC. This holds only because all 30 000 samples are on the one endpoint
under test.

### Cold starts and ClickHouse-side time

**BLOCKED — not obtainable with current credentials.** These numbers live in
CloudWatch in the account that hosts the production stack (Lambda
`InitDuration` / `ConcurrentExecutions`, API Gateway `Latency` vs
`IntegrationLatency` over the `phase:main` window; the `Latency −
IntegrationLatency` split is what makes a "where did the p95 go" table
writable). The `rumblefish-admin` / `rumblefish-readonly` profiles
(045028348791) return zero API Gateway, Lambda and CloudWatch resources in
`eu-central-1`, so the production stack is not in that account.

Needs read access to the prod account before this section can be filled, or
someone who has it to pull the window. See _Open items_.

|                                                |     |
| ---------------------------------------------- | --- |
| Cold starts in `phase:main`                    | TBD |
| Max concurrent executions                      | TBD |
| Gateway `Latency` − `IntegrationLatency` (p95) | TBD |
| ClickHouse query time                          | TBD |

## Distance to the Tranche 3 bar (p95 <100 ms)

TBD — state the gap per regime. If regime 3 clears 200 ms but not 100 ms, the
levers in ascending cost are: raise the per-endpoint TTL (§6 /
[0122](../lore/1-tasks/backlog/0122_TEST_apigateway-cache-ttl-verification.md)), Lambda provisioned concurrency (~+$45/mo per §10), or
move a hot column producer-side. Record which was chosen and why.

## Why a window is needed

The read path lands on the ClickHouse box **shared with soroban-block-explorer**.
Regime 3 is 30 000 cache misses in five minutes — real load on infrastructure
another team depends on, and the live question [0047](../lore/1-tasks/backlog/0047_RESEARCH_cross-tenant-throughput-verification-on-shared-hetzner-ch.md) was
opened to answer. Two rules:

1. Check the box is quiet before **and** after. A contaminated run cannot be
   corrected after the fact — discard it, re-run, and name the discarded run
   here rather than dropping it quietly.
2. Schedule away from our own OHLCV batch, and tell BE before starting.

|                                |     |
| ------------------------------ | --- |
| BE notified                    | TBD |
| Window                         | TBD |
| BE-side alarms during the run  | TBD |
| Runs discarded as contaminated | TBD |

## Open items

- [ ] BE window agreed
- [ ] Three regimes run, `--summary-export` JSON archived alongside this report
- [ ] CloudWatch access to the production account obtained, or the window pulled
      by someone who has it
- [ ] Verdict written, and this report cited by
      [0128](../lore/1-tasks/backlog/0128_DOCS_scf-milestone-2-verification-package.md)
