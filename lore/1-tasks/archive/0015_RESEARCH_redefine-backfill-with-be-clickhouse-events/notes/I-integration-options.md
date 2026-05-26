---
title: 'How prices-api consumes BE ClickHouse — four options'
type: idea
status: mature
spawned_from: ../README.md
spawns: []
tags: [integration, clickhouse, block-explorer, options]
links:
  - './R-be-clickhouse-schema-and-status.md'
  - './G-ch-tables-for-price-calculation.md'
  - '../../../archive/0009_RESEARCH_shared-infra-with-block-explorer/notes/I-integration-options.md'
history:
  - date: 2026-05-12
    status: mature
    who: okarcz
    note: 'Four consumption-pattern options sketched; trade-offs captured for synthesis.'
---

# Four options for prices-api → BE ClickHouse consumption

The R-note establishes the schema is right-shaped; the G-note shows
the queries that work. This note answers the **how do we run those
queries** question. Four options, ordered from least to most
prices-api-owned infrastructure.

## Option A — Direct cross-account query against BE's production CH

**Sketch:** prices-api opens a CH client connection to a BE-deployed
ClickHouse cluster in the shared AWS sub-account; runs the
G-note queries live during Tranche 1 backfill **and** as a
continuous read source for go-forward AMM data.

**Pros:**

- Lowest infra to own — zero CH operational surface area for
  prices-api.
- Single source of truth — no replication lag, no drift.
- Aligns with the §11 "infrastructure sharing" theme of the prices
  design doc.

**Cons:**

- **BE has no AWS-deployed CH cluster as of 2026-05-12.** BE task
  0206's CH writer targets local Docker CH for backfill. AWS
  deployment of CH would need to be its own BE project, gated on
  the still-open ADR 0044 Q6 (pilot success criteria) — which
  itself is blocked on first measurements that don't exist yet.
  Option A is **dependent on infra that doesn't exist yet** and on
  a BE go/no-go decision still pending.
- Hard runtime coupling — every prices-api Lambda invocation that
  touches Soroban AMM data is now dependent on BE's CH cluster
  availability.
- BE has not committed to a public/SLA'd read interface for CH;
  asking for one is a non-trivial scope ask.
- IAM / VPC plumbing for cross-account CH access is novel — not
  precedented by the existing BE→prices PG read-only access.

**Verdict:** Not viable as written. Could become viable in 2027+
if BE deploys CH to AWS and exposes a read interface, but planning
the prices-api architecture around it today is betting on infra
that may or may not materialize.

## Option B — One-time ETL pull from BE local CH for historical backfill; prices-api owns go-forward live ingestion

**Sketch:** Two-phase consumption:

