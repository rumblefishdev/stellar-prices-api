---
id: "0012"
title: "Design SDEX backfill on Prices-owned Fargate (Stream 2, ADR 0002)"
type: FEATURE
status: active
related_adr: ["0002", "0003"]
related_tasks: ["0011", "0014", "0020", "0022", "0023", "0027"]
tags: [priority-high, effort-medium, design, infra, ecs, fargate, backfill, sdex, stream-2]
links:
  - "../../2-adrs/0002_stream2-sdex-archive-backfill-independent-of-be.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-filter-strategy.md"
  - "../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md"
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
  - date: 2026-05-13
    status: active
    who: okarcz
    note: >
      Scope re-narrowed to design-only on activation. Task 0011 (CDK
      bootstrap) is still in backlog and explicitly gates the
      "Fargate cluster + task definition in infra/aws-cdk/" criterion.
      Acceptance criteria pruned to the design artifact; CDK landing,
      Rust binary, schema migrations, runbook, and staging smoke test
      moved to spawned impl task 0027.
---

# Design SDEX backfill on Prices-owned Fargate (Stream 2)

## Summary

ADR 0002 commits Stream 2 (SDEX, ledger 1 → tip) to a fully
prices-api-owned ECS Fargate task that reads `LedgerCloseMeta` directly
from Stellar public history archives, with zero runtime or data coupling
to Block Explorer. This task produces the **operational and
infrastructural design** for that task — task definition shape, IAM
contract, processing direction, resumability semantics, heartbeat
alarm, failure-mode taxonomy, runbook structure, and the Rust module
split that maps onto task 0022's spec. The actual CDK / binary / schema
landing is task 0027.

## Context

Stream 1 (Soroban AMM) is settled by ADR 0001 (local CH backfill via
BE's `backfill-runner`, task 0017) and is **not** in this task's scope.
This task is exclusively about Stream 2: SDEX trade extraction across
the full ~57M-ledger Stellar history.

The trade-extraction algorithm and `TradeTick` output shape live in
archived task 0020's G-note; the consumer-ready filter and
decode-and-bucket spec is produced by task 0022 (also archived) and is
the contract this design consumes. ADR 0003 (accepted 2026-05-13) pins
the `price_ohlcv` PK to include `quote_asset_id`; the design surfaces
that as a pre-backfill DDL migration in §11 of the G-note.

The implementation-side acceptance criteria from the original task
file (CDK code, Rust binary, schema migration, runbook, staging smoke
test) have been moved to spawned task 0027. That task is blocked on
task 0011 (CDK bootstrap), which is what made the original
"acceptance includes a staging deploy" framing impossible to satisfy
in this task.

## Design output

The full design lives in
[`notes/G-sdex-backfill-fargate-design.md`](./notes/G-sdex-backfill-fargate-design.md).

Section map:

| § | Topic                                                              |
| - | ------------------------------------------------------------------ |
| 0 | Scope (what this design covers vs what task 0027 lands)            |
| 1 | Architecture overview (Fargate cluster, networking, trigger model) |
| 2 | Processing direction — tip-backward, single-task, with reasoning   |
| 3 | Task definition shape — image, sizing, env vars, secrets, logs     |
| 4 | IAM contract — execution role, task role, **forbidden actions**    |
| 5 | Resumability — `backfill_progress` shape, per-ledger atomicity     |
| 6 | Heartbeat metric + 20-min CloudWatch alarm + SNS topic shape       |
| 7 | Failure modes — S3 5xx, RDS write, parser panic, OOM, clock drift  |
| 8 | Logging — group name, retention, stable event names                |
| 9 | Runbook outline — start, observe, stop, resume, alarm response    |
| 10 | Rust module split — 1:1 mapping onto task 0022's spec             |
| 11 | Handoff checklist for task 0027                                   |
| 12 | Spawned follow-up tasks                                           |

## Acceptance Criteria

- [x] Design G-note covers Fargate task shape, IAM, heartbeat, resumability,
      runbook outline, and module split (`notes/G-sdex-backfill-fargate-design.md`).
- [x] Processing direction decided (tip-backward, single-task) with
      reasoning grounded in Tranche 1 §5.6 acceptance criterion and
      task 0022's whole-row-replacement UPSERT semantics (§2).
- [x] IAM contract enumerates execution-role and task-role policies,
      and lists the BE-related actions that must NOT appear (§4.3).
- [x] Module split table maps each Rust module to a specific section
      of task 0022's filter-strategy or decode-and-bucket spec (§10).
- [x] ADR 0002 and ADR 0003 referenced from the G-note; ADR 0003's
      `quote_asset_id` PK migration called out as a pre-backfill
      schema step (§11).
- [x] Impl task spawned as backlog 0027 covering CDK landing, Rust
      binary, schema migration, runbook, and staging smoke test (§12).

## Implementation Notes

Design-only delivery. No code, no CDK, no runbook landing — those
deliverables moved to task 0027.

Files produced on this branch:

- `notes/G-sdex-backfill-fargate-design.md` (~480 lines) — the full
  operational design.
- This README — design summary + acceptance against the new scope.

## Design Decisions

### From Plan

1. **Design-only scope on activation.** Original AC included a
   staging deploy that requires task 0011 (CDK bootstrap) — still
   in backlog. Scoping to the design artifact and spawning task
   0027 for impl matches the pattern set by task 0024 → 0026.

2. **Tip-backward processing direction.** Tranche 1's "≥ 6 months
   of recent history" UX gate is recency-biased. Task 0022's §5.4
   whole-row replacement makes direction correctness-neutral, so
   the choice is UX-driven. G-note §2 carries the full reasoning.

### Emerged

3. **Disjoint-range parallelisation defined but not provisioned.**
   The original task body left parallelisation open ended. The
   G-note (§2 item 4) explicitly defers it as a v2 escape hatch —
   the binary supports `LEDGER_RANGE_START` / `LEDGER_RANGE_END`
   so disjoint-range fan-out is possible, but CDK ships a single
   task. This avoids over-provisioning CDK code for a capability
   that may never be exercised.

4. **CDK unit-test for forbidden IAM actions.** G-note §4.3
   enumerates BE-related actions that must be absent from the task
   role. To make this verifiable rather than aspirational, the
   design requires the impl task to add a CDK unit test asserting
   the synthesized policy document doesn't contain those statements.
   The original task body did not specify how the "no BE access"
   constraint would be enforced.

5. **20-min heartbeat alarm threshold, justified from §0022 numbers.**
   The original "20 min" figure was a round number. G-note §6.2
   ties it to the measured 311 ledgers/s decode rate plus the
   binary's own §7.1 80-second S3 retry budget, so the threshold
   has a numeric basis rather than a guess.

6. **`stellar-xdr` resolution verified via `cargo tree` in task
   0027, not here.** The original AC said "verified via `cargo
   tree`" but no Rust workspace exists yet. Moved to the impl task
   where it is actually executable.

## Future Work

Single follow-up:

- **[0027](../../backlog/0027_FEATURE_sdex-backfill-fargate-impl.md)** —
  Implementation landing. CDK Fargate stack, Rust binary, schema
  migrations (incl. ADR 0003 PK change), runbook, staging smoke
  test. Blocked on task 0011.
