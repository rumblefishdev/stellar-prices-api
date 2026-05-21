---
id: "0017"
title: "Local ClickHouse instance setup and access for prices-api Tranche 1 backfill"
type: FEATURE
status: backlog
related_adr: ["0001"]
related_tasks: ["0015", "0018"]
tags: [layer-infra, priority-high, effort-small, milestone-M1, infra, backfill, clickhouse, block-explorer]
milestone: 1
links:
  - "../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "../archive/0015_RESEARCH_redefine-backfill-with-be-clickhouse-events/notes/S-redesigned-backfill-recommendation.md"
  - "../../../../soroban-block-explorer/lore/1-tasks/archive/0205_FEATURE_backfill-runner-clickhouse-target-flag.md"
  - "../../../../soroban-block-explorer/lore/1-tasks/active/0206_FEATURE_clickhouse-persist-real-inserts/README.md"
history:
  - date: 2026-05-12
    status: backlog
    who: okarcz
    note: >
      Spawned from task 0015 closure. ADR 0001 commits Stream 1 to a
      local-CH-sourced backfill on a developer laptop (okarcz's). This
      task is the operational landing: spin up CH locally, run BE's
      backfill-runner against it, document the access mechanism that
      lets the prices-api Tranche 1 consumer query the laptop's CH.
---

# Local ClickHouse instance setup and access for prices-api Tranche 1 backfill

## Summary

Stand up a local ClickHouse instance on okarcz's developer laptop,
populated by running BE's `backfill-runner --target=clickhouse`
(BE task 0205) for the Soroban-activation-onward ledger range
(~ledger 48.5M to current tip, ~8.5M ledgers). Document the access
mechanism that lets the prices-api Tranche 1 consumer (separate
follow-up) query this CH instance to extract Soroban AMM swap events.

Tear-down trigger: once Tranche 1 backfill completes and the
extracted OHLCV trade points are persisted in prices-api PostgreSQL.

## Context

ADR 0001 commits prices-api Stream 1 to local-CH-sourced backfill.
This task is the infrastructure side. It is gated on BE task 0206
(real CH writer) reaching a quality bar that prices-api can consume —
BE task 0117 (local backfill benchmark) is the proxy signal.

## Implementation

- Docker compose service mirroring BE's task 0204 compose definition
  (CH version, port mapping, healthcheck, volume mount).
- Run BE's `backfill-runner --target=clickhouse` against
  `<archive-source>` for the Soroban range. Estimate disk: BE's task
  0117 benchmark is the input here.
- Decide access mechanism for the prices-api consumer:
  - **Option a:** SSH tunnel from the consumer's host to laptop CH
    HTTP port (8123). Lowest friction, requires laptop online.
  - **Option b:** Cloudflare tunnel / tailscale-exposed CH for
    multi-machine prices-api workflow.
  - **Option c:** Read-only CH snapshot exported to S3, consumer
    reads from S3 via clickhouse-local.
- Document the chosen mechanism in `lore/3-wiki/` (or inline in
  prices-api repo docs).
- Capture the actual disk usage, completion time, and any
  population errors as a closing note for ADR 0001's "consequences"
  section.

## Acceptance Criteria

- [ ] Local CH instance running on okarcz's laptop with BE schema
      applied (idempotent `init.sql` from BE task 0204).
- [ ] BE `backfill-runner --target=clickhouse` completes against
      the Soroban-activation-onward ledger range with no
      `parts_to_throw_insert` errors and zero parser-data loss
      (verified against BE task 0206's coverage contract).
- [ ] Access mechanism chosen and documented; prices-api Tranche 1
      consumer (separate follow-up) can run a smoke query
      (`SELECT count() FROM soroban_events WHERE signature = 'swap'`)
      against the laptop CH.
- [ ] Disk usage, run time, and any anomalies captured as a closing
      note appended to ADR 0001 or a `notes/G-backfill-run-log.md`.
- [ ] Tear-down checklist documented (when to nuke the volume,
      how to do it cleanly).

## Notes

- BE's task 0206 must be merged before this task can run to completion.
  If it is still active when this task starts, coordinate with BE
  (fmazur) on whether a development-grade CH writer is good enough
  for prices-api's Tranche 1 consumption, or whether we wait for
  0206's full landing.
- Storage estimate: BE has not published a hard number for the
  Soroban-activation-onward window. Order-of-magnitude estimate
  from extrapolating ADR 0044's ~550 GB full-mainnet backfill is
  ~100–150 GB for the Soroban-only range; verify against BE task
  0117 benchmark output.
