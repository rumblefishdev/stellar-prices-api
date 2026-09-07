---
margin:
  x: 1.5cm
  y: 1.5cm
---

# Stellar Prices API — Milestone 2 Deliverable Evidence

> - **Project:** Stellar Prices API
> - **Team:** Rumble Fish
>
> This document is the written companion to the Milestone 2 submission video. It
> maps every acceptance criterion from §9 of the technical design
> (_"Tranche 2 — Public API"_) to concrete evidence against the deployed
> production API: live URLs, runnable commands with their output, SQL a reviewer
> can execute, and code references.
>
> **Everything here is reproducible against the public deployment.** The API base
> is `https://prices-api.sorobanscan.rumblefish.dev`, a REGIONAL custom domain
> mapped at the root, so there is no stage path in any URL. Key-gated routes need
> an `x-api-key` header; the key is available on request via the address on the
> submission form.
>
> **Two acceptance criteria are graded against amended wording, and both
> amendments are declared rather than assumed.** AC 2's target is unchanged but
> no key in the account can sustain it; AC 3 names a response header this
> platform does not emit. Each is set out in full in
> [`milestone-2-rfp-deviations.md`](milestone-2-rfp-deviations.md), which is part
> of this submission and should be read alongside this document.
>
> This project runs as a second tenant on infrastructure the **Soroban Block
> Explorer** already operates — a shared AWS sub-account and a shared Hetzner
> ClickHouse cluster. The Soroban Block Explorer is abbreviated **SBE** below.
>
> Screenshot placeholders in this source are replaced with inline evidence images
> in the published PDF.

## Table of contents

