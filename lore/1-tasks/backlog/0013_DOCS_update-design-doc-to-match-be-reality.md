---
id: "0013"
title: "Update prices-api-general-overview.md §2.3/§5.6/§11 to match BE reality"
type: DOCS
status: backlog
related_adr: ["0001"]
related_tasks: ["0009", "0010", "0012", "0014", "0015"]
tags: [priority-medium, effort-small, docs, infra, clickhouse]
links:
  - "../../../../docs/prices-api-general-overview.md"
  - "../../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md"
  - "../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
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
---

# Update prices-api-general-overview.md §2.3/§5.6/§11 to match BE reality

## Summary

The Prices API technical design document contains two assumptions that contradict the
Block Explorer's actual infra:

1. **§2.3 row "ECS Fargate cluster"** and **§11.1 row "ECS Fargate cluster"** claim BE
   provides a multi-tenant cluster for Prices backfill tasks. BE rejected Fargate backfill
   (BE ADR 0010); no such production cluster exists.
2. **§2.3 row "Block Explorer `soroban_events` table"**, **§5.6 Stream 1**, and **§11.1
   row 6** assume BE persists decoded JSONB topics+data. BE ADRs 0029/0033 indicate the
   table is appearance-only with read-time XDR fetch.

## Context

Spawned from research task 0009. Once tasks 0010 (schema verification) and 0012 (Fargate
design) land, the design doc should be updated for accuracy.

## Implementation

- Revise §0 wording on "shared core infrastructure" to drop the ECS cluster claim.
- Rewrite §2.3 rows 5 and 6 reflecting reality (one-stream or two-stream depending on
  0010 outcome).
- Rewrite §5.6 backfill plan in line with the Prices-owned Fargate design from 0012.
- Update §11 cost table to reflect Prices-owned cluster (cluster overhead ~$0 still
  holds; backfill compute now charged to Prices, which §10 already does).
- If the AMM stream collapses into archive reads, update tranche milestones in §5.6 and
  §9 accordingly.

## Acceptance Criteria

- [ ] §2.3 rows accurately reflect BE-owned vs. Prices-owned
- [ ] §5.6 Stream 1 rewritten per ADR 0001: local CH instance
      (dev-laptop populated by BE `backfill-runner --target=clickhouse`),
      Tranche 1 fast path preserved, hours-not-weeks claim retained
- [ ] §5.6 Stream 2 updated per task 0020's recommendation (only
      after 0020 lands; deferred until then)
- [ ] §11.1 cost table updated; no claim that Prices does not pay for ECS cluster overhead
      if it doesn't apply
- [ ] Cross-link to BE ADRs 0010/0029/0033/0040/0044 in §11.4 risk section
- [ ] Cross-link to prices-api ADR 0001 from §5.6 and §11.4
- [ ] §11.4 note: BE's ClickHouse copy is the source for Tranche 1
      Soroban-AMM backfill (time-boxed, dev-laptop hosted, no
      AWS-deployed CH cluster involved); prices-api live runtime
      does not depend on BE CH infra
- [ ] Reviewer (project lead) approves the revision
