---
title: "0040 implementation plan — endpoint→data-source map + phased build"
type: G
status: mature
spawned_from: "0040"
date: 2026-06-30
---

# 0040 Implementation Plan

Authoritative build plan for the Prices API. Supersedes the inline
"Implementation Plan" in the task README where they differ (the README predates
the ADR 0007 ClickHouse retarget and the single-Lambda decision; the stale
`sqlx`/RDS phrasing there is corrected but this note is the source of truth).

## Topology decision (locked for this milestone)

**Single axum Lambda**, all routes, copied from the BE
(`soroban-block-explorer`) `crates/api` skeleton — NOT five per-route Lambdas.
Rationale and trade-offs in [`S-lambda-topology-single-vs-five.md`](S-lambda-topology-single-vs-five.md).
Drivers: §4 imposes no topology constraint; the load-test SLO
(`100 req/s, p95<200ms, err<0.1%` on `/price`) is won by the in-process (moka)
cache + gateway response cache + warm mTLS pool — all of which BE already
implements and we copy wholesale. §2.1's literal "function per route group"
is the deviation recorded in [ADR 0008](../../../2-adrs/0008_single-axum-lambda-for-prices-api.md). Escape hatch:
handlers are modular (one module per route), so a hot endpoint can be split into
its own Lambda later without a rewrite; add a reserved-concurrency hook on the
function.

## Data layer that already exists (`packages/prices-clickhouse`)

- **Tables** (`init.sql`): `assets`, `price_ohlcv_{1m,15m,1h,4h,1d,1w,1M}`,
  `current_prices`, `asset_supply`, `oracle_prices`, `backfill_progress`,
  `backfill_sdex_ledgers`, `discovery_state`.
- **Read views** (`views.sql`): `current_price_usd`, `price_usd_series(_1h)`,
  `usd_reference(_1h)`, `identity_by_contract`.
- **MTLS client**: `prices_clickhouse::mtls::client_from_lambda_env(database)`
  (reads `MTLS_SECRET_NAME` + `CH_DOMAIN`; `api` CN → `prices_reader`).

## Endpoint → data-source map

Legend: 🟢 view/table ready, wrap it · 🟡 data exists, query is new · 🔴 new query+logic

| Endpoint | Feeds from | Status | New work at API layer |
|---|---|---|---|
| `GET /assets` | `assets` (+ `current_price_usd` for sort) | 🟡 | cursor pagination, filter, sort (CH idiom — not naive `ORDER BY (volume, id)`) |
| `GET /assets/{id}` | `assets` | 🟡 | single-row lookup; natural id → `asset_id` |
| `GET /assets/{id}/price` ⭐ | `current_price_usd` view | 🟢/🟡 | view gives `price_usd`+`updated_at` directly; `sources`/`price_xlm`/`change_24h_pct` stubbed (see below) |
| `GET /assets/{id}/ohlcv` | `price_ohlcv_{grain}` | 🔴 | `timeframe→granularity` (view-picks), `FINAL`/argMax dedup, `backfill_note` |
| `POST /prices/batch` | `current_price_usd` view | 🟡 | batch lookup (`WHERE id IN (…)`), cap size (~100) |
| `GET /oracles/{id}` | `oracle_prices` | 🟡 | latest-per-oracle lookup; table is retention-capped → handle empties |
| `GET /backfill/status` | `backfill_progress` | 🟡 | read both stream rows; shape `sdex`/`soroban_amm` JSON per §5.6 |
| `GET /health` | — | 🟢 | already stubbed in `api-gateway-stack.ts` |

The heavy lifting (USD collapse, identity resolution, dedup) is baked into the
views. The two real new-query items are **`/assets` listing** and
**`/ohlcv` grain selection**.

## `/price` field-population decision (D now → 0072 later)

`mv_current_prices` (`current.sql`, sole writer of `current_prices`) populates
only the v1 subset. For the `/price` response:

| field | source | v1 status |
|---|---|---|
| `price_usd`, `volume_24h_usd`, `vwap_24h`, `updated_at` | MV | ✅ real |
| `price_xlm`, `change_24h_pct` | — | ❌ DEFAULT `0` (stub) |
| `sources` | — | ❌ DEFAULT `''` → API returns `{}` (stub) |

**Decision (2026-06-30):** ship `/price` now with the real scalar fields and
`sources: {}` (+ zero `price_xlm`/`change_24h_pct`), documented as stubs. Do NOT
derive them per-request — a 24h `GROUP BY source` scan on the load-test endpoint
would fight the p95 SLO. Materializing them is producer-side work, tracked as
**task 0072** (extend the MV); when it lands, `/price` flips to pass-through.

## Phased plan

```
Phase 0 — Scaffold (copy BE)
  new crate crates/prices-api (axum + lambda_http); copy BE common/:
  cursor, pagination, extractors, errors, cache_control, conditional(ETag/304), head
  AppState { ch: clickhouse::Client } via prices_clickhouse mtls
  wire GET /health end-to-end

Phase 1 — Shared core
  in-app X-API-Key auth (ct_eq, copy BE auth/mod.rs); moka cache;
  identity parser {asset_identifier}→asset_id; utoipa openapi + bin/extract_openapi.rs

Phase 2 — Cheap reads (views exist) 🟢🟡
  GET /assets/{id}/price (⭐ load-test target), POST /prices/batch,
  GET /assets/{id}, GET /oracles/{id}, GET /backfill/status
  → /price live → run load test early

Phase 3 — New-query reads 🔴
  GET /assets (cursor+filter+sort), GET /assets/{id}/ohlcv (grain select+dedup+backfill_note)

Phase 4 — Gateway + perf (wins the SLO)
  attach /v1 routes to api-gateway-stack skeleton (LambdaRestApi proxy);
  usage plan + API key → per-key 100 req/s;
  stage response cache 0.5GB, per-endpoint TTLs (/assets 60s, /ohlcv 60s,
  /price 15s, /backfill 30s, batch uncached); reserved-concurrency hook

Phase 5 — Verify
  per-endpoint integration tests vs prod-pinned CH 26.3.10.60;
  k6: 100 req/s × 5min on /price → p95<200ms, err<0.1%; CloudWatch visibility
```

**Value order:** 0 → 1 → **2 (gets `/price` live, load-test early)** → 4 (prove
SLO) → 3 (harder list/ohlcv) → 5.

## Grain-selection ownership (from 0061 §12.6)

`/ohlcv` + the `price_usd_at(id, ts)` point-lookup own **view-picks**:
`ledger → ts → finest-retained grain` (`_1m` ≤7d, `_15m` ≤30d, else `_1h`/`_1d`).
In-cluster views stay **caller-passes**. See `views.sql` header + 0061 §12.6.

## Pending sub-steps before "done"

- ~~Write the ADR formalizing the single-Lambda deviation from §2.1.~~ ✅ ADR 0008.
- Confirm batch-size cap for `/prices/batch`.
- `<100ms p95` (§6) re-validation soft-depends on 0047 — informational here.

## Constraints carried in

Local-first / prepare-not-deploy: handlers develop against fixture + live-CH
reads; no AWS deploy / API Gateway apply without explicit approval.
