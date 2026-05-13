---
id: "0012"
title: "Design SDEX backfill on Prices-owned Fargate (Stream 2, ADR 0002)"
type: FEATURE
status: active
related_adr: ["0002"]
related_tasks: ["0011", "0014", "0020", "0022"]
tags: [priority-high, effort-large, infra, ecs, fargate, backfill, sdex, stream-2]
links:
  - "../../2-adrs/0002_stream2-sdex-archive-backfill-independent-of-be.md"
  - "../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/G-sdex-trade-extraction-design.md"
  - "../../../../soroban-block-explorer/lore/2-adrs/0010_local-backfill-over-fargate.md"
  - "../../../../soroban-block-explorer/lore/2-adrs/0040_multi-laptop-backfill-snapshot-merge-hazards.md"
history:
  - date: 2026-05-11
    status: backlog
    who: okarcz
    note: "Spawned from 0009 future work. Replaces the design's assumed BE shared cluster (BE ADR 0010)."
  - date: 2026-05-12
    status: backlog
    who: okarcz
    note: "Refs added by task 0014: BE ADR 0040 (multi-laptop backfill, refines ADR 0010) and BE ADR 0044 (CH pilot — possible future AMM source). Neither changes the C1 Fargate decision today."
  - date: 2026-05-13
    status: backlog
    who: okarcz
    note: >
      Scope narrowed by ADR 0002: this task now covers SDEX (Stream 2)
      only — Stream 1 (Soroban AMM) is settled by ADR 0001 (local CH
      backfill, task 0017). The "depends on 0010 AMM-stream verdict"
      branch is removed. Extractor logic is fed by task 0020's G-note
      (archived) and the consolidated spec produced by task 0022.
      BE-coupling exits the design entirely: archive reads only, BE
      `stellar-xdr` parser imported as a library crate.
  - date: 2026-05-13
    status: active
    who: okarcz
    note: "Activated via /promote-task. Starting design work."
---

# Design SDEX backfill on Prices-owned Fargate (Stream 2)

## Summary

ADR 0002 commits Stream 2 (SDEX, ledger 1 → tip) to a fully
prices-api-owned ECS Fargate task that reads `LedgerCloseMeta` directly
from Stellar public history archives, with zero runtime or data coupling
to Block Explorer. This task lands the infrastructure: ECS cluster,
Fargate task definition, IAM roles, CloudWatch heartbeat alarm, and the
operations runbook.

## Context

Stream 1 (Soroban AMM) is settled by ADR 0001 (local CH backfill via
BE's `backfill-runner`, task 0017) and is **not** in this task's scope.
This task is exclusively about Stream 2: SDEX trade extraction across the
full ~57M-ledger Stellar history.

BE ADR 0040 (multi-laptop backfill, accepted 2026-05-07) refines BE
ADR 0010's local-CLI choice with a `db-merge` operator playbook. Confirms
BE has no Fargate backfill in production — Prices needs its own,
consistent with the C1 (own Fargate cluster + task definition)
recommendation from research task 0009.

The trade-extraction algorithm and `TradeTick` output shape live in
archived task 0020's G-note; the consumer-ready filter + decode spec is
produced by task 0022 and is the contract this task consumes.

## Implementation

- ADR 0002 captures the architectural decision; this task is the
  infrastructure landing only (no separate ADR needed).
- Add an ECS cluster + Fargate task definition to the CDK app (task
  0011 must land first).
- Define task IAM role: read Stellar public archive (S3 GET on archive
  bucket), write Prices RDS (`price_ohlcv` historical partitions +
  `backfill_progress`), emit CloudWatch heartbeat metric.
  **Explicitly: no IAM for BE RDS, BE CH, or any BE-owned resource.**
- Task binary embeds the BE-authored `stellar-xdr` parser crate (Cargo
  workspace dep), reads `LedgerCloseMeta`, filters trade-shaped
  operations (types 2, 3, 4, 12, 13), emits `TradeTick`s per
  `ClaimAtom`, bucket-aggregates into 1m OHLCV rows, UPSERTs into
  `price_ohlcv` historical partitions.
- Resumable: checkpoint in `backfill_progress.current_ledger`; restart
  reads the row and resumes.
- Direction of processing (oldest-first vs newest-first vs chunked)
  decided here based on candle-merge semantics under ON CONFLICT and
  on the desired `earliest_data_available` UX (tip-backward gives
  Tranche 1 ≥ 6 months back faster; ledger-1-forward gives stable
  oldest-data progress).
- Add CloudWatch alarm on backfill heartbeat (>20 min stale → SNS).
- Document task lifecycle (start, resume, stop) and operations runbook.

## Acceptance Criteria

- [ ] ADR 0002 referenced from CDK code comments and runbook (no
      separate ADR needed for this task)
- [ ] Fargate cluster + SDEX backfill task definition in `infra/aws-cdk/`
- [ ] IAM role scoped to archive S3 read + Prices RDS write only — no
      BE resource access
- [ ] BE-authored `stellar-xdr` parser crate consumed as a Cargo
      workspace dependency; verified via `cargo tree`
- [ ] CloudWatch heartbeat alarm wired (SNS on >20 min stale)
- [ ] Runbook in `docs/runbooks/backfill-sdex.md`
- [ ] Test deploy to staging; backfill processes a sample 10k-ledger
      range with `TradeTick` rows landing in `price_ohlcv` and
      `backfill_progress.current_ledger` advancing
- [ ] Spec from task 0022 is folded into the Rust implementation
      module (`filter`, `decode`, `bucket` boundaries match the spec)
