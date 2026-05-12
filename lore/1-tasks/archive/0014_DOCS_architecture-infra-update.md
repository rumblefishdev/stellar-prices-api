---
id: "0014"
title: "Architecture infra update — propagate BE develop changes (ADR 0044 CH pilot, ADR 0040 backfill) into Prices backlog tasks"
type: DOCS
status: completed
related_adr: []
related_tasks: ["0009", "0010", "0012", "0013"]
tags: [priority-medium, effort-small, docs, infra, block-explorer]
links:
  - "../../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md"
  - "../../../../soroban-block-explorer/lore/2-adrs/0040_multi-laptop-backfill-snapshot-merge-hazards.md"
  - "../../../../soroban-block-explorer/docs/architecture/infrastructure/infrastructure-overview.md"
  - "../../../../soroban-block-explorer/docs/architecture/technical-design-general-overview.md"
history:
  - date: 2026-05-12
    status: backlog
    who: okarcz
    note: "Re-checked 0009 against BE develop @ b462c86. Two new BE artifacts post-date 0009 and need to be reflected in Prices follow-up tasks 0010/0012/0013."
  - date: 2026-05-12
    status: active
    who: okarcz
    note: "Promoted to active on branch docs/0014_architecture-infra-update."
  - date: 2026-05-12
    status: completed
    who: okarcz
    note: "Edits applied to 0010/0012/0013 (3 frontmatter link blocks + 3 history entries + new acceptance bullets). 0011 reviewed and untouched. 0009 archive untouched per convention. Branch ready for commit."
---

# Architecture infra update — propagate BE develop changes into Prices backlog tasks

## Summary

Re-check of research task 0009 against the latest soroban-block-explorer `develop` branch
(@ `b462c86`). Two BE artifacts post-date 0009's research and need to be reflected in the
Prices-side follow-up tasks 0010, 0012, and 0013. No new task is created; the existing
backlog tasks are amended with the new references.

## Context

Task 0009 was archived 2026-05-11. Since then:

1. **BE ADR 0044 — ClickHouse pilot parallel store** (proposed 2026-05-08; merged into BE
   develop with task 0204). Adds `crates/db-clickhouse` + a local-only Docker compose
   service mirroring the Postgres schema, with one critical divergence: a full-content
   `soroban_events` table (per-row XDR inlined). The pilot is **read-empty** (no indexer
   write path, no API read path) and **explicitly NOT part of the AWS-hosted runtime** —
   production stays Postgres-only until a follow-up ADR with measured PASS/FAIL criteria.
2. **BE ADR 0040 — Multi-laptop backfill** (accepted 2026-05-07). Refines BE ADR 0010
   (`local-backfill-over-fargate`) with a `db-merge` operator playbook for parallel
   laptop snapshots. Reaffirms: no Fargate, local CLI only.
3. **BE infra-overview §5.2** updated 2026-05-10 to describe the local-dev CH pilot;
   **BE tech-design §3 component table** annotates the RDS row to mention the read-empty
   CH pilot lives next to it locally, not in AWS.

## Impact on 0009's recommendation

Core conclusions hold:

- No shared production Fargate cluster on the BE side → Prices owns its backfill compute
  (Option C1 in `0009_*/notes/I-integration-options.md`).
- BE production still has no decoded soroban_events payload → AMM Tranche 1 plan
  (Option D3 → D1) remains correct **for production today**.
- The cost-sharing story (Galexie + S3 + VPC + NAT) is unchanged.

What changes:

- **0010 (verify BE soroban_events schema)** gets a wider question: not just "does
  `soroban_events_appearances` carry decoded payload?" but also "if/when the CH pilot
  graduates to AWS with full-content `soroban_events`, does that re-open Stream 1 of
  Prices §5.6?". Answer is gated on BE's own follow-up ADR.
- **0012 (Prices-owned Fargate backfill)** gets ADR 0040 added as a refining reference for
  the operator-driven backfill model BE actually runs, plus a "future option" line about
  the CH pilot as a possible AMM source.
- **0013 (update Prices design doc)** gets ADR 0044 added to the cross-link list and a
  brief acknowledgement of the CH pilot — the design doc should note that BE may have a
  full-content event store in future, but should NOT plan against it until 0044's
  follow-up ADR lands.
- **0009 archive is left untouched** by convention; this task is the trail.

## Implementation

- [x] Amend `lore/1-tasks/backlog/0010_RESEARCH_verify-be-soroban-events-schema.md`:
      add ADR 0044 to `links`, expand summary to include the CH-pilot dimension, add
      acceptance bullet on the CH future-option question.
- [x] Amend `lore/1-tasks/backlog/0012_FEATURE_design-prices-owned-backfill-fargate.md`:
      add ADR 0040 + ADR 0044 to `links`, add a "Future option" subsection covering the
      CH-as-AMM-source possibility, note ADR 0040 confirms BE's local-CLI choice.
- [x] Amend `lore/1-tasks/backlog/0013_DOCS_update-design-doc-to-match-be-reality.md`:
      add ADR 0044 to `links` and to the §11.4 cross-link acceptance bullet, add an
      acceptance bullet covering a brief CH-pilot mention in the Prices design doc.
- [x] Regenerate lore index.

## Acceptance Criteria

- [x] All three backlog tasks (0010, 0012, 0013) reference ADR 0044 where relevant
- [x] 0012 also references ADR 0040
- [x] 0009 archive content unchanged (cross-trail via this task's `related_tasks`)
- [x] Prices design doc itself (`docs/prices-api-general-overview.md`) NOT edited here —
      that work is owned by 0013
- [x] Branch `docs/0014_architecture-infra-update` carries the change set

## Design Decisions

### From Plan

1. **Doc-updates-only scope (no re-research notes).** Confirmed with user upfront. The
   re-check fits in this task's README; no separate `notes/` warranted given the small
   delta (one new BE ADR with material impact, one refining ADR).
2. **0009 archive untouched.** Convention is that archived task content is immutable;
   the trail is `related_tasks` cross-links from 0014 → 0009.

### Emerged

3. **0011 (CDK + SSM lookups) skipped.** Read it during the pass; nothing in BE's
   develop changes affects platform-lookup wiring. No edit needed.
4. **No new follow-up backlog tasks spawned.** The CH pilot is read-empty + local-only
   today; any Prices-side action is gated on BE's own follow-up ADR with PASS/FAIL
   criteria. Premature to create a "consume CH from Prices" task before BE commits.

## Issues Encountered

None — straightforward documentation propagation.

## History entry note

Re-check confirmed 0009's core recommendation (separate CDK app + Prices-owned Fargate +
verify-then-archive-read for AMM) still holds. Only annotation needed: future BE ADR
could re-open Stream 1 of Prices §5.6 if the CH pilot graduates to AWS.
