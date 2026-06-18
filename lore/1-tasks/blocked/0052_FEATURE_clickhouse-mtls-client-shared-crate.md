---
id: "0052"
title: "ClickHouse mTLS client shared crate — cert loading from Secrets Manager + warm connection pool"
type: FEATURE
status: blocked
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
  - date: 2026-06-17
    status: active
    who: oski
    note: >
      Studied BE's crates/db-clickhouse/src/mtls.rs (their production mTLS CH
      client) and chose to PORT it into prices-clickhouse behind an `aws-mtls`
      feature rather than design our own or git-depend on their crate. First
      Step-1 attempt (enabling the clickhouse crate's rustls-tls features) was
      the wrong mechanism — closed PR #44 — because that can't present a client
      cert; mTLS needs a custom connector via with_http_client (confirmed
      present + connector-generic in our 0.13, so no version bump). Port + unit
      tests done; live round-trip deferred to 0063/0051.
  - date: 2026-06-18
    status: blocked
    who: oski
    note: >
      In-scope work merged to develop via PR #45 (squash d4a9657): the
      `aws-mtls`-gated `mtls` module (client_with_mtls / client_from_lambda_env
      / fetch_bundle_from_extension), the feature + optional deps, README
      env-var contract + build-once-reuse note, and the two code-review polish
      fixes (amortisation-claim wording, actionable key-parse error). 7 unit
      tests green; default build stays plaintext-lean. Moving to blocked on
      0063: the two remaining acceptance criteria — live mTLS round-trip and
      ≥1 downstream consumer — both need a real cert bundle in Secrets Manager
      + the reachable Caddy endpoint, which 0063 provisions (first exercised by
      0051's live schema-apply). No code work remains on 0052 itself.
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

> **Approach (2026-06-17): port BE's proven module, don't design our own.**
> BE's `crates/db-clickhouse/src/mtls.rs` already solves this exact problem in
> production. After studying it we chose to **port it near-verbatim** into
> `prices-clickhouse` behind an `aws-mtls` feature (decision recorded in the
> task history + Design Decisions). The custom `MtlsConfig`/two-secret API
> sketched earlier is superseded by BE's single-bundle + Lambda-Extension shape.

## Implementation Plan (as built)

### Step 1: Port the mTLS module — DONE (2026-06-17)

- `packages/prices-clickhouse/src/mtls.rs` — ported from BE. Public surface:
  - `MtlsBundle { cert_pem, key_pem, ca_pem }` (manual `Debug` redacts all PEM).
  - `fetch_bundle_from_extension(secret_name)` — fetches the bundle from the
    **AWS Parameters & Secrets Lambda Extension** (`localhost:2773`, `reqwest`),
    warm-cached; reads `AWS_SESSION_TOKEN`.
  - `client_with_mtls(domain, &bundle, database)` — builds a `hyper-rustls`
    connector with `with_client_auth_cert`, root store = `webpki-roots` + bundle
    CA, injected into `clickhouse::Client::with_http_client`.
  - `client_from_lambda_env(database)` — one-shot for cold start; reads
    `MTLS_SECRET_NAME` + `CH_DOMAIN`.
- `Cargo.toml` — new `aws-mtls` feature gating optional deps (hyper-util,
  hyper-rustls, rustls[aws-lc-rs], rustls-pemfile, rustls-pki-types,
  webpki-roots, reqwest, serde, serde_json). Default build stays plaintext-only.
- `lib.rs` — `#[cfg(feature = "aws-mtls")] pub mod mtls;`.

### Out of scope vs the original sketch

- **No `MtlsConfig`/two-secret API, no `CH_*` env quartet** — BE uses a single
  JSON bundle secret + `MTLS_SECRET_NAME`/`CH_DOMAIN`; we match it for cross-team
  consistency.
- **No embedded BE CA asset** — Caddy's server cert is public LE (verified via
  `webpki-roots`); the bundle CA arrives at runtime in the secret.
- **No `OnceCell` helper / `reconnect_count` metric** — BE builds the
  clone-cheap client once into Lambda state and relies on the hyper pool; we
  follow that. Add a metric later only if ops needs it.
- **No insert/read helpers** — extractor/backfill crates own their writers.

## Acceptance Criteria

- [x] `prices-clickhouse` gains an `aws-mtls`-gated `mtls` module
      (`client_with_mtls` / `client_from_lambda_env` / `fetch_bundle_from_extension`);
      the plaintext `Config`/`client` path is unchanged and still builds lean
- [x] mTLS stack compiles on our `clickhouse` 0.13 via `with_http_client`
      (no version bump); single `rustls` 0.23.40 in the tree
- [x] Unit tests pass: PEM-parse shape, `MtlsBundle` Debug redaction,
      missing-env error (`cargo test -p prices-clickhouse --features aws-mtls`)
- [ ] Live mTLS round-trip against the Hetzner `prices` DB (dev) — deferred to
      0063 (needs a real cert bundle + endpoint); first exercised by 0051's
      live schema-apply
- [ ] Consumed by ≥1 downstream (0051 schema-apply / 0038 / 0040 as they land)
- [x] README documents the env-var contract (`MTLS_SECRET_NAME`, `CH_DOMAIN`)
      + the build-once-reuse pattern

## Blocked on

- **None for the port + unit tests** — DONE.
- **0063** (was 0050) — the live round-trip needs a real cert bundle in Secrets
  Manager + the Caddy endpoint. The bundle is the single JSON `{cert,key,ca}`
  secret named by `MTLS_SECRET_NAME`.

## Out of scope

- ClickHouse query builder / ORM — convenience helpers only.
- Connection pooling beyond Lambda's one-container-one-client
  model. Multi-connection pools are an axum-process concern
  for the API handlers (0040); revisit there if measured
  throughput warrants it.
- Migration tooling — see 0051.

## Design Decisions

### Emerged

1. **Port BE's `mtls.rs`, don't design our own** (chosen over depending on BE's
   `db-clickhouse` crate via git or extracting a shared crate). The git-dep
   route would force a `clickhouse` version bump to BE's `=0.15.0` and drag
   their schema/persist/domain code into our build; the shared-crate route needs
   BE lead time. Porting keeps a clean boundary, stays on our 0.13, and inherits
   their war-story fixes by copying. Cost: manually track future BE changes to
   this ~250-line file.
2. **First Step-1 attempt (enable the `clickhouse` crate's `rustls-tls-*`
   features) was wrong** and was closed (PR #44). Those features make the
   crate's *default* client TLS-capable but cannot present a client cert; mTLS
   needs a custom connector via `with_http_client`. Confirmed `with_http_client`
   exists in 0.13 and its `impl<C> HttpClient for Client<C, RequestBody>` is
   generic over the connector — so BE's pattern compiles on 0.13.
3. **`aws-lc-rs` provider** (matches BE) — builds fine locally (cmake present)
   and in the Lambda toolchain. If a future build target lacks a C toolchain,
   switch the `aws-mtls` feature + `install_default_crypto_provider()` to `ring`.
4. **Single bundle + Lambda Extension** over `aws-sdk-secretsmanager` + two
   secrets — warm-cached, no SM API on the hot path, and identical to BE so the
   shared infra (CDK secret naming, `MTLS_SECRET_NAME`/`CH_DOMAIN`) is uniform.

## Implementation Notes

- Ported `mtls.rs` (gated `aws-mtls`), added the feature + optional deps to
  `Cargo.toml`, wired `pub mod mtls` in `lib.rs`.
- Verified: default build lean (no rustls); `--features aws-mtls` builds; clippy
  clean; 7 unit tests pass; fmt clean. `cargo tree` → single `rustls 0.23.40`,
  `hyper-rustls 0.27.9`, `reqwest 0.12`, `aws-lc-rs`.
- Live round-trip not runnable here (needs real cert bundle + Caddy endpoint) —
  deferred to 0063/0051 live apply.

## Notes

- Keep this module in lockstep with BE's `crates/db-clickhouse/src/mtls.rs`;
  port their fixes when they change it.
- The per-env client cert + key arrive at runtime in the Secrets Manager bundle
  — never committed. Only BE's public CA cert would ever be safe to commit, and
  we don't even need that (it rides in the bundle). See secrecy rule in memory.
- Once 0038/0039/0040 land, audit that every remote CH access goes through this
  module; flag any direct plaintext `client()` against the prod endpoint.
