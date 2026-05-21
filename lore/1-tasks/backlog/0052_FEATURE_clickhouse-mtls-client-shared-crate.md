---
id: "0052"
title: "ClickHouse mTLS client shared crate — cert loading from Secrets Manager + warm connection pool"
type: FEATURE
status: backlog
related_adr: ["0006", "0007"]
related_tasks: ["0050", "0038", "0039", "0040", "0028", "0051"]
tags: [layer-backend, priority-high, effort-medium, milestone-M1, rust, clickhouse, mtls, shared-crate, lambda, secrets-manager]
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../2-adrs/0006_runtime-framework-rust-axum.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "./0050_FEATURE_be-side-prep-sns-mtls-prices-db-provisioning.md"
  - "../blocked/0038_FEATURE_prices-ledger-processor-lambda.md"
  - "../blocked/0039_FEATURE_prices-periodic-workers-lambda-set.md"
  - "../blocked/0040_FEATURE_prices-api-gateway-and-read-handlers.md"
history:
  - date: 2026-05-21
    status: backlog
    who: okarcz
    note: >
      Spawned during Tranche 1 task-set creation. The §5.2 "mTLS
      write path" subsection calls for a warm-connection client
      with Secrets-Manager-loaded cert+key, reused across Lambda
      invocations. Every downstream Lambda (0038 live processor,
      0039 periodic workers, 0040 API handlers, 0055 backfill
      status endpoint) and the 0028 SDEX cloud-push tool need
      exactly the same setup. Captured here so it gets built
      once, not five times.
---

# ClickHouse mTLS client shared crate

## Summary

