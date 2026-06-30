---
id: "0040"
title: "Prices API Gateway + Rust/axum read handlers — public REST endpoints with API-key auth, rate limit, response cache"
type: FEATURE
status: active
related_adr: ["0003", "0004", "0006", "0007", "0008"]
related_tasks: ["0011", "0038", "0039", "0045", "0047", "0072"]
tags: [layer-backend, priority-high, effort-large, api, lambda, axum, rust, aws, clickhouse, hetzner]
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md"
  - "../../2-adrs/0006_runtime-framework-rust-axum.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md"
  - "../backlog/0011_FEATURE_bootstrap-cdk-with-ssm-platform-lookups.md"
  - "../backlog/0047_RESEARCH_cross-tenant-throughput-verification-on-shared-hetzner-ch.md"
  - "./0038_FEATURE_prices-ledger-processor-lambda.md"
  - "./0039_FEATURE_prices-periodic-workers-lambda-set.md"
history:
  - date: 2026-05-18
    status: backlog
    who: oski
    note: >
      Drafted because the general-overview doc §2.1 and §4 specify
      a REST API surface (API Gateway + Rust/axum Lambda handlers)
      that no existing task covered. Sequenced AFTER 0038 and 0039
      in spirit — endpoints read from tables maintained by those
      tasks — but only technically blocked on 0011, since the
      handlers themselves can be developed against fixture data
      before live ingestion is wired.
  - date: 2026-05-18
    status: blocked
    who: oski
    by: ["0011"]
    note: >
      Moved to blocked/ — hard-blocked on 0011 (no API Gateway
      stack, no RDS, no Secrets Manager without it). Soft-blocked
      on 0038 / 0039 for meaningful end-to-end smoke tests, but
      handlers can be developed against fixture data once 0011
      is archived, so 0038 / 0039 are listed in related_tasks
      rather than `by:`.
  - date: 2026-05-18
    status: blocked
    who: okarcz
    note: >
      Redesign pending. Task 0044's research (synthesis §3) and
      ADR 0007 (proposed) call for **moderate** rewrite — endpoint
      contracts unchanged, but read handlers retarget from sqlx
      → `clickhouse` crate, queries adapt to per-source row shape
      (GROUP BY at read time), and the latency budget (<100 ms
      p95) needs re-validation against the public-internet hop to
      Hetzner CH. Hold rewrite until ADR 0007 accepted.
  - date: 2026-05-20
    status: blocked
    who: okarcz
    note: >
      ADR 0007 accepted via task 0045's closure (agreement record
      is the cross-team contract). Endpoint contracts confirmed
      unchanged from §4 of the overview doc; the rewrite is read-
      path-only (sqlx → clickhouse crate, FINAL or GROUP BY
      argMax/argMin on ReplacingMergeTree per ADR 0007 §3.3).
      <100ms p95 budget re-validation depends on task 0047's
      throughput numbers. Task stays blocked on 0011.
  - date: 2026-06-29
    status: active
    who: oski
    note: >
      **Unblocked — hard `by:` blocker 0011 has resolved.** Task 0011
      (CDK bootstrap + SSM platform lookups) is completed/archived, so the
      CDK app, IAM, and Secrets-Manager/SSM plumbing the API stack needs now
      exist. The soft deps cited at draft time are also both done: 0038
      (ledger-processor) ✅ archived and 0039 (periodic workers) ✅ archived,
      so `price_ohlcv_*` and `current_prices` have live producers for
      end-to-end smoke tests. Remaining caveats are NOT blockers: the read-path
      rewrite (sqlx → `clickhouse` crate per ADR 0007) is in-scope build work,
      and the <100ms p95 re-validation soft-depends on 0047 (backlog) but is
      explicitly informational for this functional-endpoints task. Moving
      blocked → active. **Constraints carried in:** stay local-first /
      prepare-not-deploy — handlers develop against fixture + live-CH reads;
      no AWS deploy / API Gateway apply without explicit approval.
  - date: 2026-06-30
    status: active
    who: claude
    note: >
      Persisted the implementation plan (notes/G-implementation-plan.md):
      endpoint→data-source map against the existing prices-clickhouse views,
      phased build (scaffold→shared-core→cheap-reads→new-query-reads→gateway→
      verify), and two locked decisions. (1) **Single axum Lambda** copied from
      BE crates/api (not five per-route) — §4 imposes no topology constraint and
      the load-test SLO is won by moka + gateway cache + warm mTLS, all BE
      already provides; §2.1 "function per route group" recorded as an
      ADR-worthy deviation (ADR pending). (2) **/price ships with sources={}**
      and zero price_xlm/change_24h_pct stubs — mv_current_prices leaves them at
      DEFAULT in v1; materializing them is producer-side, spawned as backlog
      task **0072**, after which /price flips to pass-through. Corrected the
      stale sqlx/RDS phrasing in Step 1 (superseded by ADR 0007 CH retarget).
