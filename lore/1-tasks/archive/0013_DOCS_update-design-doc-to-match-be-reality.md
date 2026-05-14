---
id: "0013"
title: "Update prices-api-general-overview.md §2.3/§5.6/§11 to match ADR 0001 + ADR 0002"
type: DOCS
status: completed
related_adr: ["0001", "0002", "0005"]
related_tasks: ["0012", "0014", "0015", "0017", "0022", "0029"]
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
  - date: 2026-05-14
    status: active
    who: okarcz
    note: >
      Activated via /promote-task. Scope amendment needed before
      writing: ADR 0005 (accepted 2026-05-14) supersedes ADR 0002 —
      Stream 2 is now local-workstation backfill, not Fargate. §5.6
      Stream 2 rewrite must match ADR 0005 (local workstation +
      cloud push), not the Fargate framing in the 2026-05-13 history
      note above. Acceptance criteria below should be re-read with
      ADR 0005 as the canonical Stream 2 source.
  - date: 2026-05-14
    status: active
    who: okarcz
    note: >
      Stream 2 / ADR 0005 reconciliation landed on branch
      `docs/0013_update-design-doc-to-match-be-reality`
      (commits deca40d, e359814, 643c53a, 3bf83a9, 9d22f3c). PR opening
      next. Stream 1 / ADR 0001 reconciliation was deliberately deferred
      from each per-turn scope and is now spawned as
      [task 0029](../backlog/0029_DOCS_update-design-doc-stream-1-adr-0001.md).
      The Stream-1-flavored acceptance criteria below are marked
      "deferred to 0029"; this task stays active until 0029's PR also
      merges, at which point 0013 can be marked completed.
  - date: 2026-05-14
    status: completed
    who: okarcz
    note: >
      PR #14 squash-merged to develop as commit 5f5b3c2. 7 commits
      landed: 5 doc-edit commits (§2.3, §3.5, §4.5, §5.3, §5.6 Stream 2,
      §6, §8, §9, §10, §11.1, §11.4 — 11 sections), 1 lore-housekeeping
      commit (spawning task 0029 + marking Stream-1 criteria deferred),
      1 doc-housekeeping commit (Revision History table at the top of
      the design doc). User decided to complete 0013 now rather than
      keep it active for the Stream 1 follow-up — task 0029 is fully
      independent and will carry its own PR. Five of the eight original
      acceptance criteria ticked; the three Stream-1 criteria are
      explicitly deferred to 0029.
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

> The criteria below were originally written against ADR 0001 + ADR 0002.
> ADR 0005 superseded ADR 0002 on 2026-05-14 (Stream 2 moved from Fargate
> to local workstation CLI). The Stream 2 work landed on this branch
> against ADR 0005, not 0002 — the `[x]` marks below reflect that.
> Stream-1-flavored criteria are deferred to [task 0029](../backlog/0029_DOCS_update-design-doc-stream-1-adr-0001.md).

- [x] §2.3 rows accurately reflect BE-owned vs. Prices-owned: no
      claim of BE-shared Fargate cluster (row dropped per ADR 0005);
      `soroban_events` PG-read claim **deferred to 0029** — still
      describes BE Postgres read; ADR 0001 moves it to local ClickHouse
- [ ] §5.6 Stream 1 rewritten per ADR 0001: local CH instance
      (dev-laptop populated by BE `backfill-runner --target=clickhouse`),
      Tranche 1 fast path preserved, hours-not-weeks claim retained
      **(deferred to 0029)**
- [x] §5.6 Stream 2 rewritten per **ADR 0005** (supersedes ADR 0002):
      local Rust CLI on operator workstation, anonymous
      `s3://aws-public-blockchain` reads, local Postgres sink, separate
      `sdex-cloud-push` step, no BE runtime/data coupling, BE
      `xdr-parser` crate consumed as a git Cargo library dep only.
      The original "Fargate archive-reader" framing in this criterion
      was overridden by ADR 0005.
- [x] §9 Tranche 1 acceptance criteria: Stream 2 backfill "running and
      progressing" is the deliverable bar; full historical completion
      is explicitly out of scope for Tranche 1 (extends past Tranche 3).
      Updated to use push-cadence freshness (`sdex.last_push_at`) instead
      of Fargate heartbeat. Tranche 2 and Tranche 3 acceptance criteria
      also updated alongside.
- [x] §11.1 cost table updated; ECS Fargate cluster row dropped (Stream 2
      is local per ADR 0005); monthly saving total corrected $71 → $73.
      Soroban-AMM-related row pending in [task 0029](../backlog/0029_DOCS_update-design-doc-stream-1-adr-0001.md).
- [x] §11.4 risk section: Stream 2 has zero BE runtime/data coupling at
      any point (new "Stream 2 (SDEX) coupling" subsection added).
      Stream 1 coupling rewording **deferred to 0029** — the opening
      paragraph and risk table still describe the pre-ADR-0001
      BE-Postgres read pattern.
- [x] Cross-links from §5.6 / §11.4 to **ADR 0005** added (Stream 2).
      Cross-link to ADR 0001 in §5.6 / §11.4 **deferred to 0029**.
- [ ] Reviewer (project lead) approves the revision (will be tracked on
      the open PR for this branch; final tick once 0029 also merges)

**Additional sections reconciled with ADR 0005 (not in original criteria
but tightly coupled to §5.6 Stream 2):**

- [x] §3.5 `backfill_progress` schema: dropped `last_heartbeat`,
      `rate_per_hour`, `eta_hours`; added `last_push_at`
- [x] §4.5 `GET /backfill/status` response: removed `task_healthy`,
      `last_heartbeat`, `rate_ledgers_per_hour`,
      `estimated_hours_to_completion`; added `last_push_at`
- [x] §5.3 Ingestion Workers: split the combined Backfill Task row into
      three rows (`sdex-backfill`, `sdex-cloud-push`, placeholder
      Soroban AMM with † footnote pointing at 0029)
- [x] §6 RDS sizing: replaced db.m6g.large continuous-write paragraph
      with the push-window pattern
- [x] §8 Tech Stack Summary: Runtime row no longer says "Fargate for
      Galexie and backfill task" — split into Lambda / Galexie Fargate /
      local CLI explicitly
- [x] §10 Cost: SDEX Fargate line dropped ($216 → $0); RDS during
      backfill resized ($393 → ~$30); S3 reads zeroed (anonymous);
      new total ~$32. Soroban AMM Fargate line ($2) deferred to 0029.
