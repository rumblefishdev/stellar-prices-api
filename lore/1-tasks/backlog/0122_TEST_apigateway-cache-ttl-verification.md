---
id: "0122"
title: "API Gateway cache verification — per-endpoint TTLs match §6 and repeat requests return X-Cache: Hit"
type: TEST
status: backlog
related_adr: ["0008"]
related_tasks: ["0118", "0121", "0128"]
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

- The in-app `Cache-Control` headers (`common/cache_control.rs`) are a second,
  independent layer aimed at clients/CDNs. Check the two do not contradict each
  other — an app-level `max-age` longer than the gateway TTL is confusing but
  harmless; shorter is a real inconsistency.
- Cache cluster size is a cost line (§10 budgets 0.5 GB). If the hit rate is
  poor because of key cardinality, resizing is the wrong first lever —
  narrowing the key is.
