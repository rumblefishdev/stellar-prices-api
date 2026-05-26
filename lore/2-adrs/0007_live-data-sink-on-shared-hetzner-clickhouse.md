---
id: '0007'
title: 'Live data sink on shared Hetzner ClickHouse, not Prices-owned RDS Postgres'
status: accepted
deciders: [okarcz]
related_tasks:
  ['0044', '0045', '0046', '0047', '0011', '0017', '0038', '0039', '0040']
related_adrs: ['0001', '0003', '0004', '0005', '0006']
tags:
  [
    architecture,
    infrastructure,
    clickhouse,
    hetzner,
    shared-infra,
    block-explorer,
    data-sink,
    live-ingestion,
  ]
links:
  - '../1-tasks/archive/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/README.md'
  - '../1-tasks/archive/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/S-refactor-recommendation.md'
  - '../1-tasks/archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md'
  - '../1-tasks/archive/0046_RESEARCH_empirical-prices-ch-storage-estimate-from-10k-ledgers/notes/G-empirical-storage-estimate.md'
  - '../1-tasks/backlog/0047_RESEARCH_cross-tenant-throughput-verification-on-shared-hetzner-ch.md'
  - '../../../soroban-block-explorer/docs/architecture/infrastructure/infrastructure-overview.md'
  - '../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md'
  - '../../../soroban-block-explorer/lore/2-adrs/0045_clickhouse-local-backfill-then-mirror-to-hetzner-via-freeze-rsync-attach.md'
history:
  - date: 2026-05-18
    status: proposed
    who: okarcz
    note: >
      Drafted to capture the architectural commitment that emerged
      from task 0044's seven-step research. Status remains
      `proposed` until task 0045's cross-team conversation with BE
      produces written commitments on the four agenda clusters
      (architecture buy-in, capacity/retention/backup, auth, money).
      Transitions to `accepted` after that conversation closes;
      reverts to `superseded` if BE rejects Option 1 in §3 and
      drives a different shape (most likely the Option 4 sidecar CH
      from task 0044's I-note).
  - date: 2026-05-20
    status: accepted
    who: okarcz
    note: >
      Transitioned proposed → accepted on the basis of task 0045's
      agreement record (notes/G-be-agreement-record.md): 10 yes /
      0 no / 3 TBD. Cluster A (architecture buy-in) all-yes locks
      in the separate-`prices`-database shape, the SNS fan-out,
      and the announcement-not-approval DDL norm. Cluster C
      (auth) all-yes locks in per-env mTLS certs, 1-year manual
      rotation, and CA-rotation revocation. Cluster B is mostly
      yes — hardware/cost (B4), Caddy headroom (B5), Borg target
      (B7), and daily RPO (B8) all accepted on the empirical basis
      task 0046 established. Cluster D money mechanism (D13) yes.
      Remaining items tracked as engineering follow-ups, not
      architectural unknowns:
      (a) Task 0047 (B6 throughput verification) is a hard
          engineering gate before the implementation tasks
          (0011/0038/0039/0040) can begin their rewrites. A RED
          outcome would supersede this ADR via Alternative 3
          (sidecar CH on the same Hetzner box).
      (b) B7 Borg cadence (daily vs. weekly) is a soft operational
          knob; daily is the recommendation pending BE
          confirmation.
      (c) D12 cost-share number is a soft commercial follow-up;
          the brief's ~1-2% pro-rata stance (per task 0046's
          empirical ~0.45 GB/yr footprint) is the proposal,
          re-validated once BE's `default.*` lands on the
          production box.
      The original "BE Hetzner CH reaches production" gate (BE
      tasks 0216 + 0227) is acknowledged still in flight on the
      BE side; the architectural commitment does not wait on the
      production cutover, but implementation work tied to that
      cutover (mTLS endpoint, Caddy address, cert issuance) is
      sequenced after BE 0227 lands.
---

# ADR 0007: Live data sink on shared Hetzner ClickHouse, not Prices-owned RDS Postgres

**Related:**

- [Task 0044](../1-tasks/active/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/README.md) — research that produced this ADR
- [Task 0044 synthesis](../1-tasks/active/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/S-refactor-recommendation.md) — detailed reasoning
- [Task 0045](../1-tasks/backlog/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy.md) — cross-team conversation that gates acceptance
- BE [ADR 0044](../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md) and [ADR 0045](../../../soroban-block-explorer/lore/2-adrs/0045_clickhouse-local-backfill-then-mirror-to-hetzner-via-freeze-rsync-attach.md)

---

## Context

The current prices-api design (general-overview §1.2, §2.1, §5.2)
commits to writing live OHLCV / trade rows into a Prices-owned RDS
PostgreSQL instance. The blocked tasks 0011 (CDK bootstrap), 0038
(Prices Ledger Processor Lambda), 0039 (periodic workers), and
0040 (API Gateway + read handlers) all assume that RDS.

Two things shifted since those tasks were drafted:

1. **BE committed to a Hetzner-hosted production ClickHouse** as
   the system-of-record data plane for the soroban-block-explorer
   (BE tasks 0216 + 0227; §5.6 of BE infrastructure-overview).
   That cluster is being stood up regardless of what prices-api
   does.
