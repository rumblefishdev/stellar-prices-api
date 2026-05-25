---
id: "0007"
title: "Choose runtime framework and generate first app"
type: FEATURE
status: completed
related_adr: ["0006"]
related_tasks: ["0006"]
tags: [layer-tooling, phase-future, priority-medium, effort-medium, infra]
links:
  - "../../2-adrs/0006_runtime-framework-rust-axum.md"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-05-11
    status: backlog
    who: claude
    note: "Spawned from 0006 future work."
  - date: 2026-05-14
    status: completed
    who: okarcz
    note: >
      Closed by ADR 0006 (Rust + axum). The framework decision was
      already made in docs/prices-api-general-overview.md §8 Tech Stack
      Summary before this task came up for activation. Task 0007's
      original framing (Express / Fastify / Nest / Hono / bare Node)
      was obsolete — the runtime is Rust + axum + sqlx on Lambda,
      with the Nx TypeScript workspace retained only for CDK infra.
      No separate "generate first app" deliverable: the first Rust
      binary lands with the Tranche 1 Ledger Processor.
---

# Choose runtime framework and generate first app

## Summary

Pick the runtime framework (Express / Fastify / Nest / Hono / bare Node) for the prices API
service and generate the first app under `packages/` (or `apps/` if the layout is revised).

**Resolved by [ADR 0006](../../2-adrs/0006_runtime-framework-rust-axum.md):** Rust + `axum`
on Lambda (shared workspace with the Soroban Block Explorer). The original framing of this
task — picking among JS/TS frameworks under the Nx workspace — was obsolete: by the time
this task came up for activation, `docs/prices-api-general-overview.md` §8 had already
committed the runtime to Rust + axum for code-sharing with the funded Block Explorer
codebase.

## Context

Task 0006 set up the Nx workspace with the `ts` preset deliberately — no framework opinion.
We need an HTTP framework before any service code lands. Choice depends on the API surface
(REST? GraphQL? gRPC?), expected throughput, and team familiarity. Likely an ADR.

**Update (2026-05-14):** The choice was already encoded in the post-2nd-review design doc
(`docs/prices-api-general-overview.md` §8). Rather than activate this task and re-litigate
the decision, it is closed by ADR 0006, which formalizes the existing choice (Rust + axum)
with rationale and alternatives.

## Implementation

- Compare candidates against API surface and ops requirements.
- Write an ADR capturing the choice.
- Generate the first app via the relevant Nx plugin (e.g., `@nx/node`, `@nx/express`,
  `@nx/nest`, or a community plugin for Fastify/Hono).

## Acceptance Criteria

- [x] ADR written and committed in `lore/2-adrs/` — [ADR 0006](../../2-adrs/0006_runtime-framework-rust-axum.md)
- [ ] First app generated under `packages/<app-name>/` (or `apps/<app-name>/`) — superseded; the first Rust binary lands with the Tranche 1 Ledger Processor (not an Nx-generated app)
- [ ] `npx nx serve <app>` boots a hello-world endpoint — superseded; not applicable to the Rust runtime
- [ ] CI workflow (see 0008) runs the new app's lint/test/build targets — deferred to task 0008, which must now cover both Nx (TS) and Cargo (Rust) targets

## Design Decisions

### From Plan

1. **Write an ADR for the framework choice.** Done — ADR 0006.

### Emerged

2. **Decision pre-existed in the design doc, not made by this task.** The runtime was
   already committed to Rust + axum in `docs/prices-api-general-overview.md` §8 (driven
   by code-sharing with the funded Soroban Block Explorer). This task did not re-litigate
   the choice — it formalized it post-factum in ADR 0006.

3. **Nx workspace retained for CDK only, not discarded.** Task 0006 set up the Nx
   TypeScript workspace; the design doc keeps it for AWS CDK infrastructure code
   (Section 8: "Infrastructure is managed via AWS CDK (TypeScript)"). The repo carries
   two toolchains (Nx for CDK, Cargo for runtime) — documented as a negative consequence
   in ADR 0006.

4. **No "first app generated" deliverable.** The original criteria assumed an Nx-generated
   JS/TS app booting via `npx nx serve`. Under Rust + axum on Lambda, there is no
   equivalent step — the first Rust binary is the Tranche 1 Ledger Processor, scoped
   under its own implementation task (not this one).

## Issues Encountered

- **Task framing was stale by the time it came up.** Task 0007 was spawned (2026-05-11)
  from 0006's future work in JS/TS framing. The post-2nd-review design doc was written
  later and changed the stack. Lesson: when a backlog task is spawned ahead of a design
  doc that may move the boundary, re-validate the task's premise at activation time
  before promoting. Caught here only because the `/promote-task` invocation explicitly
  asked to cross-check the overview doc.

## Future Work

- **Task 0008 (CI workflow):** must cover both Nx (TS) `lint/test/build` targets and Cargo
  (Rust) `cargo fmt/clippy/test/build` targets. No new backlog task needed — already
  captured in 0008's scope; ADR 0006 calls this out as a negative consequence.
