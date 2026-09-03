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

## Acceptance Criteria

- [ ] Deployed per-endpoint TTLs match §6, or §6 is corrected with a recorded
      rationale
- [ ] A cache **hit** is demonstrated for each cached endpoint
- [ ] A cache **miss after expiry** is demonstrated for at least `/price` (15s)
      and one 60s endpoint — proving the TTL is real
- [ ] `POST /prices/batch` and `/health` confirmed uncached
- [ ] Cache-key composition documented, including the `?min_volume_usd=`
      interaction from [[0118]] and the `GET /assets` param matrix
- [ ] Cache hit rate under the [[0121]] load scenarios recorded
- [ ] If `X-Cache` is not emitted by the deployed stage, that is stated plainly
      with the alternative evidence used — no hand-waving in [[0128]]

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
