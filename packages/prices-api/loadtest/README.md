# Load test — `GET /assets/{id}/price` SLO

§9 acceptance: **100 req/s sustained for 5 minutes** on `GET /assets/{id}/price`
→ **p95 < 200 ms**, **error rate < 0.1%**.

`price_load.js` is the k6 script (the §9 "script provided" deliverable). k6 exits
non-zero if either threshold is breached, so it doubles as a pass/fail gate.

## Authoritative run — deployed stage

The real SLO is measured against the deployed API Gateway stage (gateway cache +
real Lambda concurrency + cross-cloud mTLS to ClickHouse all in play):

```sh
k6 run packages/prices-api/loadtest/price_load.js \
  -e BASE_URL=https://<api-id>.execute-api.<region>.amazonaws.com/production \
  -e API_KEY=<key on a plan that permits 100 req/s — see below> \
  -e ASSET=native
```

Deploy-gated — requires the stack deployed (Phase 4 CDK) and a price row written
by the ingest/current-prices path.

> **The key matters, and the default plan will not do.** Task 0157 caps the
> CDK-managed plan (`pricing-api-free-production`) at **1 req/s with a 100 000/month
> quota**. Running this script against a key on that plan does not measure the
> system — it measures our own throttle, and reports a configuration artefact as
> an SLO result.
>
> **Provisioned for this task (0121):** plan `prices-production-loadtest-plan`
> (`i12bsj`) at **150 req/s, burst 300, 1 M/month**, key
> `prices-production-loadtest-key-20260819T114230Z` — see the registry table in
> `docs/runbooks/manual-api-key-tier.md`. The stage ceiling
> (`apiGatewayThrottleRate: 200`, burst 400) sits above the plan, so 100 req/s
> needs no CDK change. **The report must name the plan the key was on.**
> Wind the plan down after the milestone run; it is drift until then.

## Approximate run — local server

Exercises the handler + ClickHouse path only (no gateway cache, no Lambda cold
start), so local p95 is a **lower bound** on prod — useful to validate the script
and the query path, not to certify the prod SLO.

```sh
# 1. local ClickHouse (prod-pinned 26.3.10.60). Use a CLEAN volume so the
#    refreshable MV (which replaces current_prices every minute) is absent and
#    the manual seed persists — see the GOTCHA in seed.sql.
docker compose down -v && docker compose up -d clickhouse

# 2. seed one price row
curl --data-binary @packages/prices-api/loadtest/seed.sql \
  'http://localhost:8123/?database=prices'

# 3. run the server (plaintext local CH)
CLICKHOUSE_URL=http://localhost:8123 PORT=8080 \
  cargo run -p prices-api --bin serve --features local-server

# 4. load test (no API key needed locally)
k6 run packages/prices-api/loadtest/price_load.js \
  -e BASE_URL=http://localhost:8080 -e ASSET=native
```

## Knobs (env)

| var | default | meaning |
|-----|---------|---------|
| `BASE_URL` | `http://localhost:8080` | API base (include `/<stage>` for the gateway) |
| `ASSET` | (unset) | pin a single asset — the cache-dominated regime |
| `ASSETS` | 0120's 20-asset list | path to a JSON id pool (ignored when `ASSET` is set) |
| `API_KEY` | (none) | sent as `x-api-key` (required against the gateway) |
| `RATE` | `100` | requests/second |
| `WARMUP` | `30s` | low-rate phase before the measured window, excluded from thresholds |
| `DURATION` | `5m` | sustained measured duration |
| `VUS` / `MAX_VUS` | `50` / `200` | pre-allocated / max virtual users |

Smoke first with `-e RATE=20 -e DURATION=20s` before the full 5-minute run.

## Which regime to run, and why it matters more than the knobs

The gateway caches `/price` for 10 s **keyed on the path only**, so the pool size
is the only lever on the hit rate — no query parameter busts it. Over 300 s an
asset can miss at most 30 times, which fixes the arithmetic:

| pool | run with | max misses of 30 k | the p95 is really measuring |
|------|----------|--------------------|------------------------------|
| 1 asset | `-e ASSET=native` | ~30 (0.1 %) | the gateway cache |
| 20 assets (default) | *(nothing — it is the default)* | ~600 (2 %) | the AC scenario, still cache-dominated |
| 1000+ assets | `-e ASSETS=/path/pool.json` | 30 000 (100 %) | the real data path |

There is **no `X-Cache` header** on this API (verified 2026-08-20), so hit and
miss percentiles cannot be tagged per request. Run the regimes separately and
label each number with the regime it came from. Uniform sampling never warms a
hot key the way real traffic would, so the wide pool is a **worst case**, not a
typical one — report it as such rather than inventing a traffic distribution.

Generate a wide pool by walking the listing (any key works; it is one page per
200 assets):

```sh
node -e 'const f=async()=>{let c=null,out=[];do{const u=new URL(process.env.BASE_URL+"/v1/assets");u.searchParams.set("limit","200");if(c)u.searchParams.set("cursor",c);const r=await fetch(u,{headers:{"x-api-key":process.env.API_KEY}});const j=await r.json();out.push(...j.data.map(a=>a.contract_address||`${a.asset_code}:${a.issuer_address}`));c=j.cursor;await new Promise(s=>setTimeout(s,1100));}while(c);console.log(JSON.stringify(out));};f()' > /tmp/pool.json
```

## Before you run against production

The read path lands on the ClickHouse box **shared with soroban-block-explorer**,
and their load-test runbook records that our own OHLCV batch is bursty enough to
double their p95 on its own — the same is true in reverse. Two rules, both
borrowed from their harness:

1. **Check the box is quiet first**, and again after. A contaminated run cannot be
   corrected after the fact; discard and re-run it, and name the discarded run in
   the report rather than quietly dropping it.
2. **Schedule the window away from our own OHLCV batch**, and tell BE before you
   start. 100 req/s of cache misses is real load on infrastructure another team
   depends on.

## Reading the result

`dropped_iterations` is a threshold, not a statistic: any dropped iteration means
k6 could not keep the offered rate, so the run did **not** sustain 100 req/s and
its p95 is not the AC's number. Re-run with more `MAX_VUS`.

For the error-rate half, sample size is the whole argument: 30 000 requests with
zero errors puts the 95 % upper bound at 3/30000 = **0.01 %**, a 10× margin under
the 0.1 % AC (rule of three). Say that explicitly — it is the cleanest claim in
the report, and it only holds because all 30 000 samples are on the one endpoint
under test.

Cold starts are not in k6's output. Read `InitDuration` and `ConcurrentExecutions`
from Lambda, plus API Gateway `Latency` vs `IntegrationLatency`, over the
`phase:main` window — that split is what makes a "where did the p95 go" table
writable.
