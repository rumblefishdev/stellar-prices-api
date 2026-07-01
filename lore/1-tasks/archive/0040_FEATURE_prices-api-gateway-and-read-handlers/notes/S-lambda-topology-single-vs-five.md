---
title: "Decision (DECIDED: single): API Lambda topology — single axum Lambda (BE pattern) vs five per-route Lambdas"
type: synthesis
status: mature
spawns: []
tags: [api, lambda, axum, topology, cdk, reuse, adr-candidate]
links:
  - "../../../../../docs/prices-api-general-overview.md"
  - "../../../2-adrs/0006_runtime-framework-rust-axum.md"
history:
  - date: 2026-06-29
    status: seed
    who: oski
    note: >
      Captured the single-vs-five Lambda topology analysis after reviewing the
      BE (soroban-block-explorer) production API stack as a reuse donor and
      re-reading overview §4 / §2.1. Decision still open; recommendation = single.
  - date: 2026-06-30
    status: mature
    who: oski
    note: >
      DECIDED: single axum Lambda (copy the BE pattern wholesale). §2.1's
      "function per route group" wording is the deviation recorded in ADR 0008.
      Decision also recorded in the 0040 README history + the locked
      plan (G-implementation-plan.md).
---

> **DECISION (2026-06-30): SINGLE axum Lambda.** The body below is the
> comparison that led here; it is no longer "pending". Recorded in the 0040
> README history and G-implementation-plan.md.

# Decision (DECIDED: single): API Lambda topology

## The question

Should the prices REST API (task 0040) be deployed as **five Lambda
functions, one per route group** (overview §2.1's literal wording), or as
**one axum Lambda** behind `LambdaRestApi({proxy: true})` — the pattern the
BE repo (`soroban-block-explorer`) already runs in production?

## TL;DR recommendation

**Single axum Lambda (the BE pattern).** It satisfies 100% of the §4 endpoint
contract, lets us copy BE's proven `common/` kit + crate skeleton + CDK + CI
almost verbatim, and gives a better p95. The "five Lambdas" line is a §2.1
*topology preference*, not a §4 contract requirement — overriding it is an
**ADR-worthy, documented deviation**, not a spec violation.

## Key findings

1. **§4 ("API Endpoints Design") imposes zero topology constraints.** Every
   line of §4 is the HTTP contract — routes, query params, response JSON
   shapes, the Base64 keyset cursor. All 7 routes across 5 groups map cleanly
   onto a single axum `Router` under `/v1`. BE is a literal production proof:
   a single axum-on-Lambda function serving a *larger* multi-group REST API
   over ClickHouse-mTLS behind API Gateway.

2. **The "five Lambdas" idea comes from one §2.1 table cell, not §4.** That
   cell ("individual functions per route group … Rust/axum via
   `lambda_runtime`") is a deployment-topology choice. Two things weaken it as
   a hard mandate: the §1.1 architecture diagram shows a *single* "API handler
   functions" box (ambiguous, consistent with one function), and the cell says
   `lambda_runtime` where BE proved `lambda_http` is the right tool — so its
   wording is already known to be loosely binding on implementation detail.

3. **The one caveat that survives either topology:** true **per-key 100 req/s**
   (§2.1 / §7) needs the API Gateway **usage-plan** path regardless of Lambda
   count. BE only does in-app `X-API-Key` checks, so its gateway throttle is
   *global*, not per-key. Whichever topology we pick, per-key limiting is a
   gateway-config concern, independent of the handler count.

4. **For the §6 load SLO (single hot endpoint, e.g. `/price`), both topologies
   pass — single wins in practice.** A single-endpoint test warms exactly one
   function in *either* design, so raw `/price` p95 is topology-independent.
   What decides pass/fail are three topology-independent levers: the API
   Gateway 0.5 GB stage cache (dominant — collapses 100 req/s for a hot id to
   ~1 origin call/TTL-window), BE's in-process moka cache (keyed on chain
   head), and warm mTLS connection reuse + eager cold-start init. Single-Lambda
   hands us all three already-tuned; five Lambdas reimplement them in the
   `/price` handler with zero latency upside.

## Pros / cons

### Option A — Single axum Lambda (BE pattern)  ✅ recommended

**Pros**
- **Massive reuse.** Copy BE's `common/*` wholesale: keyset cursor +
  `Paginated` envelope, `ErrorEnvelope` (BE ADR 0008), 5-tier `Cache-Control`,
  `If-None-Match`→304-before-query, head-probe. Plus `main.rs`/`state.rs`/
  `config.rs`/`cache.rs` scaffolding, utoipa OpenAPI (`split_for_parts`
  keeps router+spec in sync), the `RustFunction` (cargo-lambda-cdk) cross-build
  + OIDC deploy job. Replaces most from-scratch Phase-1/Phase-5 work.
- **Better p95 (§6).** One warm moka cache + one warm mTLS pool means *any*
  traffic keeps the whole API warm; sub-ms repeat-id hits; CH shielded by
  moka + gateway cache (helps the <0.1% error target via `read_rows` Code-201
  discipline).
- **Less CDK + IaC surface.** One `RustFunction` + `LambdaRestApi({proxy:true})`
  vs five functions and per-route integration wiring.
- **Proven path.** De-risks the first axum binary in our codebase — BE runs
  this exact stack in prod.

**Cons**
- **Deviates from §2.1's literal "function per route group"** → needs an ADR to
  record the override.
- **No per-route isolation.** Shared memory sizing, IAM role, and blast radius;
  a bad deploy or a hot/heavy route affects all routes.
- **Shared cold-start.** One (slightly larger) bootstrap for all routes rather
  than independent per-route cold starts.

### Option B — Five Lambdas (literal §2.1)

**Pros**
- **Per-route independence:** scaling, memory/timeout sizing, IAM scoping, and
  cold-start isolation per route group — matches §2.1 verbatim, no ADR needed.
- **Smaller blast radius** per deploy/route.

**Cons**
- **Can't reuse BE's proxy router or its in-process cache** — the single
  biggest reuse loss. We'd reimplement the cache + mTLS hot path in (at least)
  the hot handlers.
- **~5× the CDK route wiring** and five build/deploy targets.
- **No latency upside for the §6 SLO** — the hot endpoint is one function in
  either design; isolation buys nothing for a point-lookup like `/price`.
- **Five independently-cold pools** can each go cold between runs → warm-up
  races on fresh deploys / after idle.

## What's NOT decided by this note

- **Per-key rate limiting** (§2.1/§7 100 req/s): a gateway usage-plan decision,
  orthogonal to topology — required either way.
- **Auth placement:** in-app key check (BE style, "armed" when secret set) vs
  gateway API-key. Lean BE-style in-app for portability, with the usage-plan
  providing the throttle.

## Next step

Lock the topology call. If single-Lambda: open an ADR ("API deployed as a
single axum Lambda, overriding overview §2.1 function-per-route-group") and
update this note `status: mature` + the task README implementation plan
(Step 1 crate layout collapses from 5 binaries to 1).
