---
id: "0012"
title: "Design SDEX backfill on a local workstation (Stream 2, ADR 0005 — supersedes ADR 0002)"
type: FEATURE
status: completed
related_adr: ["0002", "0003", "0005"]
related_tasks: ["0011", "0014", "0020", "0022", "0023", "0027", "0028"]
tags: [priority-high, effort-medium, design, local-backfill, workstation, postgres, cloud-push, sdex, stream-2]
links:
  - "../../2-adrs/0002_stream2-sdex-archive-backfill-independent-of-be.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../2-adrs/0005_stream2-sdex-local-workstation-backfill.md"
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
  - date: 2026-05-14
    status: active
    who: okarcz
    note: >
      ADR 0002 superseded by ADR 0005 — Stream 2 backfill now runs
      locally on the operator's workstation (BE pattern, mirrors
      crates/backfill-bench). Design G-note rewritten as
      G-sdex-backfill-local-design.md; cloud push of finalised
      prices tables is a separate post-backfill step. Task 0027
      refactored from Fargate impl to local CLI impl (unblocks
      from task 0011); new task 0028 spawned for the cloud-push
      tool. Directory slug retained ("…-fargate") for branch /
      PR continuity; the README and G-note are the authoritative
      shape.
  - date: 2026-05-14
    status: completed
    who: okarcz
    note: >
      Design closed. PR #13 merged to develop as squash commit
      0052660. Deliverables: ADR 0005 (accepted, supersedes 0002),
      ADR 0002 marked superseded, G-sdex-backfill-local-design.md
      (~580 lines + xdr-parser future-direction note), README
      rewritten, task 0027 refactored to local-CLI impl (~170 lines,
      blocked-on-0011 removed), task 0028 spawned for cloud-push
      (~130 lines, blocked on 0011 + 0027). All AC met; impl tasks
      ready for activation when operator is ready to start.
---

# Design SDEX backfill on a local workstation (Stream 2)

## Summary

ADR 0005 (supersedes ADR 0002) commits Stream 2 (SDEX, ledger 1 → tip)
to a local Rust CLI on the operator's workstation that mirrors BE's
`crates/backfill-bench` / `crates/backfill-runner` pattern. This task
produces the **operational design** for that CLI: CLI shape,
partition pipeline, local Postgres schema additions, resumability
semantics, observability, failure-mode taxonomy, runbook structure,
and the Rust module split mapping onto task 0022's spec. The actual
binary / schema / runbook landing is task 0027; the post-backfill
cloud-push tool is task 0028.

## Context

The first-pass design (committed 2026-05-13 on this branch as
`G-sdex-backfill-fargate-design.md`) committed to a prices-api-owned
ECS Fargate task per ADR 0002. Reviewing BE's actual backfill
implementation during design revealed a simpler, BE-proven
local-workstation pattern (BE ADR 0010, `crates/backfill-bench`,
`crates/backfill-runner`). The user accepted the pivot on 2026-05-14:
SDEX backfill runs locally, with a separate post-backfill cloud-push
step. ADR 0005 records the architectural change; ADR 0002 is now
`superseded`.

What's preserved from the previous design:

- The BE-independence stance (no BE runtime, no BE DB during backfill).
- The `xdr-parser` consumption pattern (library crate, not runtime
  service) — now via git Cargo dep instead of workspace path dep.
- Task 0022's filter + decode-and-bucket spec as the contract the
  Rust binary implements clause-by-clause.
- ADR 0003's `price_ohlcv` PK shape including `quote_asset_id`.
- The Rust module split (`filter`/`tick`/`canonical`/`price`/`bucket`/
  `checkpoint`) — the spec is host-shape-agnostic, so this carries over.

What's removed:

- Fargate task definition, ECS cluster, IAM roles, CloudWatch alarm,
  SNS topic. None of these exist in the new shape.
- `infra/aws-cdk/` work and the dependency on task 0011 (CDK bootstrap).
  Local backfill is unblocked.
- Heartbeat metric design and per-AWS-throttle batching — replaced by
  stdout tracing + a final summary block.

What's added:

- §11 of the G-note: cloud-push design sketch (the post-backfill
  step). Spawned as task 0028, blocked on task 0011 (cloud RDS exists)
  and task 0027 (local data exists).

## Design output

The full design lives in
[`notes/G-sdex-backfill-local-design.md`](./notes/G-sdex-backfill-local-design.md).

Section map:

| § | Topic                                                                  |
| - | ---------------------------------------------------------------------- |
| 0 | Scope (design covers vs task 0027 / 0028 lands)                        |
| 1 | Architecture overview (workstation, S3 archive, local PG, cloud-push) |
| 2 | Direction and range strategy — tip-backward chunks via operator       |
| 3 | CLI shape — clap, `--start`/`--end`/`--database-url`/`--temp-dir`     |
| 4 | Partition pipeline — single-slot prefetch, BE pattern verbatim         |
| 5 | Resumability — `backfill_progress` + per-ledger atomic tx + partition skip |
| 6 | Observability — stdout tracing JSON; no CloudWatch                    |
| 7 | Failure modes — S3 sync, PG, parser panic, sleep/network, disk, OOM   |
| 8 | Local Postgres bootstrap — Docker, migrations                         |
| 9 | Runbook outline — phases, start, stop, inspect                        |
| 10 | Rust module split — 1:1 onto task 0022's spec                        |
| 11 | Cloud-push design sketch — natural-key remap, batched UPSERT        |
| 12 | Handoff checklists for task 0027 and task 0028                       |

## Acceptance Criteria

- [x] Design G-note covers local CLI shape, partition pipeline, local
      Postgres schema, resumability, observability, runbook outline,
      and module split (`notes/G-sdex-backfill-local-design.md`).
