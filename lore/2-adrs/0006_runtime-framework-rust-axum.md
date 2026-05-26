---
id: '0006'
title: 'Runtime framework: Rust + axum on Lambda (shared workspace with Soroban Block Explorer)'
status: accepted
deciders: [okarcz]
related_tasks: ['0006', '0007']
related_adrs: []
tags:
  [architecture, runtime, framework, rust, axum, lambda, block-explorer-shared]
links:
  - '../../docs/prices-api-general-overview.md'
  - '../1-tasks/archive/0006_FEATURE_setup-nx-monorepo.md'
history:
  - date: 2026-05-14
    status: proposed
    who: okarcz
    note: >
      Drafted post-factum to formalize the framework choice already
      captured in docs/prices-api-general-overview.md Section 8.
      Task 0007 (spawned from 0006 to "choose framework") framed the
      decision around JS/TS options (Express/Fastify/Nest/Hono) but the
      design doc had already settled on Rust + axum to share the
      funded Block Explorer's Rust workspace.
  - date: 2026-05-14
    status: accepted
    who: okarcz
    note: >
      Accepted same day. Runtime code (API Lambdas, Ledger Processor,
      ingestion workers, backfill tasks) is Rust + axum + sqlx on the
      lambda_runtime / Fargate. The Nx TypeScript workspace from
      task 0006 is retained for AWS CDK infrastructure code only.
      Task 0007 closed as completed (no separate scaffold work — the
      first Rust binary lands with the Tranche 1 Ledger Processor).
---

# ADR 0006: Runtime framework: Rust + axum on Lambda (shared workspace with Soroban Block Explorer)

**Related:**

- [Task 0006: Set up Nx monorepo (archived)](../1-tasks/archive/0006_FEATURE_setup-nx-monorepo.md) — provisioned the TypeScript Nx workspace; retained for CDK infra code
- [Task 0007: Choose runtime framework and generate first app (closed by this ADR)](../1-tasks/archive/0007_FEATURE_choose-runtime-framework.md) — spawned from 0006; this ADR resolves it
- [docs/prices-api-general-overview.md §8 Tech Stack Summary](../../docs/prices-api-general-overview.md) — the design doc that already encoded the decision

---

## Context

Task 0006 set up the Nx workspace with the `ts` preset deliberately leaving
the runtime framework unspecified. Task 0007 was spawned from 0006 to pick
one of Express / Fastify / Nest / Hono / bare Node and generate the first
app under the Nx workspace.

By the time 0007 came up for activation, the post-2nd-review design document
(`docs/prices-api-general-overview.md`) had already committed the entire
service to a different stack: Rust on Lambda with the `axum` HTTP router.
The driver was infrastructure sharing with the already-funded Soroban Block
Explorer (Rumble Fish, March 2026 grant):

- A shared Rust workspace crate handles `stellar-xdr` parsing for both the
  Block Explorer's Ledger Processor and the Prices API Ledger Processor.
- The Block Explorer backend uses `axum` already; reusing the router gives
  shared middleware, error handling, and OpenAPI generation patterns.
- `sqlx` migration tooling and CDK stack patterns are copied from the
  Block Explorer codebase.

The Nx TypeScript workspace is **not** discarded — AWS CDK is TypeScript
in both services (Section 8 of the design doc), so CDK stacks live in the
Nx workspace. Only the runtime code (Lambdas, ECS workers) is Rust.

So task 0007's framing was obsolete: there is no JS/TS framework to pick.

---

## Decision

The prices-api **runtime** is **Rust (edition 2021)** with **`axum`** as the
HTTP framework, run on AWS Lambda via the `lambda_runtime` crate (custom
`provided.al2` runtime) and on ECS Fargate for the Ledger Processor and
backfill tasks. Database access uses `sqlx` (compile-time verified queries,
async). XDR parsing uses the official SDF `stellar-xdr` crate via a shared
workspace crate.

The Nx TypeScript workspace from task 0006 is **retained** for AWS CDK
infrastructure code only.

Task 0007 is closed as completed by this ADR. There is no separate
"generate first app" deliverable — the first Rust binary lands with the
Tranche 1 Ledger Processor implementation.

---

## Rationale

1. **Code sharing with Block Explorer.** The `stellar-xdr` parsing logic is
   non-trivial and easy to get wrong. Compiling it once into a shared
   workspace crate (used by both BE and prices-api Ledger Processors) is the
   single biggest dev-time saving in Section 11.2 of the design doc
   (~5–7 dev days). Switching to a JS/TS runtime would force a rewrite or a
   subprocess shim.

