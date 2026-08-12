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

> **The key matters, and no key that exists today will do.** Task 0157 caps the
> CDK-managed plan (`pricing-api-free-production`) at **1 req/s with a 100 000/month
> quota**. Running this script against a key on that plan does not measure the
> system — it measures our own throttle, and reports a configuration artefact as
> an SLO result.
>
> A 100 req/s run for 5 minutes is 30 000 requests, so even a generous monthly
> quota is a real constraint if the test is repeated. Provision a dedicated
> throttle-and-quota-headroom plan for the run (see
> `docs/runbooks/manual-api-key-tier.md`), and state in the report which plan the
> key was on. Tracked in task 0121.

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
| `ASSET` | `native` | asset identifier under test |
| `API_KEY` | (none) | sent as `x-api-key` (required against the gateway) |
| `RATE` | `100` | requests/second |
| `DURATION` | `5m` | sustained duration |
| `VUS` / `MAX_VUS` | `50` / `200` | pre-allocated / max virtual users |

Smoke first with `-e RATE=20 -e DURATION=20s` before the full 5-minute run.
