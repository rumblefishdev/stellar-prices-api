---
id: "0012"
title: "Design SDEX + AMM backfill on Prices-owned Fargate cluster"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0009", "0010", "0011"]
tags: [priority-high, effort-large, infra, ecs, fargate, backfill]
links: []
history:
  - date: 2026-05-11
    status: backlog
    who: okarcz
    note: "Spawned from 0009 future work. Replaces the design's assumed BE shared cluster (BE ADR 0010)."
---

# Design SDEX + AMM backfill on Prices-owned Fargate cluster

## Summary

The Prices API design assumes the SDEX backfill task runs in BE's shared ECS cluster.
BE has no such production cluster — it rejected Fargate backfill in favor of a local
CLI (BE ADR 0010). Design and provision a Prices-owned Fargate setup for the long-running
SDEX (and possibly AMM) backfill streams.

## Context

Spawned from research task 0009. See
`lore/1-tasks/active/0009_*/notes/I-integration-options.md` Dimension C for option
analysis (Fargate vs. EC2 vs. Step-Function-orchestrated Lambda). Recommendation is
Option C1 (own Fargate cluster + task definition).

The exact AMM stream design depends on task 0010's outcome:
- If BE's `soroban_events_appearances` carries decoded payloads → keep two streams.
- If not → collapse into one archive-based stream (Option D1 in the integration note).

## Implementation

- Wait for 0010 verdict on AMM backfill source.
- Add an ECS cluster + Fargate task definition to the CDK app (task 0011 must land first).
- Define task IAM role with: read public archive, read BE RDS (if D-not-D1), write Prices
  RDS, write to backfill_progress, emit CloudWatch heartbeat metric.
- Add CloudWatch alarm on backfill heartbeat (>20 min stale → SNS).
- Document task lifecycle (start, resume, stop) and operations runbook.

## Acceptance Criteria

- [ ] ADR drafted: "Prices-owned ECS Fargate for backfill" with cost/operational tradeoffs
- [ ] Fargate cluster + SDEX backfill task definition in `infra/aws-cdk/`
- [ ] AMM backfill task definition (one stream or two, per 0010 outcome)
- [ ] CloudWatch heartbeat alarm wired
- [ ] Runbook in `docs/runbooks/backfill.md`
- [ ] Test deploy to staging; backfill processes a sample range successfully