- [x] ADR 0005 drafted and accepted; ADR 0002 marked `superseded by 0005`
      with a history entry pointing at the new ADR.
- [x] Direction strategy decided (operator-chosen tip-backward chunks,
      ascending in-binary walk) with reasoning grounded in BE pattern
      mirroring and task 0022's whole-row-replacement UPSERT semantics (§2).
- [x] `xdr-parser` consumption pattern decided (git Cargo dep at
      pinned commit, per ADR 0005 §3); BE repo is read-only.
- [x] Module split table maps each Rust module to a specific section
      of task 0022's filter-strategy or decode-and-bucket spec (§10).
- [x] ADR 0003's `quote_asset_id` PK migration called out as a
      pre-backfill schema step (§5.1 / §8 / §12.1).
- [x] Cloud-push step designed at sketch level (§11): natural-key
      `assets` remap, batched `price_ohlcv` UPSERT, idempotent.
- [x] Impl tasks spawned: 0027 (local backfill CLI; unblocked) and
      0028 (cloud-push tool; blocked on 0011 + 0027).

## Implementation Notes

Design-only delivery. No code, no CDK, no runbook landing — those
deliverables moved to task 0027 (local) and task 0028 (cloud push).

Files produced on this branch:

- `notes/G-sdex-backfill-fargate-design.md` (~480 lines, removed) →
  renamed to `notes/G-sdex-backfill-local-design.md` (~580 lines,
  fully rewritten).
- New ADR `lore/2-adrs/0005_stream2-sdex-local-workstation-backfill.md`.
- ADR 0002 marked `superseded by 0005` (frontmatter + history only;
  body untouched per ADR convention).
- This README — design summary + acceptance against the local-backfill
  scope.
- Task 0027 (backlog) refactored from Fargate impl to local CLI impl;
  blocked-on-0011 removed.
- Task 0028 (backlog) spawned for the cloud-push tool; blocked on 0011 + 0027.

## Design Decisions

### From Plan

1. **Design-only scope on activation.** Original AC included a
   staging deploy that requires task 0011 (CDK bootstrap) — still
   in backlog. Scoping to the design artifact and spawning impl as
   a follow-up matches the pattern set by task 0024 → 0026.

2. **Tip-backward chunked direction.** Tranche 1's "≥ 6 months of
   recent history" UX gate is recency-biased. Task 0022's §5.4
   whole-row replacement makes direction correctness-neutral, so
   the choice is UX-driven. G-note §2 carries the full reasoning.

### Emerged

3. **Local workstation pattern (ADR 0005 supersedes ADR 0002).**
   First-pass design committed to Fargate per ADR 0002. Reviewing
   BE's actual backfill implementation (BE ADR 0010,
   `crates/backfill-bench`, `crates/backfill-runner`) during the
   design phase revealed a BE-proven simpler shape; user accepted
   the pivot 2026-05-14. The architectural commitments preserved
   from ADR 0002 (BE-independence, library-only XDR parser dep)
   are explicit in ADR 0005's Decision section.

4. **`xdr-parser` via git Cargo dep, not workspace path.** Per the
   user directive "never modify BE repo," a git Cargo dependency
   pinning a commit hash satisfies the constraint while consuming
   BE's `decompress_zstd` + `deserialize_batch` helpers verbatim.
   Alternative considered: port the ~50 LOC of zstd glue into our
   repo. Decision logged in ADR 0005 §Rationale.

5. **Ascending in-binary walk, operator-chosen chunks.** BE's
   `partitions_for_range` returns ascending partitions; reversing
   direction in-binary would diverge from BE's pattern with no
   UX benefit (operator already picks chunk order). G-note §2.

6. **Single-laptop v1; multi-laptop deferred.** BE ADR 0040
   documents the surrogate-id remap + watermark reconciliation
   hazards of parallel-laptop backfill. Prices-api's `assets`
   table has the same surrogate-id hazard. v1 ships single-laptop;
   v2 lands a `prices-db-merge` analog when wall-clock measurement
   motivates it. ADR 0005 §9, §Alternative 2.

7. **Cloud push as a separate, smaller tool (task 0028).** Direct
   cloud-RDS writes during backfill would burn provisioned IOPS
   over weeks and add cross-internet latency to every UPSERT. A
   batched `INSERT … ON CONFLICT` push step is far cheaper and
   keeps the backfill unblocked from cloud-RDS provisioning. The
   surrogate-id remap pattern is narrowed to two tables, vs. BE's
   full `db-merge` complexity.

8. **Directory slug retained as `…-fargate`.** The on-disk task
   directory keeps its 2026-05-13 slug for branch + PR continuity;
   the README's title and body are the authoritative shape. v2 may
   rename if it ever feels meaningfully wrong.

9. **G-note file renamed (`G-sdex-backfill-fargate-design.md` →
   `G-sdex-backfill-local-design.md`).** The rename is a clean
   signal that the document shape changed; the previous file's
   content is preserved in git history on this branch's first
   commits.

## Future Work

Two follow-ups:

- **[0027](../../backlog/0027_FEATURE_sdex-backfill-local-impl.md)** —
  Implementation landing for the local backfill CLI. Cargo workspace,
  Rust binary, schema migrations (including ADR 0003 PK change),
  `docker-compose.yml`, runbook, smoke test. Blocked-on-0011 removed
  by ADR 0005; task is unblocked.
- **[0028](../../backlog/0028_FEATURE_sdex-cloud-push.md)** —
  Implementation landing for the post-backfill cloud-push tool.
  `sdex-cloud-push` binary, natural-key `assets` remap, batched
  `price_ohlcv` UPSERT, runbook section. Blocked on task 0011 (cloud
  RDS exists) and task 0027 (local data exists).
