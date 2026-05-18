---
id: "0044"
title: "Refactor architecture: shared Galexie + AWS Lambda → BE's Hetzner ClickHouse for live data"
type: RESEARCH
status: active
related_adr: ["0001", "0005", "0006"]
related_tasks: ["0009", "0011", "0017", "0038", "0039", "0040"]
tags: [layer-research, priority-high, effort-medium, infra, architecture, aws, shared-infra, block-explorer, clickhouse, hetzner, galexie, refactor]
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../../../soroban-block-explorer/docs/architecture/infrastructure/infrastructure-overview.md"
  - "../../../../soroban-block-explorer/lore/1-tasks/active/0216_RESEARCH_hetzner-clickhouse-deploy/README.md"
  - "../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "../../2-adrs/0005_stream2-sdex-local-workstation-backfill.md"
  - "../../2-adrs/0006_runtime-framework-rust-axum.md"
  - "../archive/0009_RESEARCH_shared-infra-with-block-explorer/README.md"
history:
  - date: 2026-05-18
    status: backlog
    who: okarcz
    note: >
      Drafted as a follow-on to 0009 (shared-infra recommendation) now
      that BE has committed to a Hetzner-hosted ClickHouse production
      data plane (BE task 0216 + §5.6 of BE infrastructure-overview).
      The current prices-api plan (0038/0039/0040) writes live OHLCV
      into a Prices-owned RDS Postgres. This task analyses whether to
      pivot the live-ingestion sink to BE's planned Hetzner ClickHouse
      to share the data plane and avoid standing up a Prices-owned
      RDS — extending the cost-sharing story established by 0009.
  - date: 2026-05-18
    status: active
    who: okarcz
    note: "Promoted from backlog to active to begin research."
---

# Refactor architecture: shared Galexie + AWS Lambda → BE's Hetzner ClickHouse for live data

## Summary

Research and draft a refactored ingestion architecture for prices-api
that reuses BE's shared Galexie + S3 ingestion edge **and** writes
live OHLCV / trade rows into BE's planned Hetzner-hosted ClickHouse
(BE task 0216) instead of a Prices-owned RDS Postgres. The output is
a reasoned recommendation with deployment topology, security/auth
shape (mTLS to Hetzner per BE §5.6), schema-ownership boundaries,
and migration impact on the existing blocked tasks 0038 / 0039 / 0040.

## Context

**Where we are today.** Task [0009](../archive/0009_RESEARCH_shared-infra-with-block-explorer/README.md)
established the first cost-sharing layer with BE: Galexie ECS, S3
ledger bucket, VPC, and NAT Gateway are BE-owned; prices-api adds
itself as a second S3 PutObject notification target. That research
was framed around a Prices-owned RDS Postgres as the live sink:
tasks [0011](./0011_FEATURE_bootstrap-cdk-with-ssm-platform-lookups.md),
[0038](../blocked/0038_FEATURE_prices-ledger-processor-lambda.md),
[0039](../blocked/0039_FEATURE_prices-periodic-workers-lambda-set.md),
[0040](../blocked/0040_FEATURE_prices-api-gateway-and-read-handlers.md)
all assume that RDS.

**What changed.** BE has since committed to graduating its local-dev
ClickHouse pilot to a **production deployment on a Hetzner-hosted
dedicated server** (BE task 0216, captured in §5.6 of BE's
infrastructure-overview). Hetzner hosts the data plane only; the
AWS-side application keeps API + Lambda. As part of that migration
BE is restructuring its AWS topology: Lambdas move out of the VPC,
the long-running ingestion task moves to a public subnet, the NAT
Gateway is removed, and auth between AWS-side workloads and the
Hetzner database uses mutual TLS.

**What this task asks.** If BE is paying for and operating a
production ClickHouse on Hetzner anyway, the cost-saving framing of
0009 extends naturally: can prices-api also write its live OHLCV /
trade rows into that same ClickHouse rather than provisioning its
own RDS? This research produces the analysis, the proposed
topology, and a clear go/no-go recommendation — not the
implementation.

## Inputs to read

