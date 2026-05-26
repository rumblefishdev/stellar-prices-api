---
id: '0057'
title: 'Define the Tranche 1 (M1) task set — author 7 new tasks, tag 7 existing'
type: DOCS
status: completed
related_adr: ['0001', '0005', '0007']
related_tasks:
  [
    '0011',
    '0017',
    '0027',
    '0028',
    '0034',
    '0037',
    '0038',
    '0050',
    '0051',
    '0052',
    '0053',
    '0054',
    '0055',
    '0056',
  ]
tags:
  [
    layer-docs,
    priority-medium,
    effort-small,
    milestone-M1,
    meta,
    lore-admin,
    task-definition,
    tranche-1-scope,
  ]
milestone: 1
links:
  - '../../../docs/prices-api-general-overview.md'
history:
  - date: 2026-05-21
    status: active
    who: okarcz
    note: >
      Meta-task to own the lore commit that defines the Tranche 1
      (milestone-M1) task set. Created during a working session
      that authored 7 new tasks (0050–0056) and tagged 7 existing
      tasks with `milestone-M1`. Spawned this task so the commit
      has a clean lore-task association (per the project's
      branch-before-commit + Conventional-Commits-with-lore-scope
      convention) without retro-fitting any single child task as
      the umbrella.
  - date: 2026-05-21
    status: completed
    who: okarcz
    note: >
      Merged via PR #27 (squash, develop). 15 task files touched
      (8 new: 0050–0056 + 0057; 7 modified: 0011, 0017, 0027,
      0028, 0034, 0037, 0038). All 15 carry `milestone: 1`
      frontmatter field (matches BE 0240 convention). T1 task
      set complete; execution gated on BE 0227 + task 0047.
---

# Define the Tranche 1 (M1) task set

## Summary

Author the lore task set that fully covers the Tranche 1 / M1
deliverable (design-doc §9 "Infrastructure & Real-time Ingestion,
Weeks 1–4") from `docs/prices-api-general-overview.md`. Output: 7
new tasks (0050–0056) and the `milestone-M1` tag applied to 7
existing T1-relevant tasks (0011, 0017, 0027, 0028, 0034, 0037,
0038).

## Context

Prior to this task, the §9 Tranche 1 work bullets and acceptance
criteria were partially covered by existing tasks (CDK bootstrap,
Stream 1 CH prep, SDEX backfill, Ledger Processor kernel + Lambda,
Phoenix WASM tolerance) but lacked owning tasks for the BE-side
prep, schema migration, mTLS client crate, AMM CLI, asset
discovery, `/backfill/status` endpoint, and CloudWatch alarms.
Without owning tasks those deliverables risk slipping into
informal coordination.

This task captures the act of producing that full task set so the
commit has a clean lore-task scope without retro-fitting any
single child task as the umbrella for the others.

## What got produced

**New tasks** (`backlog/`, all `milestone-M1`):

| ID   | Title                                                               | Area                     |
| ---- | ------------------------------------------------------------------- | ------------------------ |
| 0050 | BE-side prep — SNS fan-out + mTLS issuance + prices DB provisioning | layer-infra, cross-team  |
| 0051 | ClickHouse `prices.*` schema + MV chain migration                   | layer-database           |
| 0052 | ClickHouse mTLS client shared crate                                 | layer-backend            |
| 0053 | Soroban AMM Backfill CLI (`soroban-amm-backfill`)                   | layer-indexing, stream-1 |
| 0054 | Asset Discovery Lambda (T1 minimal scope)                           | layer-indexing           |
| 0055 | `GET /backfill/status` endpoint (T1 isolated)                       | layer-backend            |
| 0056 | CloudWatch alarms — SDEX push freshness + mTLS NotAfter             | layer-infra              |

**Existing tasks tagged with `milestone-M1`** (no shape changes):

0011 (CDK bootstrap), 0017 (Local CH for Stream 1 prep), 0027
(SDEX local backfill), 0028 (SDEX cloud-push), 0034 (Phoenix
multi-WASM tolerance), 0037 (Tranche 1 Ledger Processor skeleton),
0038 (Prices Ledger Processor Lambda).

## Decisions captured

- **Canonical T1 tag is `milestone-M1`** — older ad-hoc
  `tranche-1` tag was removed from all 14 M1 tasks on
  2026-05-21 to consolidate vocabulary.
- **Carve-outs** from larger bundles:
  - 0054 carves Asset Discovery out of 0039's 5-worker bundle
    so the T1-scoped slice ships independent of T2 work.
  - 0055 carves `/backfill/status` out of 0040's full API
    surface for the same reason (explicit T1 acceptance criterion).
  - 0056 keeps observability separate from 0011's CDK
    bootstrap so it can land without conflating bootstrap +
    observability scopes.

## Acceptance Criteria

- [x] 7 new tasks authored in `backlog/`
- [x] 7 existing T1 tasks tagged with `milestone-M1`
- [x] `tranche-1` tag removed from all 14 M1 tasks for
      vocabulary consolidation
- [x] Memory entry `tranche-1-task-set.md` records carve-out
      reasoning + canonical tag for future sessions
- [x] PR opened against `develop` and merged (PR #27, squash,
      commit 674d625)
- [x] `milestone: 1` frontmatter field added to all 15 M1 tasks
      (matches BE 0240 convention; follow-up after initial
      task-set landed)

## Out of scope

- T2 (Public API) and T3 (Production Launch) task sets — separate
  meta-work when those tranches are scoped.
- Implementation of any of the 14 tasks — this task only defines
  the set.
- Changes to the design doc itself — the doc is the canonical
  source; tasks reflect what it says.

## Notes

- Task scope intentionally limited to T1. Per the user's
  instruction, T2 and T3 tasks are deferred so focus stays on
  shipping T1.
- Several M1 tasks are blocked on BE 0227 (Hetzner Ansible) +
  task 0047 (cross-tenant throughput check). The M1 task set is
  complete as a definition; execution unblocks as gates clear.
