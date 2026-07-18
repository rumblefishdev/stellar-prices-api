---
id: "0008"
title: "Single axum Lambda for the Prices API (deviation from §2.1 'function per route group')"
status: accepted
deciders: [okarcz]
related_tasks: ["0040", "0072"]
related_adrs: ["0006", "0007"]
tags: [architecture, api, lambda, axum, topology, cdk, reuse, block-explorer-shared, deviation]
links:
  - "../../docs/prices-api-general-overview.md"
  - "../1-tasks/active/0040_FEATURE_prices-api-gateway-and-read-handlers/notes/S-lambda-topology-single-vs-five.md"
  - "../1-tasks/active/0040_FEATURE_prices-api-gateway-and-read-handlers/notes/G-implementation-plan.md"
history:
  - date: 2026-06-30
    status: accepted
    who: okarcz
    note: >
      Drafted post-factum to formalize the topology decision made while planning
      task 0040: the public Prices API ships as a SINGLE axum Lambda serving all
      route groups, copied from the Block Explorer (soroban-block-explorer)
      crates/api skeleton — a deliberate deviation from overview §2.1's "API
      handlers — individual functions per route group". Decision also recorded in
      the 0040 README history, G-implementation-plan.md, and the topology
      synthesis note (S-lambda-topology-single-vs-five.md).
---

# ADR 0008: Single axum Lambda for the Prices API

**Related:**
- [Task 0040: Prices API Gateway + Rust/axum read handlers](../1-tasks/active/0040_FEATURE_prices-api-gateway-and-read-handlers/README.md)
- [Task 0072: Materialize current_prices deferred columns](../1-tasks/backlog/0072_FEATURE_materialize-current-prices-deferred-columns.md)
- [ADR 0006: Runtime framework — Rust + axum on Lambda](0006_runtime-framework-rust-axum.md)
- [ADR 0007: Live data sink on shared Hetzner ClickHouse](0007_live-data-sink-on-shared-hetzner-clickhouse.md)

---

## Context

The general-overview design doc describes the API read surface in two places:

- **§4** fixes the HTTP contract — seven endpoints across five groups (Assets,
  Prices/OHLCV, Batch, Oracle, Backfill Status), with exact request/response
  shapes. §4 says **nothing** about how the handlers are packaged or deployed.
- **§2.1** describes the architecture and includes the line *"API handlers —
  individual functions per route group"*, i.e. one Lambda per §4.x group (five
  Lambdas).

ADR 0006 already commits us to Rust + axum on Lambda, sharing the funded Block
Explorer (BE) Rust workspace. While planning 0040 we reviewed BE's production API
stack (`soroban-block-explorer/crates/api`) as a reuse donor and found it is a
**single** axum Lambda serving all routes via `lambda_http`, with a mature,
already-tuned hot path: an in-process `moka` cache, a warm hyper/mTLS connection
pool, cold-start eager init, `ct_eq` API-key auth, keyset pagination, conditional
GET (ETag/304), and a `common/` kit we can copy wholesale.

A load-test SLO is also in play (task 0040 / overview): *100 req/s sustained for
5 min on `GET /assets/{id}/price` → p95 < 200 ms, error rate < 0.1%*. This forces
the topology question: does §2.1's five-Lambda wording buy us anything that the
single-Lambda BE pattern does not?

The full comparison lives in
[S-lambda-topology-single-vs-five.md](../1-tasks/active/0040_FEATURE_prices-api-gateway-and-read-handlers/notes/S-lambda-topology-single-vs-five.md).

---

## Decision

The Prices API ships as a **single axum Lambda** (`crates/prices-api`) serving
**all** route groups, copied from BE's `crates/api` skeleton. Routes are
organized as one axum router with a module per group (`assets`, `ohlcv`/`price`,
`batch`, `oracles`, `backfill`).

This is a deliberate, recorded **deviation from §2.1's "individual functions per
route group"**. §4 (the actual contract) is met identically either way.

Per-key rate limiting (100 req/s) and the 0.5 GB response cache remain at the API
Gateway layer (usage plan + stage cache) — independent of Lambda count.

**Escape hatch:** modules stay independent so a single hot endpoint can later be
extracted into its own Lambda without a rewrite, and a reserved-concurrency hook
is left on the function for capacity isolation if growth demands it.