Build `packages/clickhouse-client` — a thin Rust crate wrapping
the [`clickhouse`](https://crates.io/crates/clickhouse) Rust crate
with prices-api conventions: load mTLS cert + key from AWS Secrets
Manager on cold start, warm the TLS connection in Lambda global
init, expose a `Client` handle reused across invocations, and
provide ergonomic helpers for the common `INSERT` and `SELECT
FINAL` / `argMax-GROUP-BY` patterns prices-api uses against
`prices.*`.

## Context

Per §5.2 ("mTLS write path") and §6 ("Database client" row of
the performance table), every prices-api process that talks to
Caddy:443 needs to:

1. Load the per-env client cert + key from AWS Secrets Manager
   (two secrets per env per ADR 0007 §3.5).
2. Establish a warm TLS connection during Lambda global init so
   the ~80–130 ms cross-cloud RTT for TLS handshake is amortised
   across invocations.
3. Reuse the connection across invocations, with health checks
   and a reconnect path on failure.
4. Batch writes per-ledger so a typical invocation issues 1–2
   INSERTs, not one per trade.

Doing this once in a shared crate avoids each Lambda reinventing
the same boilerplate (and getting the cold-start path subtly
wrong). The crate is consumed by:

- 0038 — Prices Ledger Processor Lambda (live INSERT path).
- 0039 — Periodic workers (price-updater, oracle, discovery,
  cleanup).
- 0040 — Public API handlers (axum Lambda binaries).
- 0055 — `GET /backfill/status` endpoint Lambda.
- 0028 — `sdex-cloud-push` workstation CLI.
- 0053 — `soroban-amm-backfill` workstation CLI completion-push
  step.
- 0051 — `schema-apply` migration runner.

## Implementation Plan

### Step 1: Crate scaffolding

Add `packages/clickhouse-client/` as a library crate. Depend on:

- `clickhouse` (Rust crate, native protocol over HTTPS-mTLS).
- `aws-sdk-secretsmanager` for cert + key retrieval.
- `rustls` / `tokio-rustls` for TLS material handling (whichever
  the `clickhouse` crate's TLS feature pulls in).
- `tracing` for structured logging on connect / reconnect events.

### Step 2: Secrets Manager loading

Public API:

```rust
pub struct ClientConfig {
    pub endpoint: String,       // https://caddy.example.com:443
    pub database: String,       // "prices"
    pub user: String,           // prices-api user name
    pub cert_secret_id: String, // AWS Secrets Manager ARN
    pub key_secret_id: String,  // AWS Secrets Manager ARN
    pub ca_pem: &'static [u8],  // BE's CA, embedded at compile time
}

pub async fn from_env() -> Result<ClientConfig, ConfigError>;
pub async fn build_client(cfg: ClientConfig) -> Result<Client, BuildError>;
```

`from_env` reads conventional env var names (`CH_ENDPOINT`,
`CH_DATABASE`, `CH_USER`, `CH_CERT_SECRET_ID`, `CH_KEY_SECRET_ID`)
populated by the 0011 CDK stack from the SSM keys 0050 publishes.

`build_client` fetches the two secrets in parallel, parses the
PEMs, configures `rustls` with BE's CA, and returns a wrapped
`clickhouse::Client` ready to use.

### Step 3: Warm-connection pattern

Provide a `lambda_init` helper that lets a Lambda's `main` write:

```rust
static CLIENT: OnceCell<Client> = OnceCell::const_new();

#[tokio::main]
async fn main() -> Result<(), Error> {
    let client = CLIENT
        .get_or_init(|| async { build_client(from_env().await?).await.unwrap() })
        .await;
    lambda_runtime::run(service_fn(|event| handler(event, client))).await
}
```

The TLS connection is established once in global init; subsequent
invocations reuse the warm `Client`. Document the pattern in the
crate README.

### Step 4: Insert + read helpers

Thin wrappers for the patterns the design doc names:

- `insert_ohlcv_rows(client, granularity, rows)` — batches into
  one native-protocol INSERT against `prices.price_ohlcv_<gran>`.
- `select_final<T>(client, query)` — wraps `SELECT … FINAL` for
  read handlers that want eventual-consistency-safe reads.
- `select_argmax_groupby<T>(client, query)` — alternative for
  the read handler patterns that prefer `argMax + GROUP BY`
  over `FINAL` (ADR 0007 §3.3).

Avoid over-engineering: these are convenience wrappers, not a
query builder. Anything custom goes through the raw
`clickhouse::Client` handle exposed alongside.

### Step 5: Failure and reconnect

- On TLS handshake failure during cold start, the Lambda fails
  fast (no retry — let Lambda runtime restart the container).
- On a connection drop mid-request, the next call rebuilds the
  TLS connection transparently inside `clickhouse::Client`
  (verify this is the upstream crate's behaviour; otherwise add
  a thin retry layer with one re-handshake attempt).
- Emit a CloudWatch metric `clickhouse_client.reconnect_count`
  for ops visibility.

### Step 6: Tests

- Unit: `from_env` parsing, ClientConfig validation.
- Integration: against a Docker CH with a self-signed CA +
  client cert pair generated in the test setup, run an INSERT
  and a SELECT round-trip. Re-use the warm Client across two
  simulated invocations and assert no second TLS handshake
  occurs (instrument via tracing).

## Acceptance Criteria

- [ ] `packages/clickhouse-client` crate builds, with unit +
      integration tests passing locally
- [ ] Integration test confirms warm-connection reuse across
      simulated Lambda invocations (single TLS handshake per
      container lifetime)
- [ ] Crate consumed by at least one downstream (0051's
      `schema-apply` runner is the obvious first consumer); 0038
      and 0040 wire it in once they unblock
- [ ] README documents the `lambda_init` pattern and the env
      var contract from 0011 / 0050
- [ ] `clickhouse_client.reconnect_count` CloudWatch metric
      emitted; reconnect path verified against a manually-killed
      TLS connection in integration test

## Blocked on

- **None for authoring + Docker testing** — can start in Week 1.
- **0050** — only the live-cluster validation step needs a real
  cert + key + endpoint. Docker testing covers everything else.

## Out of scope

- ClickHouse query builder / ORM — convenience helpers only.
- Connection pooling beyond Lambda's one-container-one-client
  model. Multi-connection pools are an axum-process concern
  for the API handlers (0040); revisit there if measured
  throughput warrants it.
- Migration tooling — see 0051.

## Notes

- The `clickhouse` Rust crate's TLS feature pulls a specific
  `rustls` major version; pin the workspace-level `rustls` to
  match to avoid the multi-version-rustls compile hazard.
- BE's CA cert is checked into the crate (or workspace) as a
  static asset — that's safe to commit (public key material).
  The per-env client cert + key are runtime-loaded from
  Secrets Manager, never committed.
- Once 0038/0039/0040 land, audit that every CH access goes
  through this crate; flag any direct `clickhouse::Client::new`
  in code review.