1. [Executive summary](#1-executive-summary)
2. [Deliverable definition](#2-deliverable-definition)
3. [Architecture](#3-architecture)
4. [Deviations from the approved wording](#4-deviations-from-the-approved-wording)
5. [Acceptance-criteria evidence](#5-acceptance-criteria-evidence)
6. [Work items without a numbered criterion](#6-work-items-without-a-numbered-criterion)
7. [Milestone 1 deferrals, now delivered](#7-milestone-1-deferrals-now-delivered)
8. [What is deliberately not claimed](#8-what-is-deliberately-not-claimed)
9. [Live endpoints and access](#9-live-endpoints-and-access)
10. [Repository navigation](#10-repository-navigation)

## 2. Deliverable definition

§9 of the technical design defines Tranche 2 as **Public API**, weeks 5 to 9.
The work it names:

- The seven core endpoint groups, implemented and deployed: `GET /assets`
  (paginated, sortable, filterable), `GET /assets/{id}`,
  `GET /assets/{id}/price` (current price with a `sources` breakdown),
  `GET /assets/{id}/ohlcv` (timeframe and granularity parameters, with a
  `backfill_note` when history is partial), `POST /prices/batch`,
  `GET /oracles/{id}` (Reflector cross-reference), and `GET /backfill/status`.
- API Gateway response caching with per-endpoint TTLs, usage plans, API key
  issuance and throttling.
- The full VWAP formula wired into the current-price path (§5.5).
- Outlier detection, excluding sources that deviate beyond a configurable
  percentage from the inter-source median.
- Aquarius pool metadata integration, so Aquarius appears as a named source in
  the VWAP breakdown.
- Input validation: asset identifier format enforced, parameter ranges
  validated, `400` on invalid input.

**The backfill milestone for the tranche** is SDEX history covering
approximately January 2022 to present, written directly to Hetzner over mTLS per
ADR 0009, with the covered range visible through `GET /backfill/status`.

## 3. Architecture

Unchanged in shape from Milestone 1, extended at the API tier. The full
description, including the component table and the data-flow diagram, is
[`docs/prices-api-general-overview.md`](../prices-api-general-overview.md) §2
and §3.

What Tranche 2 added on top of the Milestone 1 platform:

| Layer         | Addition                                                                                                                                              |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| API Gateway   | Per-method response cache (0.5 GB), usage plans, key issuance, two-level throttling, CORS preflight on all seven `/v1` routes, REGIONAL custom domain |
| Lambda (axum) | The six read route groups beyond `/backfill/status`, input validation, the OpenAPI document served from the code                                      |
| ClickHouse    | `current_prices` materialised with the VWAP breakdown, `usd_rate` for measured USD conversion, the `method` provenance column                         |

The API is a single Rust axum Lambda behind API Gateway (ADR 0008), with no VPC
and no NAT gateway. Reads go over the public internet to Caddy on Hetzner with
mutual TLS.

## 4. Deviations from the approved wording

Three places where the delivered system departs from the literal wording of the
RFP or of the Tranche 2 criteria. Each is set out in full, with its measurements
and its reasoning, in
[`milestone-2-rfp-deviations.md`](milestone-2-rfp-deviations.md). That document
is part of this submission; the rows below are pointers, not summaries.

| #   | Wording says                            | We deliver                                                                                                          | Kind      |
| --- | --------------------------------------- | ------------------------------------------------------------------------------------------------------------------- | --------- |
| 1   | `Current Price (float USD)`             | a decimal **string**, so 14 fractional digits survive transport                                                     | defended  |
| 2   | AC 3: _"…return `X-Cache: Hit` header"_ | a reworded criterion graded on latency and deployed cache configuration; **no such header exists on this platform** | disclosed |
| 3   | AC 6: USDC 1d candles spot-checked      | XLM and yBTC over 28 dates; USDC excluded, reason stated                                                            | disclosed |

**AC 2 also carries an in-place amendment**, already recorded in §12 of the
design document since before this submission: the 100 req/s target is unchanged,
but since task 0157 no key on the default plan can sustain it, so the run needs a
usage plan created for it and the report must say which plan the key was on.

Declaring in-tranche refinements in the package itself is the discipline
Milestone 1 was accepted on. Its form answers carry an _"In-tranche scope
refinements"_ section, and its evidence document carries a _"what is deliberately
not claimed"_ list.

## 5. Acceptance-criteria evidence

### AC 1 — All 7 endpoint groups return correct, schema-valid responses for ≥ 20 major assets

**Verdict: met. 1032 checks pass, 0 fail, 0 skip**, against the deployed
production API on **2026-09-07 at 10:40 UTC**, and again at **11:09 UTC** with an
identical verdict.

The evidence is a scripted, re-runnable conformance suite
(`tools/scripts/conformance-0120.mjs`, task 0120). It exercises all seven route
groups for the twenty assets fixed in `tools/scripts/conformance-assets.json`,
a list derived from production volume rank on 2026-08-19 and deliberately
covering all three identifier forms: the native asset, code-with-issuer, and a
Soroban contract address.

**Every response is validated against the API's own live OpenAPI document**,
fetched from `/api-docs-json` at run time rather than from a checked-in copy, so
the API is measured against the contract it actually publishes. Error responses
are validated too. **There has been no schema failure in any run to date**; the
entire failure surface was ever only the correctness layer above the schema.

On top of the schema sit the assertions a schema cannot express: OHLC ordering on
every priced bucket, bucket alignment and strictly increasing timestamps, cursor
pagination proven exhaustive and duplicate-free, the batch endpoint agreeing with
the per-asset endpoint at a matching snapshot, decimal strings that survive the
JSON round trip, and documented sentinels asserted as sentinels.

#### Reproduce it

```bash
# API_KEY and BASE_URL from the environment or .env.local at the repo root
npm run conformance:0120 && echo "TRANCHE 2 AC 1: PASS"
```

The suite paces itself at 1.1 s per request, matching the free plan's 1 req/s
sustained rate, and drives no load. It writes a timestamped JSON report of every
individual check.

#### The trajectory, and what it does and does not show

| run                    | pass     | fail  | skip  |
| ---------------------- | -------- | ----- | ----- |
| 2026-08-19, first pass | 752      | 55    | 0     |
| 2026-08-25             | 847      | 13    | 11    |
| 2026-09-02             | 870      | 16    | 0     |
| **2026-09-07, 10:40**  | **1032** | **0** | **0** |
| **2026-09-07, 11:09**  | **1032** | **0** | **0** |

🔑 **Two things must be read alongside that table, and we would rather state them
than have them noticed.**

**The check count rose, 886 to 1032.** Nothing was relaxed to reach zero. Every
change on 2026-09-07 _added_ assertions, and each one replaced an assumption with
a measurement.

**No production code changed on 2026-09-07.** Every failure that disappeared that
day was a defect in the test, not a fix to the API. Three of them were the same
mistake in different places: an assertion that encoded a market state rather than
the contract. The suite treated a _decided_ sentinel as a pending defect; it
failed buckets that ADR 0011 §5 returns unpriced on purpose; and it called all
twenty fixture assets liquid when two of them had gone quiet since the list was
ranked three weeks earlier.

#### Why the report is citable rather than a snapshot

An acceptance report whose result moves with a background worker cannot be
evidence. The suite used to have exactly that property: three assets failed at
13:26 and passed at 13:38 on 2026-08-27 with unchanged code, because whether a
bucket carried a USD price depended on how far enrichment had advanced.

The two runs above were taken **29 minutes apart** and diffed check by check:

|                               |           |
| ----------------------------- | --------- |
| checks compared               | 1,032     |
| identical verdicts            | **1,032** |
| verdicts changed              | **0**     |
| underlying details that moved | **53**    |

The moving details are what make the identical verdicts mean something. Four
assets advanced at the tip during the gap — `native`, `EURC`, `AQUA` and `BTC`
each went from `0 of 169 buckets unpriced` to `1 of 168`. **Under the previous
assertion those four flip from pass to fail.**

#### Stated limits

- **The reports are gitignored as regenerable.** The citable artefact is the
  figures above plus the one-command reproduction, not a committed file.
- **The pagination walk moves between runs** — 19 pages and 3,725 distinct assets
  here, against 18 / 3,567 and 20 / 3,880 earlier. Exhaustive and duplicate-free
  each time; the traded population itself is what moves. Owned by task 0261.
- **One assertion is deliberately weaker than the rest.** A candle's timestamp is
  its bucket _start_, so "did this asset trade in the last 24 hours" cannot be
  answered exactly from candles alone. The liquidity test is three-valued:
  certainly inside the window, certainly outside, or a residual band one bucket
  wide that accepts either documented response. Claiming to resolve that band
  would assert something the data does not carry.
- **The fixture list was not re-derived** to today's volume ranking. Doing so
  would break comparability with the four earlier runs.
- USDT is excluded from the fixture, a known defect recorded as task 0172.
- Three defects found during the pass were spawned rather than fixed: tasks 0210,
  0211 and 0261.
- This is **not** the Tranche 3 deliverable _"integration test suite runs in
  CI"_. It is an operator-run acceptance check.

### AC 2 — Load test: 100 req/s for 5 minutes, p95 < 200 ms, error rate < 0.1 %

**Verdict: met, on the amended wording. p95 = 47.09 ms against a 200 ms bar, and
0 errors in 30,001 requests against a 0.1 % bar.** Measured 2026-09-03,
06:04–06:16 UTC, with k6 v2.2.0 (task 0121,
[`prices-api-load-test-100rps.md`](../prices-api-load-test-100rps.md)).

**The amendment, already recorded in §12 before this submission:** the target is
unchanged, but since task 0157 the default plan is 1 req/s with a 100,000 monthly
quota, so no ordinary key can sustain the run. A single 5-minute pass at 100 req/s
is 30,000 requests, nearly a third of a month's allowance. The run therefore used
a purpose-made plan, `prices-production-loadtest-plan` at 150 req/s, burst 300 —
and the criterion asks the report to say so, which it does.

| regime                  | pool          | p50       | p95       | p99        | achieved rate   | non-200 | error rate |
| ----------------------- | ------------- | --------- | --------- | ---------- | --------------- | ------- | ---------- |
| 1 — cache               | 1 asset       | 45.15     | 47.75     | 175.95     | 99.87 req/s     | 0       | 0.00 %     |
| **2 — the AC scenario** | **18 assets** | **45.04** | **47.09** | **107.53** | **99.85 req/s** | **0**   | **0.00 %** |
| 3 — wide                | 4,301 assets  | 64.97     | 76.91     | 183.16     | 94.28 req/s     | 28,314  | 94.38 %    |

Latencies in milliseconds, `phase:main`. Regime 2 is the criterion's scenario.
All four k6 thresholds held and the process exited 0, with zero dropped
iterations.

#### Reproduce it

```sh
S='avg,min,med,max,p(50),p(90),p(95),p(99)'
k6 run packages/prices-api/loadtest/price_load.js \
  -e BASE_URL=https://prices-api.sorobanscan.rumblefish.dev -e API_KEY="$API_KEY" \
  --summary-trend-stats="$S" --summary-export=loadtest-ac.json; echo "exit=$?"
```

`exit=0` means every threshold held, and that is what the report cites: in the
exported JSON a threshold recorded as `false` means _passed_, which is too easy
to misread. The trend-stats flag is required, because k6 stops at p95 by default.
Passing `-e ASSET=native` would pin a single asset and measure the gateway cache
instead of the criterion's scenario. Raw exports for all three regimes are in
`docs/loadtest-results/`.

#### 🔴 The pass is narrow, and quoting it alone would mislead

**Regime 2 is cache-dominated.** With an 18-asset pool and a 10-second TTL the
hit rate cannot fall below 98.20 %, so 47.09 ms is substantially the gateway
cache answering, not the data path. Regimes 1 and 2 are statistically
indistinguishable at p95.

**An uncontended cache miss costs roughly 170 to 240 ms.** That is already at the
Tranche 2 bar and about twice the Tranche 3 bar of 100 ms. The pass rests on the
cache concealing a data path that is, on today's measurements, too slow for
Tranche 3. **p99 already exceeds 100 ms in both passing regimes.**

**🔴 Regime 3 was an incident, not merely a failed run.** Widening the pool to
4,301 assets drove the hit rate to approximately zero and took the production
read path down. 28,314 of 30,001 requests returned `500 db_error`. Single
sequential requests were still failing minutes later. **The outage was at least
19 and at most 47 minutes**, and the width of that range is our own process
failure: the liveness probe died after its sixteenth check and the restart failed
silently. The root cause is not established and is open as task 0260.

Regime 3's latency column is therefore **not a latency measurement** and must not
be read as one, since 94 % of it is the speed of returning an error.

**A standing safety rule follows from this**, and it governs the cache evidence
below: do not verify behaviour by driving cache misses at rate.

#### Deferred

- **Cache-hit and cache-miss percentiles reported separately** — deferred to task 0260. Hits are measured. Misses are not obtainable by this method, because the
  system stops serving before a miss percentile can be sampled at 100 req/s.
- **Cold-start incidence and ClickHouse-side query time** — deferred to task
  0260, blocked on production CloudWatch access rather than on the run.
- The 20-asset pool ran as **18**: two assets returned 404 during setup and were
  excluded before measurement. Which assets are unservable drifts day to day, a
  data-freshness property rather than a fixed defect list.

### AC 3 — Cache confirmed within the TTL window

**This criterion is graded against amended wording. Read
[`milestone-2-rfp-deviations.md`](milestone-2-rfp-deviations.md) §2 for the full
argument; it is not reproduced here.**

The criterion as written asks for consecutive identical requests inside the TTL
window to return an `X-Cache: Hit` header. **This API emits no `X-Cache` header
on any route**, and cannot be made to emit a truthful one. That is not a setting
left switched off: API Gateway's stage cache has no hit-or-miss header feature.

It is graded instead against this wording:

> _"Cache confirmed: consecutive identical requests within the TTL window are
> served from the API Gateway stage cache, and a request after the window is not.
> Demonstrated by response latency, which separates cleanly, and by the deployed
> per-method cache configuration."_

🔑 **The claim weakens, and we state it in those words rather than blur it. A
header would be the cache asserting itself. Latency is behaviour consistent with
a cache.** That is a weaker form of proof than the criterion asked for.

**Verdict on the amended wording: met.** Measured 2026-09-03.

#### The cache works, and it expires when it says it does

`/price`, declared 10-second TTL, same URL throughout. Server time only, so the
TLS handshake a fresh client pays is excluded.

| request   | expected          | server time  |
| --------- | ----------------- | ------------ |
| first ask | miss              | 144.7 ms     |
| +2 s      | hit               | 53.3 ms      |
| +4 s      | hit               | 45.1 ms      |
| +6 s      | hit               | 47.7 ms      |
| **+13 s** | **miss, expired** | **139.7 ms** |
| +15 s     | hit               | 49.5 ms      |

**Hits fall between 45 and 53 ms, misses between 78 and 145 ms, and the two
ranges do not overlap.** Expiry is demonstrated on both TTL tiers: `/v1/assets`
at a 60-second TTL was still hot at +32 s and expired at +64 s. Both refilled
immediately. These hit figures independently reproduce the load test's 45 to
47 ms, from a different tool on a different day.

All six key-gated cached routes show the pattern. The deployed per-method
configuration was read straight off the production stage, and three surfaces
agree: the deployed stage, the CDK constant, and the handler's own cache-control
tiers. `x-api-key` appears in no cache key, so the cache is correctly shared
across callers for identical public data.

#### Reproduce it

```sh
BASE=https://prices-api.sorobanscan.rumblefish.dev
srv() { curl -s -o /dev/null -H "x-api-key: $API_KEY" \
  -w '%{time_starttransfer} %{time_appconnect}' "$1" |
  awk '{printf "%.1f ms\n", ($1-$2)*1000}'; }

U=$(date +%s)                                    # any unused value forces a miss
P="$BASE/v1/assets/native/price?min_volume_usd=$U"
srv "$P"; sleep 2; srv "$P"; sleep 2; srv "$P"   # miss, hit, hit
sleep 9;  srv "$P"                               # miss — TTL expired
sleep 2;  srv "$P"                               # hit — refilled
```

Because `min_volume_usd` is part of the `/price` cache key, a guaranteed miss
costs one request. No cold-asset hunting, no waiting, and **no load generation**.

#### Why the header cannot honestly be added

A header written by our own handler would be **actively wrong rather than
merely absent**. API Gateway replays a cached response byte for byte and the
Lambda runs only on a miss, so the header would freeze at miss time and report
`Miss` on every genuine hit. Measured, not assumed: the response body hash held
at `4694801b` across every hit and changed to `039f54a6` only when the entry
expired.

CloudFront, the only architecture that emits a truthful hit-or-miss header,
writes `X-Cache: Hit from cloudfront` — so the literal wording would still be
unmet after three to five days of edge work and a DNS migration. Cost is
explicitly not the argument; the two are within a couple of dollars a month.

The decision is recorded as **ADR 0012**, accepted and ratified by the team lead,
with all three alternatives closed with reasons and an explicit list of what
would justify revisiting it.

#### 🔴 Stated limits — the evidence is not uniformly strong

The gap between a hit and a miss tracks how expensive the underlying query is, so
the method discriminates unevenly across routes.

| route                  | miss / hit | daylight    |
| ---------------------- | ---------- | ----------- |
| `/ohlcv`               | 2.7×       | 120 ms      |
| `/price`               | ~2.9×      | ~95 ms      |
| **`/backfill/status`** | **1.4×**   | **19.5 ms** |

19.5 ms is real and repeatable, but close enough to ordinary variance that a
single pair of requests would not settle it on that route. There it is carried by
the deployed configuration rather than by latency. **A header would not have had
this property**, and presenting the six routes as uniformly strong would overstate
the evidence.

- **`/api-docs-json` is confirmed configured** at a 3600-second TTL but
  deliberately **not** claimed as demonstrated: its entry clears only on a deploy
  flush, so no miss can be induced to compare against.
- **The hit rate under load is derived, never observed** — there is no header to
  count. The bounds are ≥ 99.90 %, ≥ 98.20 % for the criterion's scenario, and
  approximately 0 % for the wide pool. **The derivation holds only because the
  load script sends no query string.** A client varying `min_volume_usd` gets one
  cache entry per value and none of these figures apply.
- `POST /prices/batch` and `/health` are uncached, established from the deployed
  stage configuration. `/health` timings are not offered as evidence, because it
  is a gateway mock and fast either way.
