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
  -e API_KEY=<key on a plan that permits 100 req/s — see below>
```

No `-e ASSET=` here, deliberately: that flag pins one asset and turns the run
into a measurement of the **gateway cache**, not of the AC scenario (see the
regime table below). The default 20-asset pool is the AC scenario. Copying a
command that pins an asset and reporting its p95 as the milestone number is
exactly the "configuration artefact as an SLO result" failure this file warns
about two paragraphs down.

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
| `WARMUP` | `30s` | full-rate phase before the measured window, excluded from thresholds; `0` drops it |
| `DURATION` | `5m` | sustained measured duration |
| `VUS` / `MAX_VUS` | `50` / `200` | pre-allocated / max virtual users |
| `SETUP_TIMEOUT` | `180s` | ceiling on the pool probe — raise it for pools well past 2000 |
| `PROBE_BATCH` | `10` | concurrent probes per batch in `setup()` |

Smoke first with `-e RATE=20 -e DURATION=20s` before the full 5-minute run.

## Which regime to run, and why it matters more than the knobs

The gateway caches `/price` for 10 s **keyed on the path only**, so the pool size
is the only lever on the hit rate — no query parameter busts it. Over 300 s an
asset can miss at most 30 times, which fixes the arithmetic:

| pool | run with | max misses of 30 k | the p95 is really measuring |
|------|----------|--------------------|------------------------------|
| 1 asset | `-e ASSET=native` | ~30 (0.1 %) | the gateway cache |
| 20 assets (default) | *(nothing — it is the default)* | ~600 (2 %) | the AC scenario, still cache-dominated |
| ≥ 2000 assets | `-e ASSETS=./pool-wide.json` | 30 000 (100 %) | the real data path |

Production held **3543** listable assets on 2026-09-02, and a 200-asset random
sample of them returned 200 on `/price` for **every** id — so the wide regime is
viable at 3.5× the `RATE × TTL` margin, and the pool needs no hand-curation.

⚠️ **The wide regime needs `pool ≫ RATE × TTL`, not "a big number".** Selection
is deterministic round-robin, so a pool of exactly `RATE × TTL` — 1000 at
100 req/s against the 10 s TTL — comes back to each asset every 10.0 s, right on
the expiry boundary. Hit vs miss becomes a timing coin-flip and the p95 is
neither number. 2000 gives 2× margin at 100 req/s; scale it if you change
`RATE`.

There is **no `X-Cache` header** on this API (verified 2026-08-20), so hit and
miss percentiles cannot be tagged per request. Run the regimes separately and
label each number with the regime it came from. Uniform sampling never warms a
hot key the way real traffic would, so the wide pool is a **worst case**, not a
typical one — report it as such rather than inventing a traffic distribution.

Generate a wide pool by walking the listing (any key works; it is one page per
200 assets):

```sh
node -e 'const id=a=>a.contract_address||(a.issuer_address?`${a.asset_code}:${a.issuer_address}`:"native");const f=async()=>{let c=null,out=[];do{const u=new URL(process.env.BASE_URL+"/v1/assets");u.searchParams.set("limit","200");if(c)u.searchParams.set("cursor",c);const r=await fetch(u,{headers:{"x-api-key":process.env.API_KEY}});const j=await r.json();out.push(...j.data.map(id));c=j.cursor;await new Promise(s=>setTimeout(s,1100));}while(c);console.log(JSON.stringify([...new Set(out)]));};f()' > packages/prices-api/loadtest/pool-wide.json
```

⚠️ **The native asset is why that `id()` helper is not just `code:issuer`.** XLM
comes back from the listing as `asset_type: "classic"` with **both**
`contract_address` and `issuer_address` empty, so the obvious expression yields
`XLM:` — which the API answers with **400**, not 404. `setup()` aborts on any
non-404 (deliberately: a throttled key must not read as dead assets), so a pool
built without this special case fails the run *at setup*, with a status that
points at the API rather than at the pool file. Measured 2026-09-02.

`pool-wide.json` is gitignored — it is a snapshot of production, not a fixture.
Regenerate it before a run and record the count in the report; the 20-asset
conformance list stays committed because the AC scenario must be reproducible.

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

## Run day — the three commands, in order

Export the environment once (`.env.local` at the repo root holds `BASE_URL` and
`API_KEY`; confirm the key is the 0121 loadtest one, not a free-tier key):

```sh
set -a && . ./.env.local && set +a && node -e 'console.log(process.env.BASE_URL)'
```

Regenerate the pool (see the generator above), then run the three regimes. The
flags are not optional: `--summary-trend-stats` is what produces the p50/p99 the
AC asks for — the default stats stop at p95 — and `--summary-export` is the raw
artefact the report cites.

```sh
S='avg,min,med,max,p(50),p(90),p(95),p(99)'
K="packages/prices-api/loadtest/price_load.js"

# 1/3 — cache regime (upper bound)
k6 run $K -e BASE_URL="$BASE_URL" -e API_KEY="$API_KEY" -e ASSET=native \
  --summary-trend-stats="$S" --summary-export=loadtest-cache.json; echo "exit=$?"

# 2/3 — THE AC SCENARIO: 20-asset conformance pool, no -e ASSET/ASSETS
k6 run $K -e BASE_URL="$BASE_URL" -e API_KEY="$API_KEY" \
  --summary-trend-stats="$S" --summary-export=loadtest-ac.json; echo "exit=$?"

# 3/3 — wide pool, the real data path
k6 run $K -e BASE_URL="$BASE_URL" -e API_KEY="$API_KEY" -e ASSETS=./pool-wide.json \
  -e PROBE_BATCH=25 \
  --summary-trend-stats="$S" --summary-export=loadtest-wide.json; echo "exit=$?"
```

`exit=0` means every threshold held. Leave ≥ 60 s between regimes so the 10 s
gateway cache and the warm Lambda pool from the previous run are not carried into
the next one's warmup.

⚠️ **`ASSETS` is resolved relative to the *script*, not to your shell's cwd** —
k6's `open()` works that way, which is also why the built-in default is the
`../../../tools/...` walk-up. From anywhere in the repo, `./pool-wide.json` is
therefore the correct value and `./packages/prices-api/loadtest/pool-wide.json`
is not.

⚠️ **`--summary-export` inverts the sense of a threshold.** In the exported JSON
`"p(95)<200": false` means the threshold was **not** breached, i.e. it *passed*.
Cite the process **exit code** in the report, not the boolean — it is the one
number that cannot be read backwards.

**Timing — pass `-e PROBE_BATCH=25` on the wide regime.** `setup()` probes every
asset in the pool. Measured 2026-09-02 on the 3543-asset pool: **59 s at
`PROBE_BATCH=25`**, against the 180 s `SETUP_TIMEOUT` default. At the default
batch of 10 the same probe extrapolates to ~150 s — inside the timeout, but with
too little margin to spend a coordinated BE window on. It dropped 39 assets on
404 (no price row) and named every one, leaving 3504 under test.

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
