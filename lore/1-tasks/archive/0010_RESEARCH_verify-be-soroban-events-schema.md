---
id: "0010"
title: "Verify BE soroban_events_appearances schema for Prices AMM backfill"
type: RESEARCH
status: superseded
related_adr: []
related_tasks: ["0009", "0014", "0015"]
tags: [layer-research, priority-high, effort-small, infra, block-explorer, schema, backfill]
links:
  - "../../../../soroban-block-explorer/lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md"
  - "../../../../soroban-block-explorer/lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md"
  - "../../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md"
history:
  - date: 2026-05-11
    status: backlog
    who: okarcz
    note: "Spawned from 0009 future work. Resolves the row-8 hard mismatch in the shared-infra matrix."
  - date: 2026-05-12
    status: backlog
    who: okarcz
    note: "Question widened by task 0014: BE ADR 0044 (CH pilot, local-only / read-empty today) introduces a future full-content `soroban_events` table. Add CH-future-option dimension to the verification."
  - date: 2026-05-12
    status: superseded
    who: okarcz
    by: ["0015"]
    note: >
      Superseded by task 0015. The verification question is answered
      definitively: BE PG has no full-content soroban_events (only
      appearances per ADR 0033); BE ClickHouse production schema
      (`docs/database-schema/clickhouse-prod-schema.sql`,
      populated by BE active task 0206) holds the full per-event
      content with topics_xdr + data_xdr + hoisted signature.
      Task 0015 carries the resulting backfill refactor and
      schema→price-calc mapping that 0010 would otherwise have
      stopped short of.
---

# Verify BE soroban_events_appearances schema for Prices AMM backfill

## Summary

Inspect the actual `soroban_events_appearances` (and any companion) table in the Block
Explorer's RDS / migrations to determine whether decoded JSONB topics+data exist anywhere
in BE's database. The Prices API design (§5.6 Stream 1) assumes they do; BE ADRs 0029/0033
suggest they do not.

## Context

Spawned from research task 0009. The "fast Tranche 1" Soroban AMM backfill in
`docs/prices-api-general-overview.md` §5.6 reads BE's `soroban_events` table to extract
decoded swap data, avoiding ~8.5M ledger archive reads. If that table doesn't carry
decoded payloads, this stream collapses into the SDEX-style archive-read pattern.

Re-check from task 0014 (2026-05-12): BE ADR 0044 introduced a local-only ClickHouse
pilot with a full-content `soroban_events` table. The pilot is read-empty today and is
explicitly **NOT** part of BE's AWS production runtime — but it is a plausible future
source for AMM backfill if it graduates per BE's follow-up ADR. Verification should now
cover both today's RDS shape AND the pilot's stated trajectory.

## Implementation

- Read BE migrations under `../soroban-block-explorer/crates/*/migrations/` and any
  schema docs under `../soroban-block-explorer/docs/architecture/database-schema/`.
- Confirm whether `topics` / `data` are persisted as JSONB or only as appearance pointers.
- If decoded payloads exist somewhere (perhaps a different table), document them.
- If not, write a short note recommending Option D1 from
  `lore/1-tasks/active/0009_*/notes/I-integration-options.md`.
- Inspect BE ADR 0044 + `crates/db-clickhouse/schema/init.sql` to confirm CH-side
  `soroban_events` shape and pilot status; note its read-empty / local-only constraint.

## Acceptance Criteria

- [ ] Definitive answer: are decoded Soroban event topics+data in BE's RDS or not?
- [ ] If yes: document the exact table + columns + JSONB shape
- [ ] If no: spawn a follow-up to revise Prices §5.6 (one-stream archive backfill)
- [ ] CH pilot status documented: confirm pilot is read-empty + local-only and gated on a
      follow-up BE ADR before any cross-project consumption
- [ ] Update `lore/1-tasks/active/0009_*/notes/S-shared-infra-recommendation.md` open
      question #2 with the resolution
