---
id: "0045"
title: "Cross-team bundle with BE: settle Hetzner CH tenancy, fan-out, capacity, certs, cost-share"
type: RESEARCH
status: active
related_adr: ["0007"]
related_tasks: ["0044", "0011", "0038", "0039", "0040"]
tags: [layer-research, priority-high, effort-medium, coordination, block-explorer, hetzner, clickhouse, cross-team]
links:
  - "../archive/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/S-refactor-recommendation.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../../../soroban-block-explorer/lore/1-tasks/active/0216_RESEARCH_hetzner-clickhouse-deploy/README.md"
  - "../../../../soroban-block-explorer/lore/1-tasks/active/0227_FEATURE_infra-hetzner-ansible-playbook.md"
history:
  - date: 2026-05-18
    status: backlog
    who: okarcz
    note: >
      Spawned from task 0044's synthesis §4 + §5.1. Drives the four
      BE-conversation clusters to written commitments so ADR 0007
      can transition proposed → accepted and the rewrites of
      blocked tasks 0011/0038/0039/0040 can be sequenced. Mostly
      coordination work, not engineering — calendar time depends
      on BE responsiveness.
  - date: 2026-05-19
    status: active
    who: okarcz
    note: >
      Promoted to active. Task 0044 closed and merged to develop
      (PR #20, squash commit ba6cfa4). Starting the cross-team
      bundle work on a dedicated branch.
---

# Cross-team bundle with BE: settle Hetzner CH tenancy, fan-out, capacity, certs, cost-share

## Summary

Drive the four cross-team agenda clusters from task 0044's
synthesis to written commitments with BE (likely fmazur and
whoever owns BE tasks 0216 + 0227). This task is the gating step
between ADR 0007's `proposed` status and its `accepted` status —
without these commitments the implementation tasks have no
contract to bind to.

The deliverable is **a single written record of the agreements**
(landed as a G-note in this task's `notes/` and cross-linked from
ADR 0007). No engineering work in this task itself.

## Context

Task 0044 identified 28 open questions across its seven steps;
synthesis §4 collapsed those into four BE-conversation clusters.
This task brings the four clusters to closure.

## Status: Backlog

Waiting on **task 0044 closure** (which spawns this task and
ADR 0007 simultaneously) and on **BE Hetzner CH approaching
production** (BE tasks 0216 + 0227). The conversation can begin
once 0227 lands the mTLS+Caddy plumbing; agreement on cost-share
and capacity is easier when the box specs are public.

## The four agenda clusters

### Cluster A — Architecture buy-in

Asks (from synthesis §4.1):

- Approve Option 1 from task 0044 I-note: a separate `prices`
  database in BE's CH cluster, with a dedicated CH user.
- Approve Shape B from task 0044 R-stellar-peers note §6:
  rewire bucket → SNS topic, prices-api subscribes its own
  Lambda. One-time BE CDK change.
- Confirm the announcement-not-approval norm for DDL inside
  `prices.*`.

### Cluster B — Capacity, retention, backup

Asks (from synthesis §4.2):

- Hetzner box hardware specs + monthly cost — needed for capacity
  math and cost-share negotiation.
- Confirm Caddy `max_keepalive_conns` headroom for a second
  tenant. Surface BE's current default.
- Confirm BE intent to keep BE ADR 0006 (indefinite S3
  retention). Prices-api's replay story depends on it.
- Add `BACKUP DATABASE prices` as a separate daily Borg target
  so prices-api can be restored independently.
- Backup RPO acceptable for prices-api rows (daily Borg
  granularity vs. RDS PITR). Surface as a heads-up.

### Cluster C — Auth + secrets

Asks (from synthesis §4.3):

- BE issues `prices-api-{env}` client certs (one per env) via
  the existing per-AWS-service issuance script.
- Rotation cadence: 1-year manual + NotAfter alarm; revisit in
  one year.
- Revocation model: rotate CA on compromise.

### Cluster D — Money

Asks (from synthesis §4.4):

- Cost-share number. Open with 5–10% pro-rata proposal
  (~$3–$15/mo per env). Free ride is the friendly alternative;
  flat fee acceptable up to ~$15/env without changing the
  recommendation.
- Re-open if production scales materially.

## Acceptance Criteria

- [ ] One written record of agreements in
      `notes/G-be-agreement-record.md`. Each of the four
      clusters has an outcome (accepted / counter-proposal /
      blocked).
- [ ] If all four clusters reach agreement: update ADR 0007's
      history with `status: accepted` and add a history note
      pointing at the G-note as the basis.
- [ ] If any cluster blocks: update ADR 0007's history with
      what is blocked and why; consider whether the fallback
      (Option 4 sidecar CH from task 0044 I-note) is now the
      primary path.
- [ ] Cross-link the G-note from each affected blocked task
      (0011/0038/0039/0040) so the rewrite reviewers find the
      authoritative contract.

## Out of scope

- The implementation work the agreements unblock (handled by
  the rewrites of 0011/0038/0039/0040 and the schema applier
  task — separate spawns after this task closes).
- Updates to the prices-api design doc §2.3 / §5 / §10 / §11
  — a separate DOCS task, spawned after ADR 0007 is accepted.

## Notes

- Calendar time is the limiting factor, not engineering time.
  Expect 1–2 weeks elapsed if BE is responsive; could stretch
  if the cost-share negotiation needs multiple rounds.
- Bring the entire bundle as a single brief, not four separate
  asks. The four are intertwined (capacity math feeds cost-share;
  cert issuance shape feeds rotation cadence; etc.).
- Coordinate scheduling against BE tasks 0216 + 0227. If 0227
  is still in flight, some Cluster B asks (e.g. final Caddy
  settings) may need to wait for the Ansible playbook to land.
