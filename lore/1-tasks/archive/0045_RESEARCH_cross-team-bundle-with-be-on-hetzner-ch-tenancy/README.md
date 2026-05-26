---
id: "0045"
title: "Cross-team bundle with BE: settle Hetzner CH tenancy, fan-out, capacity, certs, cost-share"
type: RESEARCH
status: completed
related_adr: ["0007"]
related_tasks: ["0044", "0046", "0047", "0011", "0038", "0039", "0040"]
tags: [layer-research, priority-high, effort-medium, coordination, block-explorer, hetzner, clickhouse, cross-team]
links:
  - "../0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/S-refactor-recommendation.md"
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
  - date: 2026-05-19
    status: blocked
    who: okarcz
    by: ["0046"]
    note: >
      Moved to blocked pending 0046's empirical capacity / cost
      numbers. The BE conversation brief (drafted on PR #21) lands
      hand-waved 5-10% pro-rata stance for Cluster D cost-share
      and Cluster B capacity; both need real numbers before going
      to BE. 0046 produces them from a 10k-ledger sample. Unblock
      0045 once 0046 closes and update the brief's Cluster B/D
      asks with the empirical figures.
  - date: 2026-05-19
    status: blocked
    who: okarcz
    by: ["0047"]
    note: >
      Blocker re-targeted. 0046 closed (empirical numbers landed,
      brief updated, BE responded). Now blocked on 0047 — cross-
      tenant throughput verification, spawned from the BE
      agreement record's Cluster B6 TBD. ADR 0007 stays `proposed`
      until 0047 closes GREEN or YELLOW. Cluster B7 (Borg cadence)
      and D12 (cost-share number) are soft TBDs handled in-line
      with BE once 0047 resolves.
  - date: 2026-05-20
    status: active
    who: okarcz
    note: >
      Reopened from blocked. The narrow purpose of this task —
      drive the four cross-team clusters to written commitments
      with BE — is satisfied: agreement record landed at
      notes/G-be-agreement-record.md with 10 yes / 0 no / 3 TBD,
      and the architectural shape is locked in via Cluster A
      all-yes. The remaining TBDs are tracked as follow-ups, not
      as gates on this task's deliverable: B6 throughput
      verification spawned as task 0047 (engineering gate before
      implementation lands), B7 Borg cadence and D12 cost-share
      handled in-line with BE post-completion. Operator override:
      ADR 0007 transitions proposed → accepted on the strength of
      Cluster A acceptance; a RED outcome from 0047 would
      supersede the ADR per its own Alternative 3 (sidecar CH).
  - date: 2026-05-20
    status: completed
    who: okarcz
    note: >
      Closed. All 4 acceptance criteria met. Deliverables:
      G-be-conversation-brief.md (13 asks, sent), G-be-agreement-
      record.md (BE responses, 10 yes / 0 no / 3 TBD). Side
      effects on close: ADR 0007 transitioned proposed → accepted
      (history entry 2026-05-20, Decision section rewritten);
      agreement record cross-linked from blocked tasks
      0011/0038/0039/0040 (they stay blocked on their pre-existing
      technical predecessors); related_tasks expanded to include
      0046 + 0047. Follow-ups: 0047 (already in backlog) is now
      the hard engineering gate before 0011/0038/0039/0040 begin
      their rewrites; D12 cost-share and B7 Borg cadence handled
      in-line with BE; new DOCS task to be spawned for the
      design-doc refactor to match ADR 0007.
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

- [x] One written record of agreements in
      `notes/G-be-agreement-record.md`. Each of the four
      clusters has an outcome (accepted / counter-proposal /
      blocked). **Done** — 10 yes / 0 no / 3 TBD recorded.
- [x] If all four clusters reach agreement: update ADR 0007's
      history with `status: accepted` and add a history note
      pointing at the G-note as the basis. **Done with caveat** —
      operator override (see Design Decisions → Emerged below).
      Cluster A all-yes is treated as sufficient for the
      architectural acceptance; the three remaining TBDs (B6, B7,
      D12) are reframed as engineering / commercial follow-ups,
      not architectural unknowns. ADR 0007's history entry
      (2026-05-20) records this basis.
- [x] If any cluster blocks: update ADR 0007's history with
      what is blocked and why; consider whether the fallback
      (Option 4 sidecar CH from task 0044 I-note) is now the
      primary path. **Done** — no cluster outright blocks; B6 is
      the only TBD with architectural-revision potential and is
      captured in ADR 0007's revised Decision section as the hard
      engineering gate. A RED outcome from task 0047 supersedes
      the ADR via Alternative 3 (sidecar CH).
- [x] Cross-link the G-note from each affected blocked task
      (0011/0038/0039/0040) so the rewrite reviewers find the
      authoritative contract. **Done** — links + 2026-05-20
      history note added to each of the four blocked tasks (they
      stay blocked on their pre-existing technical predecessors,
      not on 0045 or ADR 0007).

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

## Artifacts in this task

- `notes/G-be-conversation-brief.md` — single-bundle brief to BE.
  13 asks across the four clusters, with proposed numbers, fallbacks,
  and an outcome-tracking table. Sent.
- `notes/G-be-agreement-record.md` — BE's responses to the brief.
  10 yes / 0 no / 3 TBD (B6 spawns 0047; B7 + D12 pending). Records
  what unblocks now vs. what stays gated.

## Follow-up tasks spawned

- **0047** — Cross-tenant throughput verification on shared Hetzner
  CH. Now reframed as the **hard engineering gate before the
  rewrites of 0011/0038/0039/0040 can begin**, not as a gate on
  ADR 0007 acceptance (which landed on the strength of Cluster A).
  Spawned from B6 TBD.

## Implementation Notes

- The brief (`notes/G-be-conversation-brief.md`) bundled all 13
  asks into a single conversation, not four separate ones. This
  was deliberate: capacity math (B) feeds cost-share (D); cert
  issuance shape (C) feeds rotation cadence (C); etc. BE was
  able to respond to the bundle in a single pass.
- The agreement record (`notes/G-be-agreement-record.md`)
  preserves BE's verbatim outcome table in §5 for traceability.
- B7 (Borg cadence) accepted as "yes" but BE flagged a follow-up:
  daily vs. weekly given the empirical <1 GB/yr footprint from
  task 0046. Recommendation in the record §2.3 is daily; revisit
  if BE pushes back.
- D12 (cost-share number) is the only Cluster D TBD. BE wants
  measured-rather-than-estimated denominators once their
  `default.*` lands on the production box. Brief's opening
  (~1-2% / $1-2 per env/mo) holds.

## Issues Encountered

- **None — coordination workflow.** No engineering surprises.
  Calendar time was the only variable; BE responded within
  a single iteration. The "could need multiple rounds" caveat
  in §Notes did not materialise.

## Design Decisions

### From Plan

1. **Single-bundle brief, not four separate asks.** Stated in
   §Notes from task creation. Validated by BE's single-pass
   response.

2. **Daily Borg recommendation regardless of empirical footprint.**
   The cost difference at <1 GB/yr is trivial; the RPO difference
   (1 day vs. 7 days of replay) matters more. Replay-from-S3 path
   takes ~minutes per day, so recovering a week of lost OHLCV
   would be operationally unpleasant.

3. **D12 opening at ~1-2% pro-rata, not the brief's original
   5-10%.** Empirical footprint from task 0046 (~0.45 GB/yr, ~74
   bytes/ledger, order of magnitude smaller than the hand-waved
   estimate) shifted the proposal down by ~5×.

### Emerged

4. **Operator override: ADR 0007 accepted on Cluster A acceptance,
   not on all-four-clusters-yes.** The original transition rule
   from ADR 0007's frontmatter required "after that conversation
   closes" with the implicit expectation of all-yes. The
   conversation has closed with 10 yes / 0 no / 3 TBD; Cluster A
   (architecture buy-in) is the load-bearing cluster for the
   architectural decision, and it is all-yes. The remaining TBDs
   are engineering follow-ups (0047 throughput) and commercial
   follow-ups (D12 cost-share), not architectural unknowns. ADR
   0007's Decision section was rewritten to reflect this — the
   "conditional go" framing is replaced with explicit follow-up
   gates that sequence implementation work without re-opening the
   architectural question.

5. **0047 reframed: engineering gate, not ADR gate.** The
   agreement record §2.2 originally framed 0047 as the "final
   gate on ADR 0007 → accepted". With the operator override above,
   0047 is reframed as the hard gate before the implementation
   tasks (0011/0038/0039/0040) can begin their rewrites. A RED
   outcome supersedes ADR 0007 via Alternative 3 (sidecar CH on
   the same Hetzner box) rather than blocking acceptance.

6. **Cross-linking pattern for the four blocked impl tasks.**
   Each of 0011/0038/0039/0040 gets the agreement record added to
   `links:` plus a single 2026-05-20 history entry noting ADR
   0007 acceptance. They stay blocked on their pre-existing
   technical predecessors (0011, 0037 chain) — this task's
   completion does not unblock them.

## Future Work

- **Task 0047** — already spawned (backlog). Activates when
  BE 0216 + 0227 land and the Hetzner CH is queryable. Final
  engineering gate before 0011/0038/0039/0040 rewrites.
- **D12 cost-share number** — finalise once BE's `default.*` is
  on the production box and a measured fraction is computable
  (~1-2 weeks post-cutover). Handled in-line with BE; no
  dedicated task spawned because the dollar values are
  trivial and the mechanism (D13) is already agreed.
- **B7 Borg cadence** — confirm daily vs. weekly with BE.
  Handled in-line; no dedicated task.
- **DOCS task: refactor `docs/prices-api-general-overview.md` to
  reflect ADR 0007.** §1.1, §2.1, §3, §5.2, §6, §8, §10, §11 all
  describe RDS Postgres as the live sink. With ADR 0007 now
  accepted, the design doc rewrite is the next docs deliverable
  — separate task to be spawned.
