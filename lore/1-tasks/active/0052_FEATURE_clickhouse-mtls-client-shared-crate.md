---
id: "0052"
title: "ClickHouse mTLS client shared crate — cert loading from Secrets Manager + warm connection pool"
type: FEATURE
status: active
related_adr: ["0006", "0007"]
related_tasks: ["0060", "0063", "0038", "0039", "0040", "0028", "0051", "0050"]
tags: [layer-backend, priority-high, effort-medium, milestone-M1, rust, clickhouse, mtls, shared-crate, lambda, secrets-manager]
milestone: 1
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
  - date: 2026-06-17
    status: active
    who: oski
    note: >
      Activated as the recommended starting point for the Hetzner DB
      track: it has zero external dependencies (author + Docker test
      need no admin access), and it is the foundation 0051's
      schema-apply runner and the 0038/0039/0040 Lambdas all import.
      Live-cluster validation step still waits on 0063 (was 0050) for
      a real cert + endpoint.
---

# ClickHouse mTLS client shared crate

> **Rescoped 2026-06-17 (Emerged):** original plan specced a greenfield
> `packages/clickhouse-client`. The codebase has since grown
> `packages/prices-clickhouse` (task 0060), which already owns
> `Config::from_env()` + `client(cfg)` (the plaintext client surface)
> and the schema. To avoid two crates both "handing out a configured
> client" (the fragmentation this task's Notes warn against), 0052 now
> **extends `prices-clickhouse`** with the mTLS transport. No new crate.

## Summary

Add the **mTLS + warm-connection transport** to the existing
`packages/prices-clickhouse` crate: load the per-env client cert +
key from AWS Secrets Manager on cold start, build a rustls-configured
`clickhouse::Client` that talks to Caddy:443 over HTTPS-mTLS, warm
the connection in Lambda global init, and reuse the handle across
invocations. The plaintext local-dev path (`http://localhost:8123`,
password auth) stays intact for Docker / tests.

## Context

`packages/prices-clickhouse` today exposes `Config { url, user,
password, database }`, `Config::from_env()`, and `client(cfg)` that
builds a **plaintext** `clickhouse::Client`. The workspace `clickhouse`
dep is pinned at `0.13` with only the `inserter` feature — **no TLS**.
What is missing for talking to the production Hetzner box (ADR 0007
§3.5 / design §5.2 "mTLS write path"):

1. Load the per-env client cert + key from AWS Secrets Manager
   (two secrets per env per ADR 0007 §3.5).
2. Establish a warm TLS connection during Lambda global init so
   the ~80–130 ms cross-cloud RTT for TLS handshake is amortised
   across invocations.
3. Reuse the connection across invocations, with health checks
   and a reconnect path on failure.
4. Batch writes per-ledger so a typical invocation issues 1–2
   INSERTs, not one per trade.

Doing this once in the shared crate avoids each Lambda reinventing
the same boilerplate (and getting the cold-start path subtly
wrong). The mTLS client surface is consumed by:

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

### Step 1: TLS-enable the `clickhouse` dependency — DONE (2026-06-17)

- Enabled `rustls-tls-ring` + `rustls-tls-webpki-roots` on the
  workspace `clickhouse` dep alongside `inserter`. `ring` (not the
  default `aws-lc` bundle) avoids a cmake/C toolchain — friendlier for
  Lambda cross-builds; `webpki-roots` bundles Mozilla roots to verify
  Caddy's Let's Encrypt server cert.
- Verified: workspace builds, and the tree resolves to a single
  `rustls` (0.23.40) — no multi-rustls hazard, so no explicit pin
  needed (left for Step 2 if a direct rustls dep is added).
- **Dropped the "commit BE's CA cert as a static asset" item** — see
  Emerged decision 1. Caddy's *server* cert is Let's Encrypt (public),
  so the client verifies it with `webpki-roots`; BE's CA lives on Caddy
  for *client* verification, not in our code.
- `aws-sdk-secretsmanager` / `aws-config` deferred to Step 2, where
  they're actually used (avoids a dead dependency in a Step-1-only PR).

### Step 2: mTLS config + Secrets-Manager loader (extend, don't replace)

Keep the existing `Config` / `from_env()` / `client(cfg)` plaintext
path untouched (Docker + local dev + the `prices-clickhouse-init`
binary depend on it). Add an mTLS variant beside it:

```rust
pub struct MtlsConfig {
    pub endpoint: String,        // https://caddy.example.com:443
    pub database: String,        // defaults to PROD_DATABASE ("prices")
    pub user: String,            // e.g. prices_writer / prices_reader
    pub cert_secret_id: String,  // AWS Secrets Manager ARN
    pub key_secret_id: String,   // AWS Secrets Manager ARN
}

pub async fn mtls_from_env() -> Result<MtlsConfig, ConfigError>; // CH_* env
pub async fn client_mtls(cfg: &MtlsConfig) -> Result<Client, BuildError>;
```