1. **BE infrastructure overview** —
   `../../../../soroban-block-explorer/docs/architecture/infrastructure/infrastructure-overview.md`
   Focus on:
   - §2.3 Event-Driven Ingestion Path (Galexie → S3 → Lambda chain)
   - §3 Target System Topology (Stellar peers → Galexie → S3 →
     processor → DB)
   - §5.1 Ingestion Components (Galexie on ECS Fargate, Captive Core)
   - §5.2 Storage Components (incl. the ClickHouse pilot + Hetzner
     production §5.6)
   - §5.6 Production ClickHouse on Hetzner (mTLS, post-CH topology)
   - §6.4 External Dependency Boundary — "Stellar network peers —
     live data feed for Galexie (ingest-time)"
2. **BE task 0216 notes** —
   `../../../../soroban-block-explorer/lore/1-tasks/active/0216_RESEARCH_hetzner-clickhouse-deploy/`
   Especially `notes/S-decisions.md` for the architecture decisions
   already made on the Hetzner side.
3. **Prices-api current design** —
   `../../../docs/prices-api-general-overview.md` §0, §1.2, §2.1,
   §2.3, §5.2, §11.
4. **Existing follow-ups built on RDS assumption** —
   tasks 0011, 0017, 0038, 0039, 0040 (see frontmatter `links`).
5. **Stellar peers / Galexie live feed** — verify how Galexie
   subscribes to peers via Captive Core. Starting points:
   Stellar developer docs on Captive Core ledger streaming and BE's
   ADR set referenced from §5.1.

## Research Plan

### Step 1 — Catalogue the BE Hetzner shape

Distill from §5.6 + BE task 0216 the parts of BE's Hetzner-CH plan
that are externally consumable: physical host topology, network
ingress, mTLS identity model, schema ownership boundary, multi-
tenant assumptions (does the cluster expose more than one DB / set
of tables?). Capture as `notes/R-be-hetzner-ch-shape.md`.

### Step 2 — Map prices-api live-ingest needs onto that shape

Take the existing ingestion contract from 0038 (S3 PutObject →
decode → extract → 1-min OHLCV UPSERT) and map each downstream
write target from RDS to ClickHouse:

- `price_ohlcv` (1m candles): map UPSERT-with-incremental-merge
  semantics (ADR 0004 columns) to ClickHouse's `ReplacingMergeTree`
  / `AggregatingMergeTree` / native materialised view options. Note
  that ClickHouse does not have Postgres-style row UPSERT; the
  merge contract has to be re-expressed.
- Rollups (15m → 1h → 4h → 1d → 1w → 1M): which become CH
  materialised views vs. which stay as Lambda-driven aggregations.
- `current_prices` (VWAP across sources): re-express as a CH view
  or keep as a small Postgres / DynamoDB / in-memory layer.
- `assets`, `backfill_progress`, oracle prices, asset discovery
  state: low-write-volume relational tables — decide whether they
  also move to CH or stay in a small relational store.

Output: `notes/R-ingest-target-mapping.md`.

### Step 3 — Stellar peers / Galexie ingest-time path

Confirm and document the live data feed chain end-to-end so the
refactor can be reasoned about without assuming BE's setup:

- Stellar mainnet peers (overlay protocol) — how Galexie discovers
  and subscribes.
- Captive Core — how Galexie embeds stellar-core to receive
  validated `LedgerCloseMeta` per ledger close (~5–6s cadence).
- S3 emit step — file naming, retention, event notification model.
- Second-consumer registration — what changes (if anything) when
  the prices-api Lambda is replaced or moved.

Output: `notes/R-stellar-peers-galexie-live-feed.md`.

### Step 4 — Auth and network boundary

The AWS↔Hetzner hop is the new boundary. Capture:

- mTLS identity issuance + rotation (who issues, where the cert
  material lives — Secrets Manager? ACM PCA?).
- Network path: Lambda (post-VPC-exit per BE §5.6) → public
  internet → Hetzner ingress → CH HTTP(S) / native port. Latency
  expectations vs. RDS-in-VPC.
- Failure mode: what happens to live ingestion if the Hetzner side
  is unreachable for N minutes — buffer in S3 (already inherent
  in the S3-event model) vs. add a queue.

Output: `notes/R-aws-hetzner-auth-network.md`.

### Step 5 — Schema-ownership boundary

BE owns the CH schema today (per §5.2 + BE task 0204). If
prices-api writes into the same cluster:

- Does it write into a **separate database** within the same CH
  cluster, with its own schema and migration tooling?
