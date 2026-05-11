---
id: "0013"
title: "Update prices-api-general-overview.md §2.3/§5.6/§11 to match BE reality"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0009", "0010", "0012"]
tags: [priority-medium, effort-small, docs, infra]
links:
  - "../../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-05-11
    status: backlog
    who: okarcz
    note: "Spawned from 0009 future work. Two design-doc assumptions contradict BE production state."
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
- [ ] §5.6 backfill plan matches the implementation decided in 0012
- [ ] §11.1 cost table updated; no claim that Prices does not pay for ECS cluster overhead
      if it doesn't apply
- [ ] Cross-link to BE ADRs 0010/0029/0033 in §11.4 risk section
- [ ] Reviewer (project lead) approves the revision
