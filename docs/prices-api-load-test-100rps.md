# Load test — `GET /assets/{id}/price` at 100 req/s

> **STATUS: all three regimes run on 2026-09-03. The AC scenario PASSED — and
> regime 3 took the production read path down.** Both results are real and
> neither cancels the other: the acceptance criterion is written against the
> 20-asset scenario, which passed with ~4× margin, while the wide-pool regime
> that actually reaches the database failed 94.38 % of its requests and left the
> API returning `500` to single requests. See _The outage_ before quoting the
> PASS anywhere.

Task [0121](../lore/1-tasks/active/0121_TEST_api-load-test-100rps-report.md) ·
Tranche 2 AC 2 · script: [`packages/prices-api/loadtest/`](../packages/prices-api/loadtest/README.md)

## The acceptance criterion

> _"Load test (k6 or Locust, script provided): 100 req/s sustained for 5 minutes
> on `GET /assets/{id}/price` → p95 latency <200ms, error rate <0.1%."_

Tranche 3 AC 5 later tightens the same measurement to **p95 <100 ms**, and §6 of
the design doc states the standing target as <100 ms. This report records the
distance to that bar as well as the pass/fail against the 200 ms one.

## Verdict

**PASS** on the acceptance criterion.

The AC scenario (regime 2, the 20-asset spread) sustained 99.85 req/s for
5 minutes with **p95 = 47.09 ms** against the 200 ms bar and **zero errors in
30 001 requests** against the 0.1 % bar. k6 exited **0**, so all four thresholds
held.

**This PASS is narrow, and quoting it without the next sentence would be
misleading.** The criterion is met on the scenario it names. On the wide-pool
regime — the only one whose requests actually reach ClickHouse rather than the
gateway cache — the same 100 req/s failed 94.38 % of requests and took the read
path down for every consumer, not just the test. A reviewer signing off Tranche 2
on this report is entitled to the pass; anyone reading it as "the API handles
100 req/s" is reading it wrong.

Two AC checklist items remain open: cache-**miss** percentiles were not obtained
(the system stopped serving misses before they could be measured), and
cold-start / ClickHouse-side timing needs credentials this machine does not have.

## Environment

|               |                                                                                                 |
| ------------- | ----------------------------------------------------------------------------------------------- |
| Endpoint      | `https://prices-api.sorobanscan.rumblefish.dev`                                                 |
| Region        | `eu-central-1` · stage throttle 200 req/s, burst 400                                            |
| Gateway cache | enabled, TTL 10 s on `/price`, **key is the path only**                                         |
| Usage plan    | `prices-production-loadtest-plan` (`i12bsj`) — 150 req/s, burst 300, 1 M/month                  |
| API key       | `prices-production-loadtest-key-20260819T114230Z` ([registry](runbooks/manual-api-key-tier.md)) |
| Generator     | k6 v2.2.0, darwin/arm64, single host                                                            |
| Run date      | 2026-09-03, 06:04–06:16 UTC                                                                     |

⚠️ **The endpoint changed under this task the day before the run.**
Task 0126 retired the `execute-api` URL
(`a635439`, merged 2026-09-02 14:56 CEST) in favour of the custom domain above.
The pre-flight smoke ran at 14:28 the same day — 28 minutes before the merge —
so the first attempt on run day aborted at setup with `native → 403`, which is
API Gateway's response to an unknown key and is byte-identical to the response
for no key at all. Worth recording because it is the same indistinguishability
task 0255 is open on: nothing in the
response separates "your key is wrong" from "this endpoint is gone".

**Usage-plan membership was verified empirically, not from the console.** The
prod account is not reachable from here, so instead: 60 requests issued at
~52 req/s returned 60 × `200` and zero `429`. That rules out
`pricing-api-free-production` (1 req/s, burst 5) decisively — a key on it would
have thrown 429s immediately. The run therefore measures the system, not our own
throttle.

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

All figures from `phase:main` only, in milliseconds. Raw k6 exports, which every
number below is read from:
[`loadtest-results/2026-09-03-regime1-cache.json`](loadtest-results/2026-09-03-regime1-cache.json)
and
[`loadtest-results/2026-09-03-regime2-ac.json`](loadtest-results/2026-09-03-regime2-ac.json).

### Latency

| regime                 | p50   | p90   | p95       | p99    | max    | requests |
| ---------------------- | ----- | ----- | --------- | ------ | ------ | -------- |
| 1 — cache (1 asset)    | 45.15 | 46.51 | 47.75     | 175.95 | 930.13 | 29 995   |
| 2 — **AC (18 assets)** | 45.04 | 46.31 | **47.09** | 107.53 | 246.20 | 30 001   |
| 3 — wide (4301 assets) | 64.97 | 69.85 | _76.91_   | 183.16 | 456.75 | 30 001   |