---

# Prices API Gateway + Rust/axum read handlers

## Summary

Stand up the **public REST API** for prices-api: an AWS API
Gateway with API-key auth, 100 req/s per-key rate limiting, and a
0.5 GB built-in response cache with per-endpoint TTLs (per
overview §2.1), backed by Rust/axum Lambda handlers per
ADR 0006 §Decision. Implements all five endpoint groups listed
in overview §4 (Assets, Prices/OHLCV, Batch, Oracle, Backfill
Status), each handler deployed as an individual Lambda function
of 256–512 MB / 15 s timeout per §2.1.

## Context

Overview §1.1 (API Layer) and §2.1 (API Gateway row + API
handlers row) describe the read surface. §4 fixes the endpoint
list and response shapes:

| Section | Endpoint group |
|---------|----------------|
| §4.1    | `GET /assets`, `GET /assets/{id}` |
| §4.2    | `GET /assets/{id}/ohlcv`, `GET /assets/{id}/price` |
| §4.3    | `POST /prices/batch` |
| §4.4    | `GET /oracles/{id}` |
| §4.5    | `GET /backfill/status` |

§2.1 fixes the gateway-level non-functional requirements:

- **Auth**: API keys via API Gateway usage plans.
- **Rate limit**: 100 req/s per key.
- **Response cache**: 0.5 GB, per-endpoint TTLs.
- **Handler shape**: Rust / axum via `lambda_runtime`,
  256–512 MB, 15 s timeout, one function per route group.

ADR 0006 §Decision fixes the framework choice (axum) and the
runtime choice (`provided.al2` via `lambda_runtime`); this task
is the first axum binary in the codebase (the Ledger Processor in
0038 is `lambda_runtime` only — no HTTP layer).

## Implementation Plan

> **Authoritative plan:** [`notes/G-implementation-plan.md`](notes/G-implementation-plan.md)
> (endpoint→data-source map, phased build, single-Lambda decision, the
> `/price` stub decision → task 0072). The steps below are the original draft,
> kept for history; where they differ from the G-note, the G-note wins. In
> particular the `sqlx`/RDS phrasing predates the ADR 0007 ClickHouse retarget
> and the single-Lambda topology decision.

### Step 1: Workspace + crate layout

Add `packages/api/` as the umbrella for handler crates, sharing a
library crate `packages/api-core/` for:

- ClickHouse access via `prices_clickhouse::mtls::client_from_lambda_env`
  (mTLS to Hetzner CH per ADR 0007) — **not** `sqlx`/RDS (superseded).
- Asset identifier parsing per §4.1 (`{code}:{issuer}`,
  `{contract_address}`, `native`).
- Cursor encode/decode for §4.1's keyset pagination (Base64 JSON
  with sort-column value + asset id).
- Common error → HTTP response mapping.
- Per-endpoint response cache key + TTL hint headers (API
  Gateway honours these for the §2.1 0.5 GB cache).

**DECISION (locked 2026-06-30): a SINGLE axum Lambda serving all
routes**, copied from BE `crates/api` — NOT five per-route binaries.
The original draft below proposed a 5-binary split (mirroring §2.1's
"function per route group"); that is **superseded**. One binary crate
`crates/prices-api` with one axum router and a module per route group
(`assets`, `ohlcv`/`price`, `batch`, `oracles`, `backfill`). Modules
stay independent so a hot endpoint can be split into its own Lambda
later without a rewrite (escape hatch + reserved-concurrency hook). The
§2.1 "function per route group" wording is the deviation recorded in
[ADR 0008](../../2-adrs/0008_single-axum-lambda-for-prices-api.md). See
[`notes/G-implementation-plan.md`](notes/G-implementation-plan.md)
and [`notes/S-lambda-topology-single-vs-five.md`](notes/S-lambda-topology-single-vs-five.md).

~~Recommended split (5 binaries, one per §4.x sub-section)~~ — superseded:

