# prices-api

Public Prices REST API — a **single axum Lambda** serving all route groups, on
ClickHouse over mTLS. Implements task **0040** (overview §4). Topology decision:
[ADR 0008](../../lore/2-adrs/0008_single-axum-lambda-for-prices-api.md) (single
Lambda, not five per-route — copied in skeleton from BE's `crates/api`).

## Layout

```
src/
├── lib.rs        # app(state) -> Router — the single source of routes (bin + tests share it)
├── main.rs       # Lambda entrypoint (feature `lambda`): cold-start CH client + lambda_http::run
├── config.rs     # AppConfig::from_env()
├── state.rs      # AppState { ch: Option<clickhouse::Client> } (Arc-backed, cloned per request)
├── common/       # framework primitives (errors, cache_control; cursor/conditional later)
└── ops/          # GET /health (non-versioned, auth-exempt)
tests/health.rs   # in-process oneshot smoke test
```

## Route ownership (target — §4)

| Path | Status |
|------|--------|
| `GET /health` | ✅ Phase 0 |
| `GET /v1/assets`, `GET /v1/assets/{id}` | Phase 3 / Phase 2 |
| `GET /v1/assets/{id}/price` | Phase 2 (load-test target) |
| `GET /v1/assets/{id}/ohlcv` | Phase 3 |
| `POST /v1/prices/batch` | Phase 2 |
| `GET /v1/oracles/{id}` | Phase 2 |
| `GET /v1/backfill/status` | Phase 2 |

Full plan: [`0040 G-implementation-plan.md`](../../lore/1-tasks/active/0040_FEATURE_prices-api-gateway-and-read-handlers/notes/G-implementation-plan.md).

## Build & test

```bash
# default build/test — no AWS/mTLS stack, router exercised via tower oneshot
cargo test -p prices-api

# Lambda artifact (ARM64) — the `lambda` feature is REQUIRED or the bin is skipped
cargo lambda build -p prices-api --release --arm64 --features lambda
```

## Env (Lambda)

| Var | Purpose |
|-----|---------|
| `CH_ENABLED` | build the mTLS CH client at cold start (default true; `0`/`false` to skip) |
| `MTLS_SECRET_NAME`, `CH_DOMAIN` | mTLS bundle + endpoint (read by `prices-clickhouse::mtls`) |
| `API_BASE_URL` | OpenAPI `servers` URL. Set by `ComputeStack` from `apiBaseUrl` in `infra/envs/production.json`; MUST include the stage path (`…/production`) |

## OpenAPI

The spec is generated from the axum routes by `utoipa`, so it cannot drift from
the implementation. It is served at `GET /api-docs-json` — **anonymous**, both
at the API Gateway (`apiKeyRequired: false`) and in the in-app key gate
(`auth::is_exempt`) — and cached for an hour.

```bash
npm run openapi:extract   # → target/openapi.json (servers stamped from config)
npm run openapi:lint      # extract + Redocly recommended ruleset; runs in CI
```

`tests/openapi.rs` asserts the document's contract: route coverage against the
gateway in both directions, the `x-api-key` scheme, the anonymous opt-outs, and
the `servers` stamp.

## Cache-Control / TTL decisions

Per-endpoint TTL tiers live in `common/cache_control.rs`; the API Gateway stage
cache (Phase 4) mirrors them. Final TTLs documented here when Phase 4 lands
(overview §2.1: `/assets` 60s, `/ohlcv` 60s, `/price` 15s, `/backfill` 30s,
batch uncached).