2. **Task 0009's cost-sharing framing extends naturally.** If BE
   is operating a production CH cluster, prices-api can write
   into it instead of provisioning a parallel RDS. The Galexie +
   S3 sharing already in §11.1 of the design doc gains a third
   shared component (the CH data plane).

Task 0044 spent seven research steps establishing whether the
refactor is technically sound, what the topology looks like, what
it costs, and what each blocked task needs to become. This ADR
captures the architectural conclusion.

---

## Decision

The prices-api live data sink is **BE's Hetzner-hosted ClickHouse
cluster**, not a Prices-owned RDS Postgres. Specifically:

1. **Storage plane.** All live OHLCV / current-prices / oracle /
   asset registry / backfill-progress data lives in a
   `prices` database inside BE's CH cluster, isolated from BE's
   `default.*` schema by ClickHouse's first-class multi-tenant
   primitives (database, user, quota, profile).

2. **Live-ingest path.** Unchanged from today's plan:
   S3 PutObject on BE's `stellar-ledger-data` bucket fans out to
   both BE and prices-api Lambdas via an SNS topic. Prices-api
   Lambda decodes XDR, extracts trades, writes OHLCV rows to
   `prices.price_ohlcv_*` over HTTPS-mTLS to Caddy:443.

3. **Schema engine.** `price_ohlcv` per-source rows on
   `ReplacingMergeTree(version)`, one row per
   `(timestamp, asset_id, quote_asset_id, granularity, source)`.
   Cross-source merge expressed at read time via `GROUP BY`.
   Per-granularity tables (`price_ohlcv_1m`, `_15m`, ..., `_1M`)
   so cleanup is `ALTER TABLE … DROP PARTITION`, not `DELETE`.

4. **Rollups.** Implemented as a chain of CH materialised views
   (`1m → 15m → 1h → 4h → 1d → 1w → 1M`). The OHLCV Rollup
   Lambda from the previous design is **eliminated**.

5. **Auth.** Mutual TLS between AWS-side Lambdas and Caddy:443,
   using BE's self-signed CA and BE's existing per-AWS-service
   client-cert issuance script. Prices-api cert + key live in
   AWS Secrets Manager (2 secrets per env).

6. **AWS-side simplification.** Prices-api Lambdas run **outside
   any VPC** (no Prices-api VPC, no NAT Gateway, no security
   groups for Lambda↔DB). Network path is public-internet
   outbound from Lambda; gating is mTLS at Caddy, not IP-based.

7. **Schema ownership boundary.** Prices-api owns `prices.*`
   schema, migrations, and DDL evolution unilaterally. Cross-
   database reads (`SELECT FROM default.*`) are reviewable by
   BE but not approval-gated; in practice they should be wrapped
   in named `prices.*` views to keep the breakage surface
   narrow.

**Status: accepted (2026-05-20).** Task 0045's agreement record
([G-be-agreement-record.md](../1-tasks/archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md))
landed BE acceptance across the four agenda clusters (10 yes / 0
no / 3 TBD; architectural shape locked via Cluster A all-yes).
Engineering follow-ups before implementation can begin:

- **Hard gate — task 0047** (cross-tenant throughput verification).
  A RED outcome (shared CH+Caddy cannot absorb combined load)
  supersedes this ADR via Alternative 3 (sidecar CH on the same
  Hetzner box). GREEN/YELLOW unblocks the rewrites of
  0011/0038/0039/0040.
- **Soft follow-up — B7 Borg cadence** (daily vs. weekly). Daily
  is the recommendation; resolvable in-line with BE.
- **Soft follow-up — D12 cost-share number.** Brief's ~1-2%
  pro-rata stance (per task 0046's empirical ~0.45 GB/yr
  footprint) is the opening proposal; finalised once BE's
  `default.*` lands on the production box and a measured fraction
  is computable.
- **Sequencing — BE 0216 + 0227.** The mTLS endpoint, Caddy
  address, and per-env cert issuance script are gated on BE's
  Ansible playbook landing. Architectural acceptance does not
  wait on that cutover, but implementation work that touches the
  production endpoint does.

---

## Rationale

