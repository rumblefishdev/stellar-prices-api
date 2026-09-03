---
id: "0122"
title: "API Gateway cache verification — per-endpoint TTLs match §6 and repeat requests return X-Cache: Hit"
type: TEST
status: active
related_adr: ["0008"]
related_tasks: ["0118", "0121", "0128", "0126", "0260", "0261"]
tags: [layer-infra, priority-medium, effort-small, milestone-M2, api-gateway, caching, verification, acceptance]
milestone: 2
links:
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
  - "../../../packages/prices-api/src/common/cache_control.rs"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Owns Tranche 2
      acceptance criterion 3. The cache cluster and per-method
      `cachingEnabled` flags already exist in CDK from M1
      (`api-gateway-stack.ts:199-238`, `apiGatewayCacheEnabled: true`), so
      this is verification and TTL reconciliation, not new infrastructure.
  - date: 2026-09-02
    status: backlog
    who: okarcz
    note: >
      Noted from [[0126]] while probing the same stack: TWO of the §6 TTLs in
      this task's own table are already contradicted by the CDK, so AC 1 has a
      known answer waiting. `/price` is 10s, not 15s, and `/backfill/status` is
      60s, not 30s (`CACHE_TTL`, `api-gateway-stack.ts:53-59`). Not fixed here
      — deciding whether the code or §6 is wrong IS this task. Also: 0126
      reworded its own "re-verify 0122's cache-hit behaviour" AC after finding
      there is no baseline to re-verify, and narrowed its scope to the one
      question its change raised (do the two hostnames share the stage cache).
      Everything else stays here.
  - date: 2026-09-03
    status: active
    who: okarcz
    note: >
      Activated. [[0121]] closed the same day and handed this task the finding
      that decides its method: **there is no `X-Cache` header on this API** —
      verified 2026-08-20, still absent 2026-09-03. AC 2 as the Tranche 2
      criterion words it ("consecutive identical requests within TTL window
      return `X-Cache: Hit`") cannot pass against the deployed API, so the last
      AC on this task — the one that anticipated exactly this — is now the live
      question, not a contingency. Full handover in its own section below.
      🔴 Carries a hard safety constraint from 0121: **do not drive cache misses
      at load**. Regime 3 took the entire ClickHouse read path down for 19-47
      minutes on 2026-09-03 ([[0260]]). Verifying a TTL takes a handful of
      requests; there is no reason to go near that.
  - date: 2026-09-03
    status: active
    who: okarcz
    note: >
      First measurement pass, all read-only. **AC 3 closed** — hit, expiry and
      re-fill demonstrated on both the 10s and the 60s tier. AC 1, 5 and 7 have
      full measured answers awaiting a write-up decision; AC 4 is confirmed from
      stage config (its timing control is void — `/health` is a gateway MOCK).
      🔴 **The "cache key is path-only" claim inherited from [[0121]] is
      FALSIFIED**: `/price` keys on `min_volume_usd` too and `/v1/assets` on
      seven params. Its real source is a stale comment in `price_load.js:16-18`,
      not the CDK, which has been right since 2026-08-28. This makes a
      guaranteed cache miss cost one request, which is how this pass avoided
      load entirely. AC 1 resolves as **correct §6, not the code** — the
      deployed stage, `CACHE_TTL` and `cache_control.rs` all agree and §6 is
      wrong on two TTLs, on "cache key includes query params", and omits four
      cached routes. Also logged: a bimodal miss distribution (a miss following
      cache hits costs ~2x one following misses) as an explicit hypothesis.
  - date: 2026-09-03
    status: active
    who: okarcz
    note: >
      PR #279 merged (`652bbd8`) — §6 corrected and the stale path-only claim
      fixed at its source in `price_load.js`. **AC 1, 4 and 5 closed**; 4 of 7
      ACs now done. Remaining: AC 2 (hits on the four cached routes not yet
      probed — `/assets/{id}`, `/ohlcv`, `/oracles/{id}`, `/backfill/status`),
      AC 6 (hit rate from 0121's pool arithmetic) and AC 7, which is not
      measurement but a **decision** — add an `X-Cache` header or renegotiate
      the criterion with the reviewer on the timing evidence. AC 7 is the one
      that gates [[0128]].
  - date: 2026-09-03
    status: active
    who: okarcz
    note: >
      **AC 7 closed — the `X-Cache` decision is made**: renegotiate the
      criterion, do not add the header. Operator's call, taken after costing
      CloudFront at 3-5 days for a header that would still read
      `Hit from cloudfront` and not match the wording. Evidence document
      `docs/prices-api-cache-verification.md` opened as PR #281. **5 of 7 ACs
      done.** Remaining: AC 2 (hits on the four cached routes not yet probed)
      and AC 6 (hit rate, derivable from 0121's pool arithmetic — no new run).
      ⏳ The rewording still needs the reviewer's agreement; nothing else here
      depends on it.
---

# API Gateway cache verification

## Summary

Tranche 2 AC 3: *"Cache confirmed: consecutive identical requests within TTL
window return `X-Cache: Hit` header."*

The infrastructure is already deployed — `api-gateway-stack.ts` enables a cache
cluster when `apiGatewayCacheEnabled` is set (true in `envs/production.json`),
turns `cachingEnabled` on per method for the data routes, and explicitly off for
`/health` and `POST /prices/batch`. What has never been verified is that the
**TTLs match §6** and that a real client actually observes a hit.

## Context

§6 specifies per-endpoint TTLs:

| Endpoint | TTL |
|---|---|
| `GET /assets` (list) | 60s |
| `GET /assets/{id}/ohlcv` | 60s |
| `GET /assets/{id}/price` | 15s |
| `GET /backfill/status` | 30s |
| `POST /prices/batch` | uncached |

Two things make this worth a dedicated task rather than a line in [[0121]]:

1. **`X-Cache` is not on by default.** API Gateway only returns the
   `X-Cache: Hit`/`Miss` header when **`cacheDataEncrypted`/method-level cache
   headers are surfaced** — verify what the deployed stage actually emits. If
   the header is absent, the AC as written cannot be demonstrated, and the fix
   (enabling it, or evidencing hits via CloudWatch `CacheHitCount` /
   `CacheMissCount` instead) is part of this task. **Check this first** — it
   determines whether the AC needs a documented reinterpretation.
2. **Cache keys include query params** (§6: *"Cache key includes query
   params"*). That interacts directly with [[0118]]'s new `?min_volume_usd=`
   param and with `GET /assets`' `sort`/`order`/`cursor`/`limit` matrix: a
   high-cardinality key space silently destroys the hit rate that [[0121]]'s p95
   depends on. Measure it.

## Handover from 0121 (2026-09-03)

[[0121]] is **closed and archived**: Tranche 2 AC 2 passes — 100 req/s held for
5 minutes, **p95 47.09 ms** against the 200 ms bar, **0 errors in 30,001
requests**. Report at `docs/prices-api-load-test-100rps.md`, raw k6 exports in
`docs/loadtest-results/`. It measured this task's subject in passing and the
findings below are its handover, not this task's own work.

### 🔴 The thing to settle first — there is no `X-Cache` header

Verified 2026-08-20 and still absent 2026-09-03. The Tranche 2 criterion is
worded *"consecutive identical requests within TTL window return `X-Cache:
Hit`"*, and that **cannot pass against the current API**. Two ways out, and
picking one is the first decision this task makes:

1. Someone adds the header.
2. The criterion is renegotiated with the reviewer.

This task's last AC already anticipated the possibility; it is now the live
question rather than a fallback.

**If it is renegotiated, 0121 supplies the evidence to do it with.** The latency
split is unambiguous — roughly 4x, with no overlap between the distributions:

| measured | latency |
|---|---|
| served from gateway cache | **45-47 ms** (p50-p95, 60,000 requests) |
| cache miss, uncontended | **163-238 ms** (5 cold assets, ~0.3 req/s) |

A hit/miss verdict **by timing alone is therefore defensible**. It is a weaker
claim than a header — it shows behaviour consistent with a cache rather than the
cache asserting itself — but it is measured, reproducible and already written
up. Say which of the two it is in [[0128]]; do not blur them.

### Three things that shape the method

- **The cache key is the path only.** 0121 states this and its whole
  three-regime design rests on it, but it is **inherited, not independently
  proven** — worth ~10 minutes of confirmation here, because AC 5 owns cache-key
  composition anyway. ⚠️ If a query parameter *does* bust the cache, the TTL
  test gets much easier, and the [[0118]] `?min_volume_usd=` interaction this
  task already lists becomes a real hit-rate risk rather than a theoretical one.
- **Pool size is the only lever on hit rate.** Over a 300 s run an asset can
  miss at most 30 times at any offered rate. That arithmetic is what let 0121
  build its 0% / 2% / 100% miss series.
- **Regimes 1 and 2 ran through the custom domain**
  (`prices-api.sorobanscan.rumblefish.dev`) and showed clean cache behaviour.
  That is **weak positive evidence** for [[0126]]'s open cross-hostname question
  recorded in the Notes below — an observation in passing, not an experiment.
  The experiment still belongs here.

### 🔴 Safety constraint — do not drive cache misses at load

On 2026-09-03, driving 100 req/s of **cache misses** at `GET /assets/{id}/price`
returned `500 db_error` on 94.38% of requests and took the whole ClickHouse read
path down for **19-47 minutes** — `/price` and `/v1/assets` alike, failing even
a single request with no concurrency behind it. It recovered unattended. Root
cause unresolved, now [[0260]].

⚠️ **[[0260]]'s open question bears directly on this task's conclusions**:
whether the ceiling is connections or query cost decides whether *raising a TTL*
is even a relevant lever. The Notes below suggest narrowing the key before
resizing the cluster; 0260 may add that raising TTL only hides the ceiling
behind a higher hit rate rather than removing it.

**Verifying a TTL needs a handful of requests, not load.** There is no reason to
go near that regime from this task.

### Also inherited

- **[[0261]]** — the asset listing churns heavily between full walks (two walks
  a day apart returned 3,543 and 4,306 ids; 2,128 added, 1,365 removed) because
  cursor pagination runs over a table the MV replaces every minute. **Relevant
  only if this task enumerates assets** — which AC 5's `GET /assets` param
  matrix might.
- **The k6 script is reusable for single-asset work**:
  `-e ASSET=native -e RATE=… -e DURATION=…`. See
  `packages/prices-api/loadtest/README.md`.

## Implementation

- Read the deployed stage's method settings and reconcile every TTL against the
  §6 table. Fix drift in CDK; if a TTL was deliberately changed, correct §6
  instead and record why.
- Confirm the cache-key configuration: which params are part of the key, and
  whether the `x-api-key` header is (it should not be — that would give every
  key its own cache and gut the hit rate).
- Demonstrate a hit: two identical requests inside the TTL, showing
  `X-Cache: Hit` on the second (or CloudWatch `CacheHitCount` incrementing, if
  the header is unavailable). Do it per endpoint, not once.
- Demonstrate **expiry**: a request after TTL+ε is a miss again. A cache that
  never expires would also produce "Hit" and would be a correctness bug —
  `/price` at 15s TTL is a freshness contract.
- Confirm the negative cases: `POST /prices/batch` and `/health` are never
  cached.
- Measure hit rate over the [[0121]] load run and record it.
- Verify no cached response leaks across API keys (a shared cache is correct for
  identical public data; confirm that is in fact what happens and that no
  key-scoped data exists on these routes).

## Findings — first measurement pass (2026-09-03)

All read-only. Control plane read with `--profile soroban-readonly`; the live
requests were a handful of `curl`s, single-threaded, on the **free-tier key**
(`pricing-api-free-production`, 1 req/s) — deliberately not the 0121 load-test
key, and nowhere near [[0260]]'s regime.

### AC 1 — the deployed stage, and §6 is the stale side

Read from the deployed stage (`02mabge71l`, stage `production`, cache cluster
0.5 GB `AVAILABLE`), not from CDK:

| route | deployed TTL | §6 says |
|---|---|---|
| `GET /v1/assets` | 60 s | 60 s ✅ |
| `GET /v1/assets/{id}` | 60 s | *not listed* |
| `GET /v1/assets/{id}/ohlcv` | 60 s | 60 s ✅ |
| `GET /v1/assets/{id}/price` | **10 s** | 15 s ❌ |
| `GET /v1/oracles/{id}` | 60 s | *not listed* |
| `GET /v1/backfill/status` | **60 s** | 30 s ❌ |
| `GET /api-docs-json` | 3600 s | *not listed* |
| `POST /v1/prices/batch` | uncached | uncached ✅ |
| `GET /health` | uncached | *not listed* |

**Three surfaces agree and §6 is the outlier**: the deployed stage matches
`CACHE_TTL` (`api-gateway-stack.ts:53-59`), which matches the handler's tiers
(`cache_control.rs`: `SHORT` = `max-age=10`, `MEDIUM` = `max-age=60`). A live
`/price` response carries `cache-control: public, max-age=10`, so the app layer
and the gateway state the same number to the caller. `/api-docs-json` is the one
deliberate mismatch (3600 s gateway vs 300 s handler), already documented at
`api-gateway-stack.ts:60-68`.

So AC 1 resolves as **correct §6**, not the code. §6 lives at
`docs/prices-api-general-overview.md:1194`. Two further errors there beyond the
TTLs: it asserts *"Cache key includes query params"* as a blanket property (false
for `/backfill/status` and both detail routes — see AC 5), and it omits four
cached routes entirely.

### 🔴 AC 5 — the cache key is NOT path-only

**This overturns the claim inherited from [[0121]]** and recorded in the handover
above. Measured per method on the deployed stage:

| route | `cacheKeyParameters` |
|---|---|
| `/v1/assets` | `type, search, sort, order, cursor, limit, min_volume_usd` (**7**) |
| `/v1/assets/{id}/ohlcv` | path + `timeframe, granularity, start, end, base_currency` |
| `/v1/assets/{id}/price` | path + **`min_volume_usd`** |
| `/v1/assets/{id}`, `/v1/oracles/{id}` | path only |
| `/v1/backfill/status` | empty — path only |

The CDK agrees (`api-gateway-stack.ts:621-648`). **The stale claim's actual
source is the k6 script's own header comment**, which cites
`addGet(price, [PATH_ID])`; the code has read
`addGet(price, [PATH_ID, qs('min_volume_usd')])` since 2026-08-28, added after a
measured cross-caller poisoning bug (`api-gateway-stack.ts:608-616` — one
caller's `?min_volume_usd=200000` served its narrowed `sources` to the next
param-less caller for a whole TTL). 0121's runs sent no query string, so the key
did collapse to the path *for that experiment*; the generalisation is wrong.
⚠️ **`packages/prices-api/loadtest/price_load.js:16-18` needs correcting** — it
is the copy future readers will trust.

Two consequences:

1. **A guaranteed miss costs one request.** A fresh `?min_volume_usd=` value is a
   new cache entry, so the TTL test needs no cold-asset hunting, no waiting and
   no load. That is how the runs below were done.
2. **The [[0118]] hit-rate risk is real, not theoretical.** `min_volume_usd` is a
   continuous number in the key on two routes, and `/v1/assets` carries a
   seven-parameter key space. Per the Notes below, the first lever is narrowing
   the key, not resizing the 0.5 GB cluster.

`x-api-key` is in no cache key, so **the cache is shared across API keys** — the
correct outcome for identical public data, and now confirmed rather than assumed.

### AC 2 + AC 3 — hit, expiry and re-fill, on both tiers

Server time only (`time_starttransfer - time_appconnect`), so the ~45 ms TLS
handshake a fresh `curl` pays each request is excluded — that is what makes
these comparable to 0121's k6 figures, which reuse connections.

`/price`, declared 10 s, same URL throughout except A7:

| | server |
|---|---|
| A1 first ask (MISS) | 144.7 ms |
| A2 +2 s (HIT) | 53.3 ms |
| A3 +4 s (HIT) | 45.1 ms |
| A4 +6 s (HIT) | 47.7 ms |
| **A5 +13 s (MISS — expired)** | **139.7 ms** |
| A6 +15 s (HIT) | 49.5 ms |
| A7 fresh param (MISS) | 86.1 ms |

`/v1/assets`, declared 60 s (first pass, wall-clock incl. TLS): first **234 ms**,
+2 s **154 ms**, +32 s **145 ms** (still hot), **+64 s 950 ms (expired)**, +66 s
**146 ms**.

Both tiers expire when they say they do, and re-fill immediately after. The hits
cluster at **45-53 ms**, reproducing 0121's 45-47 ms independently.

⚠️ **`/v1/assets` costs ~950 ms on a miss** — roughly 4x a `/price` miss. That is
the number that makes the seven-parameter key space matter.

### The miss distribution is bimodal — hypothesis, not a conclusion

Five consecutive fresh-parameter misses ran **77.8 / 81.4 / 84.1 / 87.5 /
88.5 ms**, but the two misses that *followed a run of cache hits* (A1, A5) cost
**144.7** and **139.7 ms** — near double.

Plausible mechanism: a cache hit is served by the gateway and **never reaches the
Lambda**, so a stretch of hits leaves the handler's warm ClickHouse connection
idle, and the next miss pays to re-establish it (§6 puts the mTLS hop at
~80-130 ms RTT). Consecutive misses keep it hot. **Seven data points and no
attempt to falsify it** — recorded because it would mean *published* miss
latency depends on the hit rate preceding it, which is the opposite of the usual
assumption. If it survives scrutiny it belongs to [[0260]], not here.

⚠️ These misses (78-145 ms) are **faster than 0121's 163-238 ms**. Not a
contradiction: 0121 measured 5 deliberately cold assets at ~0.3 req/s, these are
`native` — the hottest asset — with everything warm. Do not quote the two ranges
as one distribution.

### AC 7 — there is no `X-Cache` header, confirmed independently

Full response headers on a live `/price` 200 carry `x-amzn-requestid`,
`x-amz-apigw-id`, `x-amzn-trace-id`, `cache-control`, `vary`,
`access-control-allow-origin` — **and no `X-Cache`**. 0121's finding stands.
The timing evidence above is what the criterion has to rest on instead, and the
two are not equivalent: this shows *behaviour consistent with a cache*, not the
cache asserting itself. Say which one it is in [[0128]].

### AC 4 — confirmed from config; the `/health` timing control is weak

`POST /v1/prices/batch` and `GET /health` both read `cachingEnabled: false` on
the deployed stage. ⚠️ The timing control adds nothing: `/health` answered in
133 ms then 143 ms, but it is a **gateway MOCK** ([[0126]]), so it is fast
whether cached or not. Absence of caching there is established by the stage
config alone — do not present the timings as evidence.

## Acceptance Criteria

- [x] Deployed per-endpoint TTLs match §6, or §6 is corrected with a recorded
      rationale — **§6 corrected**, PR #279 merged 2026-09-03 (`652bbd8`).
      The code was right on all three surfaces; §6 was the only wrong copy.
- [ ] A cache **hit** is demonstrated for each cached endpoint
- [x] A cache **miss after expiry** is demonstrated for at least `/price` (15s
      — in fact **10s**) and one 60s endpoint — proving the TTL is real.
      **Done 2026-09-03**: `/price` hot at +6s, expired at +13s; `/v1/assets`
      hot at +32s, expired at +64s. Both re-filled immediately after.
- [x] `POST /prices/batch` and `/health` confirmed uncached — both read
      `cachingEnabled: false` on the deployed stage. ⚠️ From **config only**;
      the timing control is void because `/health` is a gateway MOCK.
- [x] Cache-key composition documented, including the `?min_volume_usd=`
      interaction from [[0118]] and the `GET /assets` param matrix — measured
      per method, written into §6 (PR #279) and the Findings above. Includes
      that `x-api-key` is in no cache key, so the cache is shared across
      callers.
- [ ] Cache hit rate under the [[0121]] load scenarios recorded
- [x] If `X-Cache` is not emitted by the deployed stage, that is stated plainly
      with the alternative evidence used — no hand-waving in [[0128]].
      **Done 2026-09-03**: `docs/prices-api-cache-verification.md` (PR #281)
      states the absence, proves the handler cannot mark a hit, costs the
      CloudFront alternative, proposes the reworded criterion and labels the
      latency evidence as the weaker claim. ⏳ The reviewer has not yet agreed
      the rewording — that hand-off is tracked in Future Work, not here.

## Design Decisions

### Emerged

1. **AC 3's criterion is renegotiated, not satisfied — the `X-Cache` header is
   not being added.** Decided with the operator 2026-09-03 after costing the
   alternative. Evidence document: `docs/prices-api-cache-verification.md`
   (PR #281), which proposes the reworded criterion and carries everything
   below.

   The cheap version was tested and **fails**: API Gateway replays a cached
   response byte for byte (body hash `4694801b` across two hits, changing to
   `039f54a6` only on the post-expiry miss), so a header written by the handler
   is frozen at miss time and would report `Miss` on genuine hits. Worse than no
   header. The same reasoning rules out integration-response mappings.

   A truthful header needs a cache **in front of** the gateway. Nothing is:
   the REST API is `REGIONAL` and both custom domains are regional endpoints
   with `distributionDomainName: null`. CloudFront would mean ~6 cache
   behaviours to reproduce the per-route key, an origin request policy keeping
   `x-api-key` out of the key, a DNS move, a decision about the existing 0.5 GB
   stage cache, and re-verification of everything [[0126]] settled — **3-5 days**
   plus edge deploy risk. Cost is roughly neutral (~$12-13/mo CloudFront against
   ~$14-15/mo recovered), so cost is not the reason.

   🔑 **The deciding argument is that option (b) does not work either**:
   CloudFront emits `X-Cache: Hit from cloudfront`, not `X-Cache: Hit`. The
   criterion's literal wording is unmet after a week of edge work, and the same
   reviewer conversation is still required. One thing that would *not* have
   cost anything: a `*.sorobanscan.rumblefish.dev` cert already exists in
   `us-east-1`.

2. **The evidence is latency, and it is labelled as the weaker claim it is.**
   Hits 45-53 ms against misses 78-145 ms, no overlap, with expiry demonstrated
   on both tiers. This shows *behaviour consistent with a cache*, not the cache
   asserting itself. Recorded that way in the report and to be carried that way
   into [[0128]] — the distinction is not to be blurred at submission time.

3. **The report is a `docs/` deliverable, mirroring [[0121]].** Not folded into
   [[0128]]: 0128 has not started, and the same reasoning [[0248]] settled
   applies — an evidence artefact that stands on its own should not be blocked
   behind an unstarted package. 0128 cites it, as it cites the load test report.

## Notes

- 🔴 **AC 1 already has two answers waiting — measured 2026-09-02 from
  [[0126]].** The §6 table in the Context section above disagrees with the
  deployed CDK on two rows:

  | endpoint | §6 (this task) | `CACHE_TTL` in CDK |
  |---|---|---|
  | `GET /assets/{id}/price` | 15s | **10s** |
  | `GET /backfill/status` | 30s | **60s** |

  The other rows match. This is exactly the drift AC 1 exists to reconcile, so
  it is recorded rather than fixed: whether the code or §6 is wrong is a call
  this task makes, and the answer has to be written into one of them.
  ⚠️ Note the `/price` value also makes the "miss after expiry" AC cheaper than
  it reads — a 10s window, not 15.

- **The custom domain ([[0194]]) added a hostname, not a cache.** Both
  `prices-api.sorobanscan.rumblefish.dev` and the execute-api URL map to the
  same stage, so they should share one cache and one set of keys. 0126 hands
  over a short cross-hostname probe for that single property; if it comes back
  showing per-domain fragmentation, it lands here, because the hit rate this
  task measures would be the thing it damages.

- The in-app `Cache-Control` headers (`common/cache_control.rs`) are a second,
  independent layer aimed at clients/CDNs. Check the two do not contradict each
  other — an app-level `max-age` longer than the gateway TTL is confusing but
  harmless; shorter is a real inconsistency.
- Cache cluster size is a cost line (§10 budgets 0.5 GB). If the hit rate is
  poor because of key cardinality, resizing is the wrong first lever —
  narrowing the key is.
