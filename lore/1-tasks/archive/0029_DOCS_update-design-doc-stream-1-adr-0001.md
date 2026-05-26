---
id: '0029'
title: 'Update prices-api-general-overview.md Stream 1 sections per ADR 0001 (local ClickHouse-sourced AMM backfill)'
type: DOCS
status: completed
related_adr: ['0001']
related_tasks: ['0013', '0015', '0017', '0018']
tags:
  [
    layer-indexing,
    priority-medium,
    effort-small,
    docs,
    stream-1,
    clickhouse,
    backfill,
    block-explorer,
  ]
links:
  - '../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md'
  - '../../../docs/prices-api-general-overview.md'
  - '../archive/0013_DOCS_update-design-doc-to-match-be-reality.md'
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
  - date: 2026-05-15
    status: active
    who: oskar
    note: >
      Promoted to active. Starting Stream 1 / ADR 0001 reconciliation
      sweep across §2.3, §5.3, §5.6, §9, §10, §11.1, §11.4 of
      docs/prices-api-general-overview.md.
  - date: 2026-05-15
    status: completed
    who: claude
    note: >
      Reconciliation sweep merged via PR #15 (squash commit 5ce3a7e on
      develop). Single-file change to docs/prices-api-general-overview.md
      (+144/-77 lines) touching §2.3, §5.3, §5.6 Stream 1 (two-stream
      design table, ASCII architecture diagram, schema-coupling note,
      processing-rate sub-table), §9 Tranche 1 work bullets, §10, §11.1,
      §11.2, §11.4, plus a new Revision History entry. Eight design-doc
      acceptance criteria all closed; the "Task 0013 closure" AC was
      moot (0013 was already in archive pre-emptively, with its three
      Stream 1 ACs marked "deferred to 0029" — those are now satisfied
      by this PR). Backfill cost total ~$32 → ~$30. Three "Emerged"
      autonomous decisions documented under Design Decisions: row removal
      vs rewrite for §2.3/§11.1, `soroban-amm-backfill` binary naming,
      and wholesale §11.4 risk-table replacement.
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

Companion task to [0013](../archive/0013_DOCS_update-design-doc-to-match-be-reality.md).
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

- [x] §2.3 `soroban_events` row reflects the local ClickHouse pattern per
      ADR 0001 (or is dropped if the row no longer represents shared infra)
      — **dropped**; expanded "Removed rows" prose explains why
- [x] §5.3 placeholder Soroban AMM Backfill row replaced with accurate
      Stream 1 description (local CLI + local ClickHouse + extraction); the
      `†` footnote about the pending row removed
- [x] §5.6 Stream 1 architecture diagram redrawn for local ClickHouse model;
      processing-rate sub-table rewritten with local-workstation metrics