---

## Rationale

- **§4 imposes no topology constraint.** The five-Lambda guidance is only §2.1
  architecture preference, not a contract requirement.
- **The load-test SLO is won by caching, not function count.** For a
  single-endpoint test, only the `/price` handler receives traffic — that is one
  function in *both* topologies, so raw p95 is equivalent. What actually drives
  p95/error-rate down is the API Gateway response cache + the in-process `moka`
  cache + warm mTLS reuse — all of which BE already implements and we copy for
  free in the single-Lambda design.
- **Maximum reuse, minimum new code.** Copying BE's `common/` kit, CDK, and CI
  means less bespoke code to write and a proven hot path — directly lowering the
  risk of missing the SLO and the < 0.1% error target (fewer moving parts).
- **Operational simplicity at our scale.** One warm pool keeps the whole API
  warm under realistic mixed traffic; five pools let rarely-hit routes go cold.
- **Reversible.** The only real upside of five Lambdas — per-route concurrency /
  memory / IAM isolation — is preserved by the escape hatch, to be exercised only
  if a specific endpoint becomes a measured hot spot.

---

## Alternatives Considered

### Alternative 1: Five Lambdas, one per §4.x route group (§2.1 literal)

**Description:** Separate Lambda binary + function per route group, each behind
its own API Gateway integration.

**Pros:**
- Per-route concurrency, memory sizing, and IAM scoping isolation.
- A heavy endpoint (e.g. `/ohlcv`, `/batch`) cannot starve `/price` of the
  shared account concurrency pool.
- Literal match to §2.1 wording (no deviation to document).

**Cons:**
- The cache + mTLS hot path (the thing that wins the SLO) must be re-ported into
  each handler — five times the bespoke code, with drift risk, for **zero**
  latency upside on a single-endpoint load test.
- Five independent warm pools; rarely-hit routes cold-start under real traffic.
- Cannot copy BE's single-Lambda skeleton wholesale; more integration surface.
- More CDK/CI moving parts and more functions to operate.

**Decision:** REJECTED — the isolation benefit is irrelevant to the `/price`
load-test SLO (the lightest endpoint) and is recoverable later via the escape
hatch, while the cost (re-porting the hot path 5×, losing BE reuse) is paid up
front.

### Alternative 2: Single Lambda, but enforce per-key limits in-app (BE proxy style)

**Description:** Single Lambda as decided, but with API Gateway in proxy mode and
API-key gating done in-app (as BE does), rather than via a Gateway usage plan.

**Pros:**
- Even closer to BE; one fewer Gateway construct.

**Cons:**
- Gateway proxy mode makes API keys non-gating at the edge, so the 100 req/s
  **per-key** throttle (§2.1/§7) would have to be reimplemented in-app — weaker
  and more code than the managed usage-plan throttle.

**Decision:** PARTIALLY REJECTED — adopt the single Lambda, but keep per-key
rate limiting at the **API Gateway usage-plan** layer (not in-app) so the
100 req/s-per-key requirement is enforced by managed infrastructure. In-app
`ct_eq` key validation from BE is still copied as defense-in-depth.

---

## Consequences

### Positive

- One crate (`crates/prices-api`) to build, test, deploy, and operate.
- BE's `common/` kit + CDK + CI copied wholesale → faster delivery, proven hot
  path, lower SLO risk.
- One warm pool keeps the entire API warm; better realistic p95.
- Simpler CDK (one function + integration) than five.

### Negative

- Deviates from §2.1 wording — this ADR is the record of that deviation.
- All routes share one account-concurrency pool; a future heavy endpoint could
  contend with `/price` (mitigated by reserved concurrency + the split escape
  hatch).
- No per-route memory/IAM tailoring until/unless a route is split out.

---

## References

- overview §2.1 (API handlers row), §4 (endpoint contracts), §6 (p95 target), §7
- [S-lambda-topology-single-vs-five.md](../1-tasks/active/0040_FEATURE_prices-api-gateway-and-read-handlers/notes/S-lambda-topology-single-vs-five.md) — full comparison
- [G-implementation-plan.md](../1-tasks/active/0040_FEATURE_prices-api-gateway-and-read-handlers/notes/G-implementation-plan.md) — locked plan
- `soroban-block-explorer/crates/api` — the single-Lambda reuse donor
