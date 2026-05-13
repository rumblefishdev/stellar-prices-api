---
id: "0013"
title: "Update prices-api-general-overview.md §2.3/§5.6/§11 to match ADR 0001 + ADR 0002"
type: DOCS
status: backlog
related_adr: ["0001", "0002"]
related_tasks: ["0012", "0014", "0015", "0017", "0022"]
tags: [priority-medium, effort-small, docs, infra, clickhouse, sdex, backfill]
links:
  - "../../../../docs/prices-api-general-overview.md"
  - "../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "../../2-adrs/0002_stream2-sdex-archive-backfill-independent-of-be.md"
  - "../../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md"
history:
  - date: 2026-05-11
    status: backlog
    who: okarcz
    note: "Spawned from 0009 future work. Two design-doc assumptions contradict BE production state."
  - date: 2026-05-12
    status: backlog
    who: okarcz
    note: "Refs added by task 0014: BE ADR 0044 (CH pilot). Design doc should briefly acknowledge the pilot exists and is local-only / read-empty today, NOT plan against it."
  - date: 2026-05-12
    status: backlog
    who: okarcz
    note: >
      Scope amended by task 0015 + ADR 0001. The Stream 1 §5.6
      rewrite is no longer "collapse into archive reads" but
      "source from a local CH instance populated by BE's
      `backfill-runner --target=clickhouse`" — see ADR 0001 for
      the canonical decision. §11.4 note must drop the
      "don't plan against the CH pilot" framing and instead
      record that prices-api intentionally consumes BE's CH for
      a time-boxed Tranche 1 backfill window, with no live
      runtime dependency on BE infra.
  - date: 2026-05-13
    status: backlog
    who: okarcz
    note: >
      Scope amended by ADR 0002. Stream 2 §5.6 rewrite is now
      "fully prices-api-owned archive-read Fargate task, ledger 1
      to tip, BE stellar-xdr crate as library dep only — no BE
      runtime/data coupling". Drop any framing that ties Stream 2
      to BE CH `operations_appearances` (no Option B, no Option D,
      no trim-ratio pre-filter); task 0021 was canceled by ADR 0002
      and that path is closed. §11.4 must record that the only
      BE coupling at any point in prices-api's lifecycle is the
      time-boxed Stream 1 CH window (Tranche 1 only).
---

# Update prices-api-general-overview.md §2.3/§5.6/§11 to match ADR 0001 + ADR 0002

## Summary

The Prices API technical design document contains assumptions written
before BE's actual production state was known and before ADRs 0001 and
0002 fixed Stream 1 and Stream 2 architecture. Update §2.3, §5.6, §9
acceptance criteria, and §11 to match.

## Context

Spawned from research task 0009. Once tasks 0012 (Stream 2 Fargate
design) and 0017 (Stream 1 local CH setup) land, the design doc should
be updated for accuracy. Two ADRs now drive the rewrite:

- **ADR 0001** — Stream 1 (Soroban AMM) sources from a locally-run CH
  populated by BE's `backfill-runner --target=clickhouse`. Time-boxed
  to Tranche 1.
- **ADR 0002** — Stream 2 (SDEX) is fully independent: prices-api-owned
  Fargate archive-reader, ledger 1 → tip, BE `stellar-xdr` crate as
  library dep only.

## Implementation

- Revise §0 wording on "shared core infrastructure" to drop the ECS
  cluster claim (BE has no Fargate backfill in production).
- Rewrite §2.3 rows 5 and 6 reflecting reality:
  - Row "ECS Fargate cluster" — Prices-owned, not shared.
  - Row "Block Explorer `soroban_events` table" — replaced by ADR 0001's
    local-CH model (no BE PG read; no live cross-account CH read).
- Rewrite §5.6 Stream 1 per ADR 0001: local CH instance (dev-laptop
  populated by BE `backfill-runner --target=clickhouse`), Tranche 1
  fast path preserved, hours-not-weeks claim retained.
- Rewrite §5.6 Stream 2 per ADR 0002: prices-api-owned Fargate
  archive-reader, ledger 1 → tip, no CH pre-filter, BE
  `stellar-xdr` crate as library dep only. Tranche 1 acceptance =
  "task running and progressing", not "task completed".
- Update §9 Tranche 1 acceptance criteria text to reaffirm: Stream 2
  full historical completion is **not** a Tranche 1 deliverable; the
  task being deployed, running, and emitting heartbeats is the bar.
- Update §11 cost table to reflect Prices-owned cluster.
- Cross-link both ADRs from §5.6 and §11.4.

## Acceptance Criteria

- [ ] §2.3 rows accurately reflect BE-owned vs. Prices-owned (no
      claim of BE-shared Fargate cluster; no claim of BE-shared
      `soroban_events` PG read)
- [ ] §5.6 Stream 1 rewritten per ADR 0001: local CH instance
      (dev-laptop populated by BE `backfill-runner --target=clickhouse`),
      Tranche 1 fast path preserved, hours-not-weeks claim retained
- [ ] §5.6 Stream 2 rewritten per ADR 0002: prices-api-owned
      Fargate archive-reader, ledger 1 → tip, no CH pre-filter, no
      BE runtime/data coupling, BE `stellar-xdr` crate consumed as a
      library Cargo dep only
- [ ] §9 Tranche 1 acceptance criteria: Stream 2 backfill "running and
      progressing" is the deliverable bar; full historical completion
      is explicitly out of scope for Tranche 1 (extends past Tranche 3)
- [ ] §11.1 cost table updated; ECS cluster overhead correctly
      attributed to Prices
- [ ] §11.4 risk section: only Stream 1 carries any BE coupling, and
      that coupling is time-boxed to the Tranche 1 backfill window;
      Stream 2 has zero BE runtime/data coupling at any point
- [ ] Cross-links from §5.6 / §11.4 to ADR 0001 and ADR 0002
- [ ] Reviewer (project lead) approves the revision