- ~~`packages/api/assets-handler` — §4.1~~
- ~~`packages/api/ohlcv-handler` — §4.2~~
- ~~`packages/api/batch-handler` — §4.3~~
- ~~`packages/api/oracles-handler` — §4.4~~
- ~~`packages/api/backfill-status-handler` — §4.5~~

### Step 2: Endpoint implementations

For each handler, transcribe the §4 spec verbatim:

- **§4.1 `GET /assets`**: keyset pagination with the §4.1 cursor
  format (Base64-encoded `{sort_col, id}` JSON). Sort indexes
  from §3.3 (`idx_current_prices_volume_24h`, etc.) drive the
  query plan. Supports `?type` / `?search` / `?sort` / `?order`
  / `?cursor` / `?limit` (max 200).
- **§4.1 `GET /assets/{asset_identifier}`**: single-asset
  lookup; asset_identifier parsing per §4.1 bullets.
- **§4.2 `GET /assets/{id}/ohlcv`**: timeframe → granularity
  mapping per §4.2 table; `?start` / `?end` overrides;
  `?base_currency` switch; emits `backfill_note` when
  `?timeframe=all` is requested and the backfill hasn't reached
  the asset's inception.
- **§4.2 `GET /assets/{id}/price`**: read the `current_price_usd`
  view (mTLS CH). Real fields: `price_usd`, `vwap_24h`,
  `volume_24h_usd`, `updated_at`. **`sources` ships as `{}`** and
  `price_xlm` / `change_24h_pct` as zero stubs — `mv_current_prices`
  leaves them at DEFAULT in v1; materializing them is **task 0072**,
  after which this handler flips to pass-through (see G-note).
- **§4.3 `POST /prices/batch`**: read `current_prices` for the
  posted asset list; cap batch size (size TBD at impl — pick
  something defensible, e.g. 100).
- **§4.4 `GET /oracles/{asset}`**: read `oracle_prices` per §3.4
  partitioning; return latest per-oracle row.
- **§4.5 `GET /backfill/status`**: read `backfill_progress`
  (§3.5); shape per §5.6 "GET /backfill/status Freshness"
  sub-section.

### Step 3: API Gateway + caching (depends on 0011)

In `infra/aws-cdk/`:

- New `ApiGatewayStack` defining a REST API (not HTTP API —
  REST is needed for the §2.1 response cache, which HTTP API
  doesn't support).
- API key + usage plan with **100 req/s** rate limit per §2.1.
- **0.5 GB response cache** enabled at the stage level with
  per-endpoint TTL configuration per §2.1. TTLs are a design
  choice the impl makes — short for `/price` and `/batch` (the
  most volatile), longer for `/ohlcv` history windows and
  `/backfill/status`. Document the chosen TTLs in the task's
  done notes.
- Request validation: asset identifier patterns per §7
  (G-address 56 chars, C-address 56 chars).
- HTTPS-only per §7.
- Lambda integration per route → Rust binary; each Lambda
  256–512 MB, 15 s timeout per §2.1.

### Step 4: Auth + rate-limit integration tests

- API key required on every request → 401/403 without one.
- `> 100 req/s` from one key → 429.
- Response cache: a second identical GET within TTL serves a
  cached response (measured by response time and absence of
  Lambda invocation in CloudWatch).

### Step 5: Tests

- Unit: per-handler, fixture DB + golden response per endpoint
  shape in §4.
- Integration: API Gateway + Lambda stack stood up against a
  staging account (or LocalStack equivalent); run a smoke
  suite hitting each endpoint with valid + invalid inputs.
- Pagination test for §4.1: walk a 250-asset fixture set with
  `limit=50` cursor pagination and assert no row appears twice
  or is skipped.

### Step 6: OpenAPI + docs hosting

Per §2.1's S3 row and §8 Tech Stack: emit an OpenAPI 3.0 spec
from the axum routes (e.g. via `aide` or hand-authored), host it
+ Swagger UI on S3 + CloudFront. Out of scope for the first cut
if it becomes load-bearing — split into a follow-up task at
that point.

## Acceptance Criteria

- [ ] **Single** Rust/axum Lambda binary (`crates/prices-api`,
      all routes) built against `provided.al2`/PROVIDED_AL2023,
      deployed via CDK from 0011's infra app. (Topology decision
      2026-06-30 — single, not five; see G-note.)
- [ ] REST API Gateway with API-key auth, 100 req/s usage plan,
      0.5 GB response cache, per-endpoint TTLs documented.