1. **Tranche 1 historical** — set up a local CH instance,
   populated by running BE's backfill runner with
   `--target=clickhouse` (BE task 0205's CLI flag). Query the local
   CH with the G-note queries to extract Soroban AMM swap events.
   Decode XDR. Land OHLCV trade points into prices-api PG.
2. **Go-forward live** — prices-api's own Ledger Processor Lambda
   reads `LedgerCloseMeta` from S3 (or the live network), parses
   Soroban events for the AMM contracts of interest, and writes
   trade points to PG in near-real-time. No CH dependency in the
   live path.

**Pros:**

- Zero runtime dependency on BE infra after Tranche 1.
- The local-CH instance can be torn down once backfill is complete
  — no ongoing operational cost.
- Reuses BE's already-built backfill runner (task 0205) instead of
  re-implementing archive iteration in prices-api.
- Schema drift risk bounded to the backfill window — once Tranche
  1 closes, prices-api owns its own ingestion shape.
- Decoupled rollout — prices-api Tranche 2/3 work can proceed even
  if BE's CH plans change.

**Cons:**

- Two ingestion paths to build: the CH-reading backfill consumer
  AND the live-network Soroban event parser. Higher up-front
  engineering cost than Option A's single live read path.
- Local CH instance needs disk for the 11M-ledger backfill output
  — non-trivial (BE's own estimates suggest hundreds of GB
  pre-compression on the full backfill; CH compression cuts that
  significantly but it's still meaningful storage).
- Requires running BE's backfill runner on prices-api's hardware
  (or a one-shot Fargate task) — adds a Fargate workload that
  task 0012 must accommodate.

**Verdict:** Strongest candidate. Couples to BE only where there's
already a precedent (BE's CLI backfill runner is the same tool BE
uses internally — task 0205 shipped it). Decouples the
prices-api's live runtime from BE entirely. The "two ingestion
paths" cost is real but each path is well-shaped: one is a CH
SQL consumer, the other is a Soroban event parser.

## Option C — Local CH replica of BE production data, owned by prices-api

**Sketch:** prices-api operates its own CH cluster (Fargate or
managed CH-as-a-service), receives a continuous data feed from BE
(write-through dual-writer or scheduled snapshot replication),
and serves all prices analytics from it.

**Pros:**

- Full data ownership; no BE runtime dependency.
- Could serve as the analytics database for prices-api itself —
  not just a backfill source.
- Schema-stable: prices-api can pin the schema version it consumes.

**Cons:**

- **Adds CH as a second operational engine in prices-api alongside
  PG.** §11.3 explicitly lists "Block Explorer's database" as not
  shared; running its own CH is a major scope expansion vs the
  current PG-only design.
- Cost: CH cluster operating cost is non-trivial; CH-as-a-service
  (Altinity Cloud, ClickHouse Cloud) adds vendor dependency.
- Replication pipeline from BE → prices CH would be its own
  project to build and maintain.
- §10 cost estimates do not account for this.

**Verdict:** Over-scoped. Would be the right answer if prices-api's
own analytical workload justified CH (e.g., billion-row OHLCV
history with sub-second query SLA). For the current design
(Lambda + RDS PG, OHLCV pre-aggregated into `price_snapshots`),
PG is the right primary store. Don't add CH for one Tranche 1
backfill stream.

## Option D — Status quo (archive reads only, drop the CH integration)

**Sketch:** Ignore CH entirely. Both streams (SDEX and Soroban
AMM) go through the same archive-read Fargate task pattern.
Soroban AMM swap events are extracted by parsing
`SorobanTransactionMeta.events` from `LedgerCloseMeta` XDR, the
same way SDEX trades are extracted from `OperationResult`.

**Pros:**

- Maximum simplicity: one Fargate task pattern, one data source.
- Zero coupling to BE's CH plans.
- §5.6 timeline math for SDEX (~16 days pure compute for 57M
  ledgers) extends only modestly when adding the ~8.5M-ledger
  Soroban window (a few additional days).

**Cons:**

- Loses the "hours, not weeks" Tranche 1 promise from §5.6 entirely.
  Tranche 1 demo of Soroswap pair prices becomes a Tranche 2/3
  deliverable.
- Re-implements Soroban event parsing logic that BE already wrote
  and maintains in its `xdr-parser` crate — duplication of a
  non-trivial component.
- Forfeits the option value of the CH schema as a future analytical
  source.

**Verdict:** Acceptable fallback if Option B's "local CH instance
for backfill" overhead is unacceptable. Materially worse on
Tranche 1 deliverable timing.

## Comparison table

| Dimension                                   | Option A             | Option B            | Option C                         | Option D    |
| ------------------------------------------- | -------------------- | ------------------- | -------------------------------- | ----------- |
| BE infra dependency at runtime              | High                 | None                | Low (only initial sync)          | None        |
| Infra prices-api operates                   | None                 | Local CH (one-shot) | Local CH (ongoing)               | Nothing new |
| Tranche 1 fast-path viable?                 | Theoretically        | Yes                 | Yes                              | No          |
| Couples to a BE decision not yet made       | Yes (AWS CH deploy)  | No                  | Partially (replication contract) | No          |
| Engineering effort                          | Low (if BE delivers) | Medium              | High                             | Medium      |
| Forward-compatible with prices-api scale-up | Yes                  | Yes                 | Yes                              | Limited     |

## Open inputs for human review

1. Is BE willing to formalize the local-backfill-CH workflow as a
   "supported way for prices-api to seed historical data"? (Option
   B's premise.)
2. Does the prices-api team want to operate a one-shot CH instance
   (acceptable for a bounded backfill) vs avoiding the new engine
   entirely (Option D)?
3. Is there appetite to wait on BE's AWS-CH deployment timing
   before committing the prices-api §5.6 design? (Option A's
   gating decision.)

These belong in the S-note synthesis.
