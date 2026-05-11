---
id: "0010"
title: "Verify BE soroban_events_appearances schema for Prices AMM backfill"
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0009"]
tags: [priority-high, effort-small, infra, block-explorer, schema, backfill]
links:
  - "../../../../soroban-block-explorer/lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md"
  - "../../../../soroban-block-explorer/lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md"
history:
  - date: 2026-05-11
    status: backlog
    who: okarcz
    note: "Spawned from 0009 future work. Resolves the row-8 hard mismatch in the shared-infra matrix."
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

## Implementation

- Read BE migrations under `../soroban-block-explorer/crates/*/migrations/` and any
  schema docs under `../soroban-block-explorer/docs/architecture/database-schema/`.
- Confirm whether `topics` / `data` are persisted as JSONB or only as appearance pointers.
- If decoded payloads exist somewhere (perhaps a different table), document them.
- If not, write a short note recommending Option D1 from
  `lore/1-tasks/active/0009_*/notes/I-integration-options.md`.

## Acceptance Criteria

- [ ] Definitive answer: are decoded Soroban event topics+data in BE's RDS or not?
- [ ] If yes: document the exact table + columns + JSONB shape
- [ ] If no: spawn a follow-up to revise Prices §5.6 (one-stream archive backfill)
- [ ] Update `lore/1-tasks/active/0009_*/notes/S-shared-infra-recommendation.md` open
      question #2 with the resolution