- [ ] All §4 endpoints implemented per the response shapes shown
      in the overview doc, including `backfill_note` on
      `?timeframe=all` and `sources` JSONB expansion.
- [ ] Keyset pagination for `GET /assets` correctly walks a
      250-row fixture (integration test) with no duplicates or
      skipped rows across `?cursor` requests.
- [ ] Asset identifier validation rejects malformed G/C-addresses
      with 400; missing API key returns 403; over-rate returns
      429.
- [ ] Cache hit on second identical GET within TTL is observable
      in CloudWatch (no Lambda invocation).
- [ ] Per-handler Lambda memory ≤512 MB; timeout ≤15 s; p95
      latency target tracked (overview §6 has a <100 ms p95
      goal — informational here, hard target lives in §6's
      perf task).
- [ ] Docs: README in `packages/api/` describing the route
      ownership, the CDK stack layout, and the cache TTL
      decisions.

## Blocked on

- **0011** — RDS, CDK Lambda + API Gateway stacks, Secrets
  Manager for DB credentials. Without 0011, no place to deploy
  and no DB to read. Note: not technically blocked on 0038 /
  0039 — handlers can be developed against fixture data — but
  end-to-end smoke tests need at least 0038 (so `price_ohlcv`
  has rows) and 0039 (so `current_prices` has rows) running
  in the same env to be meaningful.

## Out of scope

- WebSocket / streaming endpoints — not in §4.
- Auth beyond API keys (no OAuth, no Cognito) — §7 specifies
  API keys only.
- OpenAPI Swagger UI + S3/CloudFront docs hosting at the level
  of polish §2.1 implies — keep the OpenAPI spec emission in
  scope (cheap), split the hosting/CloudFront stack to a
  follow-up if it grows.
- Performance tuning to hit §6's <100 ms p95 target — that is
  a separate Tranche 3 perf task; this task ships functional
  endpoints, perf comes after.
- Write endpoints — the public surface is read-only per §4 /
  §7. Internal admin endpoints (if any) are not in §4 and not
  in scope here.

## Notes

- **Lambda topology — DECIDED 2026-06-30: SINGLE axum Lambda** (copy the BE
  pattern wholesale). See
  [`notes/S-lambda-topology-single-vs-five.md`](notes/S-lambda-topology-single-vs-five.md)
  for the full comparison and
  [`notes/G-implementation-plan.md`](notes/G-implementation-plan.md) for the
  locked plan. §4 imposes no topology constraint; single wins on §6 p95 and lets
  us copy BE's `common/` kit + CDK + CI. §2.1's "function per route group"
  wording is the deviation recorded in
  [ADR 0008](../../2-adrs/0008_single-axum-lambda-for-prices-api.md). Per-key
  100 req/s needs the gateway usage-plan regardless.
- This is the **first axum binary** in the codebase per
  ADR 0006 §Decision; conventions for axum-on-`lambda_runtime`
  packaging, routing layout, and error mapping established here
  will be reused by any future API handler.
- ~~The §2.1 "individual functions per route group" mandate is
  deliberate: five handler binaries, not one mono-Lambda.~~
  **Superseded 2026-06-30:** we ship a single axum Lambda (BE pattern).
  Per-route cold-start/memory/IAM isolation was the only upside, and it
  doesn't help the single-endpoint load-test SLO; the escape hatch
  (split a hot route out later) preserves it if ever needed.
- Per-endpoint cache TTLs are a meaningful design call —
  document them in the done notes, not just in CDK. A future
  reviewer looking at the §2.1 row needs to know what TTLs were
  chosen and why.
- Asset identifier parsing is shared with 0038's extractor side
  (both must agree on what counts as "the same asset"). Land
  the parser in `packages/api-core/` and have 0038 / 0039
  depend on it once stable, rather than duplicating.
- **Grain-selection ownership (from 0061 §12.6, decided 2026-06-15):**
  the `price_usd_at(id, ts)` point-lookup endpoint owns **view-picks** —
  map `ledger → ts → finest-retained grain` (`_1m` ≤7d, `_15m` ≤30d,
  else `_1h`/`_1d`) and read `close_usd`. The in-cluster views stay
  **caller-passes** (per-grain: `prices.price_usd_series` / `_1h`), so
  this retention-aware routing lives in the API layer, not the views.
  See `packages/prices-clickhouse/schema/views.sql` header + 0061 note
  §12.6.