🚫 **Regime 3's latency row is not a latency measurement and must not be quoted
as one.** 94.38 % of those requests returned `500`, and an error response is
cheap to produce, so those percentiles largely describe how fast the API failed.
The honest reading of regime 3 is in _The outage_ below, not in this table. It is
printed only so the raw export and this report agree.

Regimes 1 and 3 were meant to bracket the cache-hit and cache-miss percentiles
the AC asks to be reported separately. **That bracket does not exist**: the
cache-miss side could not be measured, because the system stopped serving cache
misses before it could be. Regime 2 is the mixture the AC actually specifies and
is unaffected.

**Regimes 1 and 2 are statistically indistinguishable at p95** — 47.75 vs
47.09 ms — which is the expected result and worth stating plainly: at a 20-asset
pool only ~2 % of requests can miss the cache, so regime 2's p95 is still a
measurement of the gateway cache, not of the data path. The AC is written against
that mixture, so it passes on the number above; but **no cache-miss percentile
has been measured yet**, and regime 3 is the only run that will produce one.

The divergence shows up in the tail, where the misses actually live: p99 is
175.95 ms for one hot asset versus 107.53 ms for twenty. The single-asset regime
has the _worse_ tail because its ~30 misses all queue behind one cache key and
one warming container, while twenty assets spread the same miss count across more
of the Lambda pool. Regime 1's 930 ms max is a cold start.

**The 20-asset pool ran as 18.** `setup()` dropped `AUD` and `EQL` on 404. This
is a different pair from the pre-flight two days earlier, which dropped `USDC`
and `RON` — so the unservable set **drifts day to day** and is a data-freshness
property, not a fixed defect list. Relevant to 0178 and 0210; the AC's 0.1 %
error bar would be unreachable if these were counted, which is exactly why
`setup()` excludes them before measurement rather than during it.

### Rate sustained and errors

| regime | offered | achieved    | `dropped_iterations{phase:main}` | non-200    | error rate  | exit   |
| ------ | ------- | ----------- | -------------------------------- | ---------- | ----------- | ------ |
| 1      | 100/s   | 99.87 req/s | 0                                | 0          | **0.00 %**  | **0**  |
| 2      | 100/s   | 99.85 req/s | 0                                | 0          | **0.00 %**  | **0**  |
| 3      | 100/s   | 94.28 req/s | 0                                | **28 314** | **94.38 %** | **99** |

`dropped_iterations` is a gate, not a statistic: any drop means k6 could not hold
the offered rate, so the run did **not** sustain 100 req/s and its p95 is not the
AC's number.

