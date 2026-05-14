---
id: "0029"
title: "Update prices-api-general-overview.md Stream 1 sections per ADR 0001 (local ClickHouse-sourced AMM backfill)"
type: DOCS
status: backlog
related_adr: ["0001"]
related_tasks: ["0013", "0015", "0017", "0018"]
tags: [priority-medium, effort-small, docs, stream-1, clickhouse, backfill, block-explorer]
links:
  - "../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "../../../docs/prices-api-general-overview.md"
  - "../active/0013_DOCS_update-design-doc-to-match-be-reality.md"
history:
  - date: 2026-05-14
    status: backlog
    who: claude
    note: >
      Spawned from task 0013 future work. Task 0013's PR (#TBD)
      reconciled Stream 2 with ADR 0005 across §2.3, §3.5, §4.5, §5.3,
      §5.6, §6, §8, §9, §10, §11.1, §11.4. The corresponding Stream 1
      reconciliation with ADR 0001 was deliberately deferred — none of
      the user's per-turn scopes ("§5.6 Stream 2", "§3.5/§4.5",
      "§9/§10/§11", "§2.3/§5.3/§6", "§8") asked for Stream 1 changes.
      Each affected section either still describes Stream 1 as a
      Fargate task reading BE Postgres `soroban_events` (the pre-ADR
      0001 framing) or holds a placeholder row referencing this task.
---

# Update prices-api-general-overview.md Stream 1 sections per ADR 0001

## Summary

Reconcile every Stream 1 reference in `docs/prices-api-general-overview.md`
with [ADR 0001](../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md):
Stream 1 (Soroban AMM historical backfill) sources from a **local ClickHouse
instance** populated by BE's `backfill-runner --target=clickhouse`, not from
BE's RDS Postgres `soroban_events` table, and runs as a **local Rust CLI** on
the operator's workstation, not as an ECS Fargate task.

## Context

Companion task to [0013](../active/0013_DOCS_update-design-doc-to-match-be-reality.md).
Task 0013's PR landed Stream 2 / ADR 0005 reconciliation; this task picks up
the Stream 1 / ADR 0001 reconciliation that was deliberately out of scope on
every turn of 0013's implementation.

Why ADR 0001 changed the Stream 1 source (briefly): BE folded its Postgres
`soroban_events` table into appearances-only per
[BE ADR 0033](../../../soroban-block-explorer/lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md),
removing the full decoded-JSONB row that prices-api's design originally
assumed. BE then declared a parallel ClickHouse store per
[BE ADR 0044](../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md)
that does keep per-event rows with full inlined `topics_xdr` + `data_xdr` and
a hoisted `signature` column. Prices-api's Stream 1 backfill consumes that
ClickHouse copy via a local instance populated by BE's `backfill-runner
--target=clickhouse`.

## Implementation

Each section below needs to drop the "Fargate task reading BE RDS Postgres
`soroban_events`" framing in favor of "local Rust CLI consuming a local
ClickHouse instance populated by BE's `backfill-runner --target=clickhouse`":

- **§2.3** — `soroban_events` row currently claims read-only RDS access to
  BE's Postgres. Rewrite to describe the local ClickHouse pattern, and
  decide whether the row stays in §2.3 at all (BE PG is no longer the source;
  the actual coupling is via a transient backfill-runner invocation, not
  shared infrastructure).
- **§5.3 Ingestion Workers** — there is currently a placeholder "Soroban AMM
  Backfill" row with a `†` footnote pointing at this task. Replace with the
  real description (local CLI + ClickHouse source + extraction step).
- **§5.6 Stream 1** — the architecture diagram still shows "Block Explorer RDS
  (read-only) / soroban_events". Redraw for the local ClickHouse model. The
  processing-rate sub-table still says "ECS task configuration | 1 vCPU /
  2 GB RAM | Same ECS cluster" — replace with local-workstation metrics.
- **§9 Tranche 1 / 2 / 3** — Tranche 1's Soroban AMM milestone and validation
  still reference `soroban_amm.status: "completed"` only; check whether any
  reviewer-confirm or acceptance-criteria text references the Fargate-shape
  for Stream 1 (most of it should already work post-Stream-2 cleanup, but
  audit explicitly).
- **§10 Cost** — the "ECS Fargate — Soroban AMM backfill task | ~$2" line is
  already flagged as pending in 0013's PR. Drop it (Stream 1 is also local
  per ADR 0001 — no Fargate compute).
- **§11.1** — the `soroban_events` BE-shared row claims "no extra RDS cost"
  for read-only Postgres access. Rewrite for the local ClickHouse pattern;
  the BE-funded artefact is the `backfill-runner --target=clickhouse` writer,
  not the BE database.
- **§11.4** — the opening paragraph and the risk table describe a runtime
  read-only connection to BE's Postgres. Replace with the local-ClickHouse
  framing; the only coupling is a one-time transient backfill-runner
  invocation against BE's CH writer.

## Acceptance Criteria

- [ ] §2.3 `soroban_events` row reflects the local ClickHouse pattern per
      ADR 0001 (or is dropped if the row no longer represents shared infra)
- [ ] §5.3 placeholder Soroban AMM Backfill row replaced with accurate
      Stream 1 description (local CLI + local ClickHouse + extraction); the
      `†` footnote about the pending row removed
- [ ] §5.6 Stream 1 architecture diagram redrawn for local ClickHouse model;
      processing-rate sub-table rewritten with local-workstation metrics
- [ ] §9 Tranche acceptance criteria audited for residual Fargate-shape
      Stream 1 references; fixed if any remain
- [ ] §10 Soroban AMM Fargate cost line removed; new total recomputed
- [ ] §11.1 `soroban_events` row rewritten for the local ClickHouse pattern;
      monthly saving total updated accordingly
- [ ] §11.4 opening paragraph and risk table rewritten; BE coupling correctly
      framed as transient backfill-runner invocation, not runtime PG read
- [ ] Cross-link to ADR 0001 added wherever §5.6 Stream 1 is touched
- [ ] Task 0013 can be marked completed once this PR merges (its three
      Stream 1 acceptance criteria were marked "deferred to 0029")
- [ ] Reviewer (project lead) approves the revision

## Future Work

None spawned from this task — it closes out the design-doc reconciliation
sweep that started in 0013. If more ADRs land that change either stream's
shape, a separate task will be needed.
