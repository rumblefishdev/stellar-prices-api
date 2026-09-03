# API Gateway cache verification — TTLs, hits, expiry, and the `X-Cache` question

> **STATUS: the cache is verified working and every TTL is confirmed against the
> deployed stage. The acceptance criterion cannot be satisfied as literally
> worded, because this API emits no `X-Cache` header and cannot be made to emit a
> truthful one without an edge re-architecture.** This document requests a
> reworded criterion and supplies the evidence to judge it on. See
> [The `X-Cache` question](#the-x-cache-question) before quoting anything here.

Task [0122](../lore/1-tasks/active/0122_TEST_apigateway-cache-ttl-verification.md) ·
Tranche 2 AC 3 · measured 2026-09-03 against
`https://prices-api.sorobanscan.rumblefish.dev`

## The acceptance criterion

> _"Cache confirmed: consecutive identical requests within TTL window return
> `X-Cache: Hit` header."_

The criterion bundles two claims: **that the cache works**, and **that a
particular header proves it**. The first is confirmed below on every cached
route. The second is not achievable on this architecture.

## Verdict

| claim                                                                  | result                                                               |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------- |
| Per-endpoint TTLs are as specified                                     | ✅ confirmed on the deployed stage — §6 corrected where it disagreed |
| Consecutive identical requests inside the window are served from cache | ✅ demonstrated, 10 s and 60 s tiers                                 |
| The cache **expires** when it says it does                             | ✅ demonstrated, both tiers                                          |
| `POST /prices/batch` and `/health` are uncached                        | ✅ confirmed                                                         |
| A response carries `X-Cache: Hit`                                      | ❌ **no such header exists on this API**                             |

**Requested rewording:**

> _"Cache confirmed: consecutive identical requests within the TTL window are
> served from the API Gateway stage cache, and a request after the window is not.
> Demonstrated by response latency, which separates cleanly, and by the deployed
> per-method cache configuration."_

---

## The `X-Cache` question

### There is no `X-Cache` header, and there is no cheap way to add one

The full response headers on a live `200` from `GET /v1/assets/native/price`:

```
HTTP/2 200
date, content-type, content-length
x-amzn-requestid, x-amz-apigw-id, x-amzn-trace-id
access-control-allow-origin: *
vary: origin, access-control-request-method, access-control-request-headers
cache-control: public, max-age=10
```

No `X-Cache`. Amazon API Gateway's REST stage cache does not emit a hit/miss
header and offers no setting to turn one on. That is a property of the service,
not a configuration oversight on our side.

### The obvious workaround produces a header that lies

The natural suggestion is to have the application set the header itself. **That
cannot work here, and we measured why rather than assuming it.** The Lambda
handler only executes on a cache _miss_; on a hit, API Gateway replays the
stored response without invoking anything. And it replays the whole response,
headers included.

Requesting one URL repeatedly across a TTL boundary, hashing each body:

| t    | expected       | body hash                |
| ---- | -------------- | ------------------------ |
| 0 s  | miss           | `4694801b`               |
| 3 s  | hit            | `4694801b` — identical   |
| 6 s  | hit            | `4694801b` — identical   |
| 14 s | miss (expired) | `039f54a6` — **changed** |
| 17 s | hit            | `039f54a6` — identical   |

The response is replayed byte for byte. A header written by the handler is
therefore frozen at miss time and replayed unchanged on every subsequent hit —
so a handler-set `X-Cache` would report `Miss` on genuine hits. That is worse
than having no header, because it would be confidently wrong. The same argument
rules out API Gateway integration-response mappings, which also run only at
integration time.

### A truthful header requires CloudFront, and that is an edge re-architecture

The only component in this stack that can honestly report its own hit or miss is
a cache that sits in front of API Gateway. CloudFront does this natively. Today
nothing is in front: the REST API is `REGIONAL` and both custom domains are
regional endpoints with no distribution.

Adding one means:

1. A CloudFront distribution defined in CDK.
2. **Roughly six cache behaviours**, one per route pattern, to reproduce the
   per-route cache key documented below. A single catch-all behaviour would
   change hit-rate behaviour on every route.
3. An origin request policy that forwards `x-api-key` to the origin **without**
   including it in the cache key — otherwise every API key gets a private cache
   and the hit rate collapses.
4. Moving the DNS record for the public hostname.
5. A decision on the existing 0.5 GB stage cache: keeping it means two caches
   with two TTLs and a harder story to audit; removing it changes the very
   behaviour this document verifies.
6. Re-verification of CORS, the custom domain and the WAF decision, all of which
   are edge behaviour settled against the current edge.

Estimated at **3-5 days** plus deployment risk on the edge. One cost that does
_not_ apply: a `*.sorobanscan.rumblefish.dev` certificate already exists in
`us-east-1`, where CloudFront requires it, so no new certificate or DNS
validation is needed.

Monetary cost is approximately neutral — on the order of **$12-13/month** for
CloudFront at 10M requests, against roughly **$14-15/month** recovered by
retiring the stage cache. These are derived from list prices and should be
confirmed against the current pricing page before being relied on. Cost is not
the reason for the decision either way.

### Why we are not doing it

**CloudFront's header reads `X-Cache: Hit from cloudfront`, not `X-Cache: Hit`.**
Even after the full re-architecture, the criterion's literal wording would still
not be met, and the same conversation with the reviewer would still be needed.

Spending a week of Tranche 2 on an edge layer, re-opening settled decisions, to
arrive at a header that still does not match the words — against presenting the
measured evidence below today. We chose the evidence.

**What is deliberately not claimed:** the latency evidence demonstrates
_behaviour consistent with a cache_. It is not the cache asserting itself. That
is a weaker form of proof than a header would be, and it is stated here rather
than blurred.

---

## Evidence

All measurements are read-only. The control plane was read with a read-only
role; the live checks are a few dozen single-threaded `curl` requests on a
**free-tier key** (1 req/s). No load-generation tool was used, deliberately —
see [Safety](#safety).

### 1. Per-endpoint TTLs, read from the deployed stage

REST API `02mabge71l`, stage `production`, cache cluster **0.5 GB**, status
`AVAILABLE`.

| route                       | TTL      | cached |
| --------------------------- | -------- | ------ |
| `GET /v1/assets`            | 60 s     | yes    |
| `GET /v1/assets/{id}`       | 60 s     | yes    |
| `GET /v1/assets/{id}/ohlcv` | 60 s     | yes    |
| `GET /v1/assets/{id}/price` | **10 s** | yes    |
| `GET /v1/oracles/{id}`      | 60 s     | yes    |
| `GET /v1/backfill/status`   | 60 s     | yes    |
| `GET /api-docs-json`        | 3600 s   | yes    |
| `GET /health`               | —        | **no** |
| `POST /v1/prices/batch`     | —        | **no** |

These agree with `CACHE_TTL` in `infra/src/lib/stacks/api-gateway-stack.ts` and
with the handler's own `Cache-Control` tiers in
`packages/prices-api/src/common/cache_control.rs` (`SHORT` = `max-age=10`,
`MEDIUM` = `max-age=60`). A live `/price` response carries
`cache-control: public, max-age=10`, so the gateway and the application state
the same number to the caller.

`/api-docs-json` is a deliberate mismatch — 3600 s at the gateway, 300 s to the
client — documented at `api-gateway-stack.ts:60-68`. It is flushed on deploy.

> **§6 of the design overview was wrong and has been corrected** (PR #279). It
> stated `/price` at 15 s and `/backfill/status` at 30 s, described the cache key
> as "includes query params" without qualification, and omitted four cached
> routes. The deployed stage, the CDK and the handler all agreed with each
> other; the document was the only incorrect copy.

### 2. A hit, an expiry, and a re-fill — the 10 s tier

`GET /v1/assets/native/price?min_volume_usd=<fixed>`, same URL throughout except
the final row. Times are **server time only** (`time_starttransfer` minus
`time_appconnect`), excluding the TLS handshake a fresh `curl` pays on every
request:

| request               | expected           | server time  |
| --------------------- | ------------------ | ------------ |
| first ask             | miss               | 144.7 ms     |
| +2 s                  | hit                | 53.3 ms      |
| +4 s                  | hit                | 45.1 ms      |
| +6 s                  | hit                | 47.7 ms      |
| **+13 s**             | **miss — expired** | **139.7 ms** |
| +15 s                 | hit                | 49.5 ms      |
| fresh parameter value | miss               | 86.1 ms      |

### 3. The same, on the 60 s tier

`GET /v1/assets?limit=5&min_volume_usd=<fixed>` (wall-clock, TLS included):

| request   | expected                      | total      |
| --------- | ----------------------------- | ---------- |
| first ask | miss                          | 234 ms     |
| +2 s      | hit                           | 154 ms     |
| +32 s     | hit — still inside the window | 145 ms     |
| **+64 s** | **miss — expired**            | **950 ms** |
| +66 s     | hit                           | 146 ms     |

The expiry rows are the ones that matter. A cache that never expired would also
produce fast repeat requests; only the return to slow proves the TTL is a real
freshness boundary rather than a permanent store.

`/v1/assets` costs **~950 ms** on a miss, roughly 4× a `/price` miss. That is
what makes its cache key composition (below) worth attention.

### 4. Hit and miss latency do not overlap

Five consecutive misses forced with fresh parameter values: **77.8, 81.4, 84.1,
87.5, 88.5 ms**. Cache hits across the same session: **45.1, 47.7, 49.5,
53.3 ms**.

The [100 req/s load test](./prices-api-load-test-100rps.md) measured the cached
path independently at **45-47 ms** (p50-p95 over 60,000 requests), which these
figures reproduce.

> ⚠️ That report's _miss_ figures — 163-238 ms — came from five deliberately
> cold assets at ~0.3 req/s. The 78-88 ms misses here are `native`, the hottest
> asset, with everything warm. **The two ranges are different measurements and
> must not be quoted as one distribution.**

### 5. Cache key composition

API Gateway keys the cache **only on parameters declared as
`cacheKeyParameters`** — not on the query string as a whole. Read per method
from the deployed stage:

| route                       | cache key                                                              |
| --------------------------- | ---------------------------------------------------------------------- |
| `GET /v1/assets`            | `type`, `search`, `sort`, `order`, `cursor`, `limit`, `min_volume_usd` |
| `GET /v1/assets/{id}/ohlcv` | path + `timeframe`, `granularity`, `start`, `end`, `base_currency`     |
| `GET /v1/assets/{id}/price` | path + `min_volume_usd`                                                |
| `GET /v1/assets/{id}`       | path only                                                              |
| `GET /v1/oracles/{id}`      | path only                                                              |
| `GET /v1/backfill/status`   | path only                                                              |

**`x-api-key` appears in no cache key.** The cache is therefore shared across
API keys — correct here, because these routes serve identical public data to
every caller, and confirmed rather than assumed.

Declaring the parameters is not optional. An undeclared parameter collapses all
its values onto one entry, which is not a diluted hit rate but **cross-caller
poisoning**: measured on production on 2026-08-28, one
`GET /v1/assets/native/price?min_volume_usd=200000` caused the next parameter-less
request to receive that caller's narrowed `sources` and reweighted `vwap_24h`
for the remainder of the TTL. The rationale is recorded at
`api-gateway-stack.ts:608-616`.

### 6. Uncached routes

`POST /v1/prices/batch` and `GET /health` both read `cachingEnabled: false` on
the deployed stage.

> ⚠️ This is established from the stage configuration alone. A timing check on
> `/health` proves nothing: it is answered by a gateway MOCK integration and is
> fast whether cached or not.

---

## Method — reproducing this

Every figure above comes from two kinds of check, both cheap.

**Configuration**, needing only a read-only role:

```sh
aws apigateway get-stage --rest-api-id 02mabge71l --stage-name production \
  --query 'methodSettings' --region eu-central-1
aws apigateway get-resources --rest-api-id 02mabge71l --embed methods \
  --region eu-central-1
```

**Live behaviour**, needing an API key on any usage plan. Export the key without
echoing it, then measure server time rather than wall-clock so the TLS handshake
does not mask the difference:

```sh
read -rs API_KEY && export API_KEY
BASE=https://prices-api.sorobanscan.rumblefish.dev

srv() {                       # prints server time in ms
  curl -s -o /dev/null -H "x-api-key: $API_KEY" \
    -w '%{time_starttransfer} %{time_appconnect}' "$1" |
    awk '{printf "%.1f ms\n", ($1-$2)*1000}'
}

U=$(date +%s)                 # any unused value forces a guaranteed miss
P="$BASE/v1/assets/native/price?min_volume_usd=$U"
srv "$P"; sleep 2; srv "$P"; sleep 2; srv "$P"   # miss, hit, hit
sleep 9;  srv "$P"                               # miss — TTL expired
sleep 2;  srv "$P"                               # hit — refilled
```

The `min_volume_usd` parameter is what makes this cheap: because it is part of
the cache key on `/price`, any unused value is a guaranteed miss. No cold-asset
hunting, no waiting for eviction, and **no load generation**.

## Safety

⚠️ **Do not verify cache behaviour by driving cache misses at rate.** On
2026-09-03 a load-test regime that issued 100 req/s of misses against
`GET /assets/{id}/price` returned `500` on 94.38 % of requests and left the
ClickHouse read path unavailable for 19-47 minutes, failing even single requests
with no concurrency behind them. It recovered unattended. That incident is under
investigation as its own task and is described in
[the load test report](./prices-api-load-test-100rps.md).

Everything in this document was produced with a few dozen sequential requests on
a 1 req/s key. Verifying a TTL needs a handful of requests, not load.

## Open items

- **The criterion itself.** The rewording proposed at the top of this document
  needs the reviewer's agreement. Nothing else here depends on it.
- **Hit rate under load** is derived, not directly observed: with a path-scoped
  key and a 10 s TTL, an asset can miss at most `duration / TTL` times, which is
  what produced the load test's 0 % / 2 % / 100 % miss series by pool size.
- **Cache key cardinality is a standing risk, not a present fault.**
  `min_volume_usd` is a continuous value inside the key on two routes, and
  `/v1/assets` carries a seven-parameter key space. If hit rate degrades, the
  first lever is narrowing the key — not enlarging the 0.5 GB cluster.
- **A bimodal miss distribution was observed and is not explained.** Misses
  following a run of cache hits cost ~140 ms; misses following other misses cost
  ~80 ms. A plausible mechanism is that hits are served entirely by the gateway,
  so a stretch of them leaves the handler's ClickHouse connection idle and the
  next miss pays to re-establish it. **Seven data points, no attempt to falsify
  it.** Recorded because, if true, published miss latency would depend on the hit
  rate preceding it.