**Regime 1 dropped 6 iterations — all of them in `phase:warmup`, none in
`phase:main`.** Unscoped, that global `count<1` would have failed the run and
reported "did not sustain 100 req/s" for what was containers scaling during the
phase whose entire purpose is to scale containers. The threshold is scoped to
`phase:main` for exactly this reason (PR #234 review), and this run is the first
live case of it mattering.

**Error-rate argument.** 30 001 requests, zero errors. The rule of three puts the
95 % upper bound at 3/30 001 = **0.01 %** — a 10× margin under the 0.1 % AC. This
holds only because all 30 001 samples are on the one endpoint under test.

### The outage — regime 3 took the read path down

**Regime 3 did not measure the data path. It exhausted it.** This is the most
important result in this report, and it is a harder finding than the AC pass.

Raw export:
[`loadtest-results/2026-09-03-regime3-wide-FAILED.json`](loadtest-results/2026-09-03-regime3-wide-FAILED.json) ·
liveness probe:
[`loadtest-results/2026-09-03-outage-recovery-probe.log`](loadtest-results/2026-09-03-outage-recovery-probe.log)

| time (UTC)      | phase                         | offered    | result                                    |
| --------------- | ----------------------------- | ---------- | ----------------------------------------- |
| 06:32:17        | run starts                    | —          | —                                         |
| 06:32:17–33:23  | `setup()` probe, 4306 assets  | ~65 /s     | **clean** — 4301 × `200`, 5 × `404`       |
| ~06:33:23–34    | warmup, excluded              | 100 /s     | —                                         |
| ~06:34–06:38:54 | `phase:main`, 30 001 requests | 100 /s     | **28 314 failed (94.38 %)**, 1 687 served |
| ~06:40          | single sequential request     | 1 req      | `500`                                     |
| 06:43:53        | liveness probe starts, 1/30 s | 2 req/30 s | `500` on `/price` **and** `/v1/assets`    |
| 06:51:32        | 16th check                    | —          | still `500`                               |
| 06:57:54        | last confirmed failure        | 1 req      | `500`                                     |
| 07:25:32        | first confirmed recovery      | 1 req      | `200` on both routes                      |
| ~07:26          | miss path re-checked          | 5 req      | 5 × `200`, cold assets, 163–238 ms        |

**Outage duration: at least 19 minutes, at most 47.** It cannot be pinned closer,
and the reason is a process failure worth owning: the 30-second liveness probe
died after its 16th check and the restart failed silently, leaving no observation
between 06:57:54 and 07:25:32. The recovery was unattended and its exact moment
is unknown. Nothing was changed on our side in that gap and no load was applied,
so the system either recovered on its own or was recovered from the BE side.

Recovery is genuine, not a flicker: five distinct **cold** assets — cache misses,
the exact path that collapsed — returned `200` in 163–238 ms at ~0.3 req/s.

⚠️ **Those recovery numbers matter on their own.** A cache miss costs
**~170–240 ms with no contention at all**. That is already at the Tranche 2
200 ms bar and roughly double the Tranche 3 100 ms one, before a single
concurrent request is added. It is the clearest evidence in this report that the
wide-pool regime was never going to produce a passing p95 even had the database
survived it — and it reframes the AC pass, which rests entirely on the gateway
cache hiding a data path that is inherently too slow for the T3 target.

Errors are `500 {"code":"db_error","message":"price lookup failed"}`, and the
listing endpoint returns `500 {"code":"db_error","message":"asset list failed"}`
— so **the whole ClickHouse read path is down, not one route**, and not only
under load: a single request with no concurrency behind it fails the same way.

**What separates regime 3 from the two that passed.** Regimes 1 and 2 also
offered 100 req/s and were clean. The difference is not rate, it is _what
reached the database_: with a 1- and 18-asset pool against a 10 s cache TTL,
~98–99.9 % of those requests were served by the API Gateway cache and never
touched ClickHouse. Regime 3's 4301-asset pool defeats that cache by
construction, so it is the **first run in which 100 req/s actually arrived at the
database**. It did not survive it.

**Hypothesis, offered as a hypothesis.** The failures return _fast_ — p50 65 ms,
max 457 ms — rather than timing out. A database saturated on query execution
produces slow responses and timeouts; immediate `500`s point instead at the
connection layer: a `max_connections` ceiling, an exhausted client pool, or mTLS
session setup failing under concurrency. This is a direction for whoever has
access to check, not a conclusion — it cannot be confirmed from here (see the
credentials note below).

**Why this matters beyond a failed run.** The remediation levers this task listed
in advance — raising the per-endpoint TTL, Lambda provisioned concurrency, moving
a hot column producer-side — all reduce _latency_ or _miss rate_. If the ceiling
is connection-count rather than query performance, **none of them addresses it**;
they only hide it behind a higher cache hit rate. That distinction should be
settled before any of the three is chosen. It is squarely the question
[0047](../lore/1-tasks/backlog/0047_RESEARCH_cross-tenant-throughput-verification-on-shared-hetzner-ch.md)
exists to answer, and in the extreme it is an argument for ADR 0007's sidecar
ClickHouse fallback.

**Coordination.** The window was agreed with BE beforehand and the run was kept
to the specified duration. Load was stopped immediately on detection and has not
been resumed; the only traffic since is one liveness request per 30 s, which is
not load. BE were told during the outage, with the timeline above.

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

**On the measured regimes there is no gap — the T3 bar is already met at p95.**
Regime 2's 47.09 ms sits at less than half the 100 ms target, and regime 1 agrees
at 47.75 ms. No lever needs pulling for Tranche 3 on this evidence.

Two honest qualifications, because this number is easy to over-claim:

1. **It is a cache-dominated number.** Both passing regimes hit the gateway cache
   on ~98–99.9 % of requests. Tranche 3 AC 5 says "p95 <100 ms at 100 req/s"
   without qualifying the cache state; read as the data path, the answer is not
   47 ms — it is that the data path did not stay up at that rate.
2. **p99 already exceeds 100 ms** in both (175.95 and 107.53 ms). The T3 bar is
   written at p95 so this does not breach it, but the miss path and cold starts
   already land above 100 ms at a ~2 % miss rate.

**The plan for closing a T3 gap needs revisiting before it is used.** This report
previously assumed the gap would be a latency gap, to be closed by raising the
per-endpoint TTL (§6 /
[0122](../lore/1-tasks/backlog/0122_TEST_apigateway-cache-ttl-verification.md)),
Lambda provisioned concurrency (~+$45/mo per §10), or moving a hot column
producer-side. Regime 3 suggests the binding constraint may not be latency at
all. Two of those three levers work by _avoiding_ the database rather than making
it cope — which raises the measured p95 while leaving the ceiling exactly where
it is, and moves the failure to whenever the cache hit rate drops. Establish
whether the ceiling is connections or query performance first; the answer decides
whether any of these levers is even relevant.

## Why a window is needed

The read path lands on the ClickHouse box **shared with soroban-block-explorer**.
Regime 3 is 30 000 cache misses in five minutes — real load on infrastructure
another team depends on, and the live question [0047](../lore/1-tasks/backlog/0047_RESEARCH_cross-tenant-throughput-verification-on-shared-hetzner-ch.md) was
opened to answer. Two rules:

1. Check the box is quiet before **and** after. A contaminated run cannot be
   corrected after the fact — discard it, re-run, and name the discarded run
   here rather than dropping it quietly.
2. Schedule away from our own OHLCV batch, and tell BE before starting.

**Regimes 1 and 2 did not need the window and were run without one.** Between
them they put roughly 1 200 queries on the shared box across eleven minutes —
under 2 req/s, below that machine's noise floor. Regime 3 alone was ~37 300
requests at a sustained 100 req/s, and it was the only run BE had to be told
about.

**The caution in this section turned out to be the correct call, and still
understated the risk.** It was written expecting contention — a run that degrades
a neighbour's p95 and has to be discarded. What actually happened is that the run
took the shared read path down outright, for our own API as well. Anyone
repeating regime 3 should treat it as a **potentially destructive test of the
database tier**, not as a latency measurement that happens to be noisy, and
should agree an abort signal and an owner who can see the box _before_ starting —
neither of which existed this time.

|                                | regimes 1 & 2 (2026-09-03)                 | regime 3 (2026-09-03)                            |
| ------------------------------ | ------------------------------------------ | ------------------------------------------------ |
| BE notified                    | not required — see above                   | **yes**, before the run; again during the outage |
| Window                         | none                                       | agreed, 06:32–06:39 UTC, kept to the 5 min       |
| BE-side alarms                 | none observed on our side; not BE-verified | not visible from here — no prod-account access   |
| Runs discarded as contaminated | none                                       | none — the run is reported, not discarded        |
| Outage caused                  | none                                       | **yes** — read path down from ~06:38, see above  |

**Gap between regimes: 46 s** — regime 1 ended 06:09:37 UTC, regime 2 started
06:10:23 UTC. At 4.6× the 10 s cache TTL the gateway cache was cold for every
asset, which is the only thing the gap has to achieve.

The run-day procedure said ≥ 60 s at the time, and that figure was wrong on its
own terms: it justified itself partly by not carrying the warm Lambda pool into
the next run, which no gap of that order can do — containers survive 5–15
minutes. Container state is normalised by each run's own 30 s full-rate warmup,
not by the gap. The README is corrected to ≥ 30 s with the real reason. Recorded
here because every number in this report should be explainable, and "46 s where
the procedure said 60 s" is only worth reading once you know the 60 s was not
load-bearing.

## Open items

Ordered by urgency, not by AC order.

- [x] **Read path restored** — recovered unattended between 06:57:54 and
      07:25:32 UTC, verified on the miss path. Duration 19–47 min; see the
      outage timeline for why it is a range.
- [ ] **Root cause established: connection ceiling or query performance?** The
      whole remediation plan depends on the answer, and nothing above settles
      it. Needs someone with access to the ClickHouse box and to CloudWatch.
- [ ] **Decide whether regime 3 is ever repeated**, and under what protocol —
      it is now known to be capable of taking down shared infrastructure. If it
      is repeated, agree an abort signal and an observer with eyes on the box.
- [x] All three regimes run; every `--summary-export` archived under
      `loadtest-results/`, the failed one included
- [x] Verdict written — **PASS** on the AC, with the outage stated alongside it
- [ ] Cache-**miss** percentiles — **not obtainable by this method**. The system
      stops serving before a miss percentile can be sampled at 100 req/s. A
      lower-rate miss measurement would answer a different question and should be
      scoped deliberately rather than treated as a retry.
- [ ] CloudWatch access to the production account obtained
- [ ] Report cited by
      [0128](../lore/1-tasks/backlog/0128_DOCS_scf-milestone-2-verification-package.md)