- [x] §9 Tranche acceptance criteria audited for residual Fargate-shape
      Stream 1 references; fixed if any remain — no residual Fargate
      references found in criteria; added an explicit Tranche 1 work bullet
      for Stream 1 (Stream 1 was previously implicit, only mentioned via
      `soroban_amm.status` in AC #4)
- [x] §10 Soroban AMM Fargate cost line removed; new total recomputed
      (~$32 → ~$30)
- [x] §11.1 `soroban_events` row rewritten for the local ClickHouse pattern;
      monthly saving total updated accordingly — **row dropped** (the BE
      artefact is the `backfill-runner` tool, which belongs in §11.2 dev
      savings, not §11.1 shared infra). Monthly saving unchanged at ~$73
      since the dropped row had been valued at ~$0
- [x] §11.4 opening paragraph and risk table rewritten; BE coupling correctly
      framed as transient backfill-runner invocation, not runtime PG read
- [x] Cross-link to ADR 0001 added wherever §5.6 Stream 1 is touched
- [x] Task 0013 can be marked completed once this PR merges (its three
      Stream 1 acceptance criteria were marked "deferred to 0029")
      — moot in practice: 0013 was already pre-closed in archive at the
      start of this session (commit 45cf3cf, before 0029 was even
      activated), with its three Stream 1 ACs marked "deferred to 0029".
      Those deferred ACs are now satisfied by this PR
- [x] Reviewer (project lead) approves the revision — PR #15 merged to
      develop (squash commit 5ce3a7e)

## Implementation Notes

Single-file change: `docs/prices-api-general-overview.md` (+144/-77 lines
across §2.3, §5.3, §5.6 Stream 1, §9 Tranche 1, §10, §11.1, §11.2, §11.4,
plus a new Revision History entry).

The sweep was relatively short because tasks 0013 (Stream 2 reconciliation)
and earlier had already updated the shared backbone sections — §3.5
(`backfill_progress`), §4.5 (`GET /backfill/status` response), §6 (RDS
sizing) — to a stream-agnostic local-CLI shape. Stream 1's update therefore
only had to bring the stream-specific sections in line with the same
pattern, plus reframe the BE coupling story.

## Design Decisions

### From Plan

1. **Stream 1 is a local Rust CLI consuming a local ClickHouse instance
   populated by BE's `backfill-runner --target=clickhouse`**, per ADR 0001.
   The CLI decodes ScVal XDR via `stellar-xdr`, buckets to 1-min OHLCV,
   and runs a one-shot completion push to cloud RDS.

2. **§3.5 / §4.5 already encode the "one-shot AMM CLI push" pattern** —
   no schema or API changes needed. They were updated during 0013 to
   reference ADRs 0001 + 0005 jointly.

### Emerged

3. **Dropped §2.3 `soroban_events` row entirely** rather than rewriting it.
   §2.3 is "Components Shared with Block Explorer (no additional charge)" —
   ongoing shared infrastructure. The BE `backfill-runner` is a transient
   one-shot prep tool, not shared infra. Belongs in §11.2 development
   savings instead. Same logic applied to §11.1 (also infrastructure-only).
   Expanded the "Removed rows" prose in both sections to enumerate both
   removals (the prior Fargate-cluster row + the new `soroban_events` row)
   with cross-links to the relevant ADRs.

4. **Added a `backfill-runner` row to §11.2** ("Development Savings") to
   capture the BE-shared artefact that Stream 1 consumes. Also added a row
   for the BE-authored CH `soroban_events` schema (`clickhouse-prod-schema.sql`)
   for symmetry with how §11.2 already lists `stellar-xdr` and CDK patterns.

5. **Stream 1 binary name `soroban-amm-backfill`** chosen autonomously.
   ADR 0001 doesn't pin a name; chose this to mirror `sdex-backfill`'s
   convention from ADR 0005.

6. **Added an explicit Tranche 1 work bullet for Stream 1** even though
   the original §9 acceptance criterion #4 already exercises Stream 1
   via `soroban_amm.status`. Without the work bullet, Stream 1 looked
   under-specified compared with the SDEX bullet. The criterion was left
   unchanged — the shape is correct.

7. **Backfill total recomputed to ~$30** (was ~$32). Just dropped the $2
   Soroban Fargate line; RDS stays at $30, S3 archive reads stay $0. The
   "~95% reduction vs ADR 0002 / Fargate-era ~$636" framing carries
   through unchanged.

8. **§11.4 risk table replaced wholesale**, not minimally tweaked. The
   three original BE-PG-flavored risks (schema change, DB gaps, BE DB
   offline) didn't all apply once Stream 1's source moved to local CH.
   Replaced with five local-CH-flavored risks: schema drift during prep,
   `backfill-runner` writer bugs (BE task 0206), ledger-range misconfig
   on prep, `backfill-runner` itself unavailable, plus the existing
   gap-detection mitigation preserved. Added a Fargate-fallback breadcrumb
   pointing at task 0017 for the unlikely case the laptop is impractical
   (ADR 0001 §Consequences calls this out).

## Future Work

None spawned from this task — it closes out the design-doc reconciliation
sweep that started in 0013. If more ADRs land that change either stream's
shape, a separate task will be needed.