- Or shared tables with a `source = 'prices-api'` column?
- Migration / DDL coordination: who runs DDL, how are
  prices-api-owned schema changes versioned and applied without
  stepping on BE's pipeline?

Output: `notes/I-schema-ownership-options.md`.

### Step 6 — Cost model delta

Quantify what changes vs. the current RDS-based plan:

- Removed: Prices-owned `db.t4g.micro/small` RDS, RDS Proxy,
  RDS backup storage, RDS data transfer.
- Added: incremental Hetzner CH cost (likely $0 incremental if BE
  is buying the box anyway; capture explicitly as a cost-share
  agreement item).
- Changed: Lambda → Hetzner egress (NAT Gateway already removed
  per BE §5.6; public-internet Lambda egress is free, ingress to
  Hetzner is on Hetzner's side).
- Operational: one less production DB to operate; mTLS cert
  lifecycle added.

Output: `notes/R-cost-delta.md`.

### Step 7 — Synthesis and recommendation

Final `notes/S-refactor-recommendation.md` with:

- TL;DR go/no-go for the refactor.
- Recommended topology (sequence diagram + component list).
- Concrete impact on each of tasks 0011, 0017, 0038, 0039, 0040
  (rewrite / retarget / archive / unchanged).
- Open questions that need a cross-team decision with BE
  (likely: schema ownership, mTLS cert issuer, cost-share
  agreement).
- Suggested follow-up tasks (ADR + implementation tasks).

## Acceptance Criteria

- [ ] All five inputs above read and summarised in `notes/R-*.md`.
- [ ] Stellar peers → Galexie → S3 → Lambda live-feed chain
      documented from first principles (not just "see BE doc").
- [ ] Side-by-side mapping of prices-api write targets from RDS to
      ClickHouse, including the OHLCV merge-semantics re-expression.
- [ ] AWS-Lambda ↔ Hetzner-CH auth + network path documented with
      mTLS cert lifecycle and failure-mode analysis.
- [ ] Schema-ownership boundary recommendation with at least two
      options compared (separate DB vs. shared tables).
- [ ] Cost-delta table vs. the current RDS-based plan.
- [ ] Final recommendation note with go/no-go and impact on each
      of the blocked tasks 0011 / 0017 / 0038 / 0039 / 0040.
- [ ] Open questions for the BE team enumerated explicitly so the
      cross-team conversation can happen against a written brief.
- [ ] Follow-up backlog tasks spawned (ADR + implementation) if
      the recommendation is go.

## Open Questions (initial)

These are seeded by the user's framing; the research either answers
them or escalates them to a cross-team decision.

1. **Cost-share agreement** — BE is funding the Hetzner box. Is
   the shared-CH idea a "free ride" because BE has spare capacity,
   or does prices-api co-fund? Needs a written agreement either way.
2. **Schema ownership** — separate database inside the same CH
   cluster, or shared tables with discriminator columns? Affects
   migration tooling and blast radius of accidental DDL.
3. **OHLCV merge semantics on ClickHouse** — `ReplacingMergeTree`
   has eventual-consistency semantics that may or may not be
   compatible with the incremental-merge update expression from
   ADR 0004. The research has to land on a concrete CH table
   engine choice before any implementation task is spawned.
4. **mTLS identity issuer** — does prices-api reuse BE's PCA /
   issuer, or stand up its own and have BE trust it?
5. **Disaster-recovery and backup** — RDS gives PITR for free.
   What is the CH-on-Hetzner backup story, and is it acceptable
   for prices-api's data?

## Notes

- This task is **research only**. Do not write code, do not move
  existing blocked tasks. If the recommendation is go, the
  implementation lands as new tasks (likely an ADR + retargeting
  of 0011/0038/0039/0040).
- Today's date: 2026-05-18. BE task 0216 is **active**, not yet
  shipped — the research may have to make assumptions about parts
  of the Hetzner shape that BE has not finalised. Capture those
  assumptions explicitly so this task can be re-validated when
  0216 closes.
- Coordinate with BE (fmazur per the BE team file, or whoever
  owns 0216) early. The schema-ownership and cost-share questions
  are not unilateral.
- This is a successor in spirit to 0009. The matrix-and-options
  format from 0009's `notes/` is a good template — reuse it.