`mtls_from_env` reads `CH_ENDPOINT`, `CH_DATABASE`, `CH_USER`,
`CH_CERT_SECRET_ID`, `CH_KEY_SECRET_ID` (populated by the 0011 CDK
stack from the SSM keys 0063 publishes). `client_mtls` fetches the
two secrets in parallel, parses the PEMs, configures rustls with the
embedded BE CA + the client cert/key, and returns a `clickhouse::Client`.

### Step 3: Warm-connection helper for Lambdas

Provide a `lambda_init`-style helper using `OnceCell` so a Lambda's
`main` warms the TLS connection once in global init and reuses the
clone-cheap `Client` across invocations. Document the pattern in the
crate README (the existing `Client`-is-cheap-to-clone note already
hints at this).

### Step 4: Resilience + metric

- Cold-start TLS handshake failure → fail fast (let the Lambda
  runtime restart the container; no retry).
- Mid-request connection drop → verify the upstream `clickhouse`
  crate re-handshakes transparently; if not, add a thin one-attempt
  retry layer.
- Emit `clickhouse_client.reconnect_count` for ops visibility.

> Insert/read helpers (`insert_ohlcv_rows`, `select_final`,
> `select_argmax_groupby`) are **deferred** unless the existing crate
> lacks them — the extractor/backfill crates already own their row
> structs + writers (per the crate's own lib.rs doc comment). Add only
> if a concrete consumer needs a shared wrapper; otherwise out of scope.

### Step 5: Tests

Mirror the existing `tests/views_it.rs` Docker-CH harness:

- Unit: `mtls_from_env` parsing + `MtlsConfig` validation.
- Integration: Docker CH with a test-generated self-signed CA +
  client-cert pair; run an INSERT + SELECT round-trip over mTLS.
  Reuse the warm `Client` across two simulated invocations and assert
  no second TLS handshake (instrument via tracing).

## Acceptance Criteria

- [ ] `packages/prices-clickhouse` gains `MtlsConfig` +
      `mtls_from_env` + `client_mtls`; the existing plaintext
      `Config`/`client` path is unchanged and still compiles
- [x] Workspace `clickhouse` TLS feature enabled
      (`rustls-tls-ring` + `rustls-tls-webpki-roots`); tree resolves to
      a single `rustls` 0.23.40 (no multi-rustls build) — Step 1
- [ ] Integration test confirms warm-connection reuse across
      simulated Lambda invocations (single TLS handshake per
      container lifetime)
- [ ] mTLS surface consumed by at least one downstream (0038 live
      processor / 0040 handlers once they unblock; 0063's smoke test
      can be the first live exercise)
- [ ] README documents the `lambda_init` pattern and the env
      var contract from 0011 / 0063
- [ ] `clickhouse_client.reconnect_count` metric emitted; reconnect
      path verified against a manually-killed TLS connection in test

## Implementation Notes

**Step 1 done (2026-06-17):** workspace `Cargo.toml` `clickhouse` dep now
carries `rustls-tls-ring` + `rustls-tls-webpki-roots`. `cargo build
--workspace` is clean; `cargo tree -i rustls` shows a single `rustls 0.23.40`
with `ring 0.17.14`, `hyper-rustls 0.27.9`, `webpki-roots 1.0.8`. No code
changes — TLS is a compile-time capability switch; the mTLS wiring is Step 2.

## Design Decisions

### Emerged

1. **No embedded BE CA in the client.** The original plan said to commit
   `assets/be-ca.pem`. Confirmed from BE's `Caddyfile` that Caddy's *server*
   cert is Let's Encrypt (public) — the client verifies it with `webpki-roots`.
   BE's CA is used by Caddy to verify *our* client cert (`clients-ca.pem` on the
   box), so it never belongs in our code. Dropped.
2. **`ring` crypto provider, not aws-lc.** `rustls-tls-ring` over the default
   `rustls-tls` (aws-lc) to avoid a cmake/C-toolchain build dep — better for
   Lambda cross-compiles. Single-rustls resolution means no explicit pin needed
   yet.
3. **aws-sdk deferred to Step 2.** Adding it in Step 1 would be a dead dep; it's
   added where the Secrets-Manager loader uses it.
4. **Admin cert = ops only.** BE handed over an mTLS admin cert+key (the
   DDL-capable identity for schema-apply / 0063 provisioning, run from the
   workstation). The runtime Lambdas must still use scoped
   `prices-ingestion`/`prices-api` certs from Secrets Manager — least privilege.

## Blocked on

- **None for authoring + Docker testing** — can start now.
- **0063** (was 0050) — only the live-cluster validation step needs a
  real cert + key + endpoint. Docker testing covers everything else.

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