2. **Operational cost.** Rust Lambdas have sub-millisecond cold starts, so
   no provisioned concurrency is needed at low traffic. Section 10 puts
   API handler Lambda cost at ~$20/mo.

3. **DB throughput for backfill.** SDEX backfill must sustain ~150k
   ledgers/hour for ~16 days (Section 5.6). A native Rust binary on Fargate
   with `sqlx` is the proven pattern from the Block Explorer's
   `backfill-bench` (BE ADR 0010, mirrored by prices-api ADR 0005).

4. **Team familiarity.** The same engineers maintain both codebases. One
   Rust + `axum` + `sqlx` stack across both services beats context-switching
   between Rust (BE) and Node (prices-api).

5. **`axum` over alternative Rust web frameworks.** `axum` is already the
   BE backend's router. Picking anything else (Actix-Web, Rocket, warp)
   would split the patterns.

---

## Alternatives Considered

### Alternative 1: Express / Fastify / NestJS / Hono on Node (Nx `ts` preset)

**Description:** Original framing of task 0007. Pick a Node framework and
generate the first app via the relevant Nx plugin.

**Pros:**

- Reuses the Nx TypeScript workspace from task 0006 with no second toolchain.
- Larger pool of off-the-shelf middleware and tutorials.

**Cons:**

- Forces duplication of `stellar-xdr` parsing — either rewrite in TS
  (high risk, unmaintained TS XDR types) or call a Rust subprocess.
- Higher Lambda cold-start cost; provisioned concurrency likely needed.
- No path to share code with the funded Block Explorer's Rust workspace.
- Backfill throughput on Node is plausible but unproven against the
  Block Explorer's already-validated Rust pattern.

**Decision:** REJECTED — code-sharing with Block Explorer is the load-bearing
constraint and rules out a non-Rust runtime.

### Alternative 2: Rust with Actix-Web (instead of axum)

**Description:** Same Rust runtime, different HTTP router.

**Pros:**

- Mature, fast, well-known.

**Cons:**

- Block Explorer's backend uses `axum`. Picking Actix-Web would split
  middleware, error handling, and OpenAPI generation patterns.
- No measurable win over `axum` at the traffic profile in Section 6.

**Decision:** REJECTED — `axum` is what the BE codebase already uses.

### Alternative 3: Bare Node (no framework) on Lambda

**Description:** Drop framework entirely, write raw Lambda handlers.

**Pros:**

- Smallest dependency surface.

**Cons:**

- Reinvents routing, validation, and middleware for every endpoint.
- Same Rust-sharing problem as Alternative 1.

**Decision:** REJECTED — same constraint as Alternative 1, plus loses
`axum`'s routing ergonomics.

---

## Consequences

### Positive

- `stellar-xdr` parsing crate is written once and compiled into both the
  Block Explorer Ledger Processor and the prices-api Ledger Processor.
- `sqlx` migration tooling, CloudWatch alarm patterns, and CDK constructs
  are copy-adapted from the Block Explorer codebase (Section 11.2:
  ~11–16 dev days saved across the project).
- Sub-millisecond Lambda cold starts → no provisioned concurrency line item
  in the steady-state budget.
- Backfill task can be the same Rust binary pattern that the Block
  Explorer's `backfill-bench` already validates (ADR 0005).

### Negative

- The repo now carries two toolchains: Nx (TypeScript) for CDK and Cargo
  (Rust) for runtime. Contributors need both installed.
- CI must run both `npx nx` targets and `cargo` targets. Task 0008 (CI
  workflow) needs to cover Rust lint/test/build alongside Nx.
- Task 0007's original acceptance criterion "`npx nx serve <app>` boots a
  hello-world endpoint" is no longer applicable — replaced by the Tranche 1
  Ledger Processor as the first Rust binary.

---

## References

- [docs/prices-api-general-overview.md](../../docs/prices-api-general-overview.md) — §8 Tech Stack Summary, §11.2 Development Savings
- [ADR 0005: Stream 2 SDEX historical backfill runs locally on a workstation](./0005_stream2-sdex-local-workstation-backfill.md) — mirrors the BE `backfill-bench` Rust pattern
- [axum](https://github.com/tokio-rs/axum) — HTTP framework
- [lambda_runtime](https://github.com/awslabs/aws-lambda-rust-runtime) — Rust on Lambda
- [sqlx](https://github.com/launchbadge/sqlx) — async, compile-time checked SQL