Detailed reasoning lives in
[task 0044's synthesis note](../1-tasks/active/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/S-refactor-recommendation.md);
the short version:

- **Cheapest possible refactor surface.** The S3→Lambda half of
  the existing live-ingest plan is unchanged; only the storage
  half flips. ~4–5 person-weeks engineering total.
- **Costs less at any scale-up.** Steady-state cost is roughly a
  wash (-$14 to +$6 per env/mo depending on cost-share); at
  production scale the avoided RDS upgrades (`db.r6g.large` +
  Multi-AZ + read replica + RDS Proxy) save ~$6,936/year.
- **Strictly simpler operational surface.** One less production
  DB to operate; no Prices-api VPC; no NAT Gateway; one fewer
  Lambda (Rollup eliminated). Cert rotation (~2 hr/year) is the
  only new recurring task.
- **Builds on a commitment BE already made.** The Hetzner CH is
  funded and being stood up regardless. Joining as a second
  tenant uses CH's first-class multi-tenant primitives.
- **No data-loss path.** BE's indefinite S3 retention (BE ADR 0006) + Lambda async retry + DLQ provides structural durability
  end-to-end; no intermediate queue needed.

---

## Alternatives Considered

### Alternative 1: Status quo — Prices-owned RDS Postgres

**Description:** Execute the existing blocked tasks 0011 / 0038 /
0039 / 0040 as drafted, against a Prices-owned RDS.

**Pros:**

- Already designed; no redesign work.
- RDS PITR gives stronger backup RPO than Hetzner Borg daily.
- Lambda-in-VPC → RDS gives <1ms write latency vs. ~85ms over
  the public internet.

**Cons:**

- Adds ~$12-$550/mo per env depending on scale that the Hetzner
  shape avoids.
- Operational surface: Prices-api owns a production Postgres.
- Misses the cost-sharing opportunity BE's Hetzner cluster
  presents.

**Decision:** REJECTED at the architectural level; deferred at
the execution level until the gating events for this ADR clear.

### Alternative 2: Shared tables with `tenant` discriminator inside BE's CH

**Description:** BE's `init.sql` grows tenant columns; both writers
insert into the same tables. Cross-tenant aggregations via
`GROUP BY tenant`.

**Pros:**

- Single DDL surface.
- Cross-tenant analytics trivial.

**Cons:**

- BE's event-shaped rows and prices-api's candle-shaped rows
  don't overlap structurally. Forcing them into shared tables
  needs ~80% nullable columns per row — pathological in a
  columnar store.
- Schema evolution couples to BE's repo.

**Decision:** REJECTED. See task 0044 I-note §1.2 for full
analysis.

### Alternative 3: Sidecar `clickhouse-server` instance on the same Hetzner box

**Description:** Two CH containers on BE's Hetzner box, separate
ports, separate data volumes. Caddy routes to each.

**Pros:**

- True isolation between tenants.

**Cons:**

- Doubles operational surface (two CH versions to upgrade in
  lockstep, two backup cron jobs, two `users.d` directories).
- Asks BE for non-trivial Ansible work (touches BE task 0227
  scope).
- Partially defeats the cost-sharing premise.

**Decision:** REJECTED as primary; retained as the **fallback**
if BE rejects the separate-database shape in §3.

---

## Consequences

### Positive

- **Lower per-env baseline cost** and substantially lower at-scale
  cost (no RDS Multi-AZ / read replica / Proxy).
- **One less production DB** for prices-api to operate.
- **No Prices-api VPC / NAT / SG.** Networking simplifies.
- **One fewer Lambda** (Rollup eliminated; rollups become CH
  MV chain).
- **Replay story strengthened.** BE's indefinite S3 retention
  means prices-api can replay arbitrary windows by re-firing
  S3 events.

### Negative

- **Recovery RPO is daily Borg, not RDS PITR.** Acceptable for
  OHLCV but a real demotion from the RDS plan.
- **mTLS cert lifecycle added** (~2 hr/year of operator work +
  one NotAfter alarm per env).
- **Cross-cloud network path.** Lambda → Hetzner is ~80–130 ms
  RTT vs. <1 ms in-VPC. Mitigated by warm-container connection
  reuse and per-ledger batch writes.
- **Cross-team coordination cost.** Schema-ownership, capacity
  sizing, cert issuance, and cost-share all require BE buy-in.
- **`ReplacingMergeTree` is eventually consistent.** API read
  path must use `FINAL` or explicit `argMin/argMax + GROUP BY`
  re-aggregation; verified workable in task 0044 §2 but adds
  read-time SQL complexity.

### Implementation impact (recap from task 0044 synthesis §3)

| Existing task                             | Status after this ADR is accepted                                                     |
| ----------------------------------------- | ------------------------------------------------------------------------------------- |
| **0011** — CDK bootstrap                  | Major rewrite. No RDS, no VPC; Secrets Manager mTLS material.                         |
| **0017** — Local CH for backfill          | Unchanged (backfill is workstation-local; refactor only affects the live cloud sink). |
| **0038** — Prices Ledger Processor Lambda | Major rewrite. sqlx → `clickhouse` crate; UPSERT → ReplacingMergeTree INSERT.         |
| **0039** — Periodic workers Lambda set    | Major rewrite. OHLCV Rollup Lambda deleted; others retargeted.                        |
| **0040** — API Gateway + read handlers    | Moderate rewrite. Read handlers retargeted; endpoint contracts unchanged.             |

---

## References

- [Stellar developer docs — Galexie](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie)
- [Stellar developer docs — Captive Core](https://developers.stellar.org/docs/data/indexers/build-your-own/ingest-sdk/developer_guide/ledgerbackends/captivecore)
- [SEP-0054 (ledger data lake naming)](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0054.md)
- [ClickHouse `AggregatingMergeTree` / `ReplacingMergeTree` docs](https://clickhouse.com/docs/en/engines/table-engines/mergetree-family/)
