---
id: "0007"
title: "Choose runtime framework and generate first app"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0006"]
tags: [phase-future, priority-medium, effort-medium, infra]
links: []
history:
  - date: 2026-05-11
    status: backlog
    who: claude
    note: "Spawned from 0006 future work."
---

# Choose runtime framework and generate first app

## Summary

Pick the runtime framework (Express / Fastify / Nest / Hono / bare Node) for the prices API
service and generate the first app under `packages/` (or `apps/` if the layout is revised).

## Context

Task 0006 set up the Nx workspace with the `ts` preset deliberately — no framework opinion.
We need an HTTP framework before any service code lands. Choice depends on the API surface
(REST? GraphQL? gRPC?), expected throughput, and team familiarity. Likely an ADR.

## Implementation

- Compare candidates against API surface and ops requirements.
- Write an ADR capturing the choice.
- Generate the first app via the relevant Nx plugin (e.g., `@nx/node`, `@nx/express`,
  `@nx/nest`, or a community plugin for Fastify/Hono).

## Acceptance Criteria

- [ ] ADR written and committed in `lore/2-adrs/`
- [ ] First app generated under `packages/<app-name>/` (or `apps/<app-name>/`)
- [ ] `npx nx serve <app>` boots a hello-world endpoint
- [ ] CI workflow (see 0008) runs the new app's lint/test/build targets
