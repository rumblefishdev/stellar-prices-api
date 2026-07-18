---
id: "0001"
title: "Stream 1 Soroban AMM historical backfill is sourced from BE's ClickHouse soroban_events (local instance)"
status: superseded
deciders: [okarcz]
related_tasks: ["0015", "0017", "0018", "0053", "0060"]
related_adrs: ["0002"]
tags: [architecture, backfill, clickhouse, block-explorer, soroban, amm, stream-1]
links:
  - "../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md"
  - "../../../soroban-block-explorer/lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md"
  - "../../docs/prices-api-general-overview.md"
  - "../../docs/database-schema/clickhouse-prod-schema.sql"
history:
  - date: 2026-05-12
    status: proposed
    who: okarcz
    note: "Drafted during task 0015 closure. Reverses §5.6 Stream 1's PG-RDS assumption."
  - date: 2026-05-12
    status: accepted
    who: okarcz
    note: >
      Accepted post the five-question resolution captured in
      0015 notes/S-redesigned-backfill-recommendation.md §Resolutions.
      BE side: fmazur confirms backfill-runner --target=clickhouse is
      a supported workflow for prices-api consumption. prices-api side:
      local CH runs on dev laptop (okarcz) with shared access; the
      stellar-xdr parser crate (to be built) will handle ScVal decoding.
  - date: 2026-07-01
    status: superseded
    who: claude
    note: >
      **Superseded in place by the combined single-pass backfill** (see the
      Amendment at the top of the body). The `soroban_events`-sourced model
      below — BE `backfill-runner --target=clickhouse` populates a local
      `soroban_events` table that a separate `soroban-amm-backfill` CLI
      queries — is replaced by extracting SDEX + Soroban AMM swaps in a
      **single download/parse pass** of each `LedgerCloseMeta`. The pivot was
      locked with the operator in task 0060 (2026-06-11) and built + measured
      there; implementation is the rescoped task 0053. No separate replacing
      ADR was created — the superseding decision is recorded as the Amendment
      here (hence no `by:` ADR). Retiring the `soroban_events` role also
      retires task 0017's reason to exist.
  - date: 2026-07-01
    status: superseded
    who: okarcz
    by: "0009"
    note: >
      **Push-model clause superseded by ADR 0009.** The amendment's "local
      `prices.*` mirror → completion push to Hetzner → tear down local"
      (Model A / task 0028) is replaced by writing the backfill **directly to
      Hetzner over the 0052 mTLS client** (Model B), mirroring BE's
      production `backfill-runner --target clickhouse`. The combined
      single-pass extraction, download-once, forward discovery, and dual
      `backfill_progress` rows are UNCHANGED — only the transport to Hetzner
      changes. See ADR 0009.
---

# ADR 0001: Stream 1 Soroban AMM historical backfill is sourced from BE's ClickHouse `soroban_events` (local instance)

**Related:**

- [Task 0015: Redefine backfill plan and define price-calculation use of BE's ClickHouse full-content soroban_events](../1-tasks/archive/0015_RESEARCH_redefine-backfill-with-be-clickhouse-events/README.md) — the research that produced this decision
- [Task 0017: Local ClickHouse instance setup and access for prices-api Tranche 1 backfill](../1-tasks/backlog/0017_FEATURE_local-clickhouse-for-prices-backfill.md) — operational landing
- [Task 0018: Sample-decode per-AMM swap event shapes (Soroswap, Aquarius, Phoenix)](../1-tasks/backlog/0018_RESEARCH_decode-per-amm-swap-event-shapes.md) — pins the extraction logic
- [BE ADR 0044: ClickHouse pilot — parallel store mirroring Postgres schema, with full-content soroban_events](../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md) — the upstream enabling decision
- [BE ADR 0033: soroban_events → soroban_events_appearances (read-time event detail from S3)](../../../soroban-block-explorer/lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md) — the PG compromise this ADR routes around

---

## Amendment (2026-07-01) — SUPERSEDED: combined single-pass replaces `soroban_events` sourcing

> **Status: superseded in place.** The Decision below (source the Soroban AMM
> backfill from a local `soroban_events` table populated by BE's
> `backfill-runner --target=clickhouse`, queried by a separate
> `soroban-amm-backfill` CLI) is **no longer in force.** The original text is
> retained for history. The current decision is this Amendment.

**New decision — combined single-pass historical backfill.** The Soroban AMM
history is extracted in the **same download/parse pass as SDEX**, directly
from each `LedgerCloseMeta`, with **no intermediate `soroban_events` table
and no dependency on BE's `backfill-runner`**. Locked with the operator in
**task 0060** (2026-06-11), built and measured there (Soroswap + Aquarius
extractors, one-parse SDEX + AMM + oracle extraction, the `prices-clickhouse`
schema crate, a 100k-ledger sizing run). Implementation is the rescoped
[task 0053](../1-tasks/backlog/0053_FEATURE_soroban-amm-backfill-cli-stream-1-impl.md).

Why the pivot:

1. **Download is the bottleneck, not parsing** (0060: download-bound,
   ~37 ms/ledger). SDEX trades and Soroban events live in the *same*
   `LedgerCloseMeta`, so a second pass to build `soroban_events` would
   re-download the entire Soroban era. Single-pass extracts both from one
   download → each ledger downloaded exactly once.
2. **No `soroban_events` store** (was ~100–150 GB) and **no cross-team
   `backfill-runner` dependency**. Retires task 0017's `soroban_events` role;
   the local CH itself already exists (docker-compose + the crate).
3. **Backfill == live by construction** — the decode/bucket path is shared
   with the live Ledger Processor (0038), so backfilled candles are
   byte-identical to live ones.

Shape of the run (details in task 0053):

- **Soroban backfill** over `[activation, tip]` extracts **SDEX + AMM**
  (+ oracle) in one pass; **SDEX backfill** over `[1, activation)` extracts
  **SDEX only**. Disjoint ranges → union `[1, tip]`, no double download.
- **Forward oldest→newest** decode of the Soroban range makes every pool's
  factory-create precede its swaps → **complete pool discovery with no
  external registry seed** (downgrades task 0069 to an optimization). Guard:
  no swap may be decoded for an unregistered pool.
- **Both** `backfill_progress` rows are updated by the combined run
  (`soroban_amm`→completed **and** `sdex_archive`→`current=activation`) so
  `GET /backfill/status` (overview §3.5/§4.5) stays truthful.

What is **unchanged** from the original decision: it is still a **local
workstation one-shot** feeding the `prices.*` mirror, then a completion push
to the Hetzner cloud CH; the local instance is torn down afterwards; and
live go-forward ingestion never depended on this path (point 4 below).

---

## Context

The prices-api technical design (`docs/prices-api-general-overview.md`
§5.6, written before BE's CH work) assumed BE's RDS PostgreSQL would
expose decoded Soroban event topics+data as JSONB in a
`soroban_events` table, enabling a "fast Tranche 1" historical
backfill (hours, not weeks) for Soroban AMM swaps from Soroswap,
Aquarius, and Phoenix.

That assumption was wrong on two counts as of mid-2026:

1. **No full-content `soroban_events` in BE PG.** BE
   [ADR 0033](../../../soroban-block-explorer/lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md)
   folded the table into appearances-only (pointer + signature
   topic; full content fetched from public S3 archive at read
   time). This was a Postgres-shaped compromise driven by the row
   width of storing `topics_xdr` + `data_xdr` per event.

2. **No decoded JSONB anywhere in BE PG.** Where appearances exist,
   the payload references XDR-encoded archive bytes, not parsed
   JSON.

Between when prices-api's design was written and 2026-05-12, BE
declared a parallel ClickHouse store
([BE ADR 0044](../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md))
whose copy of `soroban_events` deliberately reverses ADR 0033's
folding: per-event row, full `topics_xdr` + `data_xdr` inlined as
ZSTD-coded `String`, with a hoisted
`signature LowCardinality(Nullable(String))` first-topic Symbol
column for cheap `WHERE signature = 'swap'` filtering. BE tasks
0204 (schema landed), 0205 (`backfill-runner --target=clickhouse`
CLI flag landed), and active 0206 (real CH writer replacing the
stub) have moved the ClickHouse copy past the "read-empty pilot"
of ADR 0044's §6 — the production schema is declared in
[`docs/database-schema/clickhouse-prod-schema.sql`](../../docs/database-schema/clickhouse-prod-schema.sql).

What is **still true** as of 2026-05-12: BE has **no
AWS-deployed ClickHouse cluster**. The production CH schema is
populated by BE's local backfill runner against local Docker CH.
Any prices-api plan that assumes "cross-account live read against
BE's CH" is betting on infrastructure that does not exist yet.

The full research record is in task 0015's notes
(`R-be-clickhouse-schema-and-status.md`,
`G-ch-tables-for-price-calculation.md`,
`I-integration-options.md`,
`S-redesigned-backfill-recommendation.md`); this ADR records the
resulting architectural decision.

---

## Decision

Prices-api's Soroban AMM historical backfill (Stream 1 of §5.6)
is sourced from a **locally-run ClickHouse instance populated by
BE's `backfill-runner --target=clickhouse`** (BE task 0205 CLI),
not from BE's production PostgreSQL.

Concretely:

1. A **local ClickHouse instance runs on a developer laptop**
   (okarcz's), populated by running BE's backfill runner against
   the Soroban-activation-onward ledger range (~ledger 48.5M to
   tip, ~8.5M ledgers). Operational details (disk sizing, access
   mechanism for the prices-api Tranche 1 consumer, tear-down
   trigger) land in [task 0017](../1-tasks/backlog/0017_FEATURE_local-clickhouse-for-prices-backfill.md).

2. The prices-api Tranche 1 consumer queries the local CH
   `soroban_events` table with the contract-id + signature
   predicate documented in
   `0015 notes/G-ch-tables-for-price-calculation.md` Requirement 1,
   JOINing `ledgers.closed_at` for wall-clock time, and decoding
   `topics_xdr` / `data_xdr` (ScVal XDR) using a `stellar-xdr`
   parser crate that will be built and ready for prices-api use.

3. Trade ticks extracted from the swap events are bucketed into
   OHLCV granularities and persisted into prices-api's PostgreSQL
   `price_snapshots` historical partitions, identical in shape to
   what §5.2 / §5.5 produce for live ingestion. The historical
   path is a one-shot job; once it completes, the local CH instance
   is torn down.

4. **Live go-forward Soroban AMM ingestion does NOT depend on CH.**
   prices-api's own Ledger Processor Lambda (§5.2) parses Soroban
   events from new `LedgerCloseMeta` directly as they arrive via
   Galexie → S3 → EventBridge. The CH dependency is bounded to the
   historical backfill window.

5. **Stream 2 (SDEX archive reads) is governed by a separate ADR.**
   See [ADR 0002](./0002_stream2-sdex-archive-backfill-independent-of-be.md):
   Stream 2 is a fully prices-api-owned archive-read Fargate task
   with no BE runtime/data coupling; only the BE-authored
   `stellar-xdr` parser is consumed as a library crate.

---

## Rationale

### Option B from the research, not Option A or C

[Task 0015's I-note](../1-tasks/archive/0015_RESEARCH_redefine-backfill-with-be-clickhouse-events/notes/I-integration-options.md)
sketched four consumption options. The chosen Option B threads the
needle between three pressures:

- **Recover the §5.6 fast-path promise.** The original
  hours-not-weeks Tranche 1 timing depended on querying a database
  rather than reading 8.5M ledgers from archive. The CH copy of
  `soroban_events` restores that path.
- **Avoid runtime coupling to BE infrastructure that does not
  exist.** A cross-account live query (Option A) presupposes an
  AWS-deployed BE CH cluster. As of 2026-05-12 there is none, and
  ADR 0044's §6 explicitly defers that decision to a follow-up
  ADR gated on first measurements. Planning prices-api around it
  is planning around vapor.
- **Don't add ClickHouse as a long-running engine alongside the
  prices-api PostgreSQL.** Option C (prices-owned CH replica) is
  over-scoped for a single backfill stream. The §10 cost estimates
  and §6 sizing don't accommodate it.

Option B uses CH only where its value is concentrated — historical
event-content queries during a bounded window — and accepts the
cost of building both a CH consumer AND a live-network Soroban
event parser.

### Local instance, not "Block Explorer's database"

Reading from a locally-run CH instance (populated by BE's existing
CLI) avoids the operational and cross-team trust problems of
treating BE's running database as a prices-api dependency.
prices-api's §11 lists "Block Explorer's database" as **not
shared**; this ADR keeps that guarantee intact. The
prices-api-runnable CH instance is prices-api's own
short-lived asset, not a coupling to BE's runtime.

### Dev-laptop instance, not Fargate/EC2

User-confirmed (Q2 resolution in 0015 S-note): the developer
laptop is the right target because (a) BE's
`backfill-runner --target=clickhouse` is the same tool BE uses
internally and is laptop-tested in BE's task 0117 benchmark,
(b) the disk requirement (hundreds of GB pre-compression cut
significantly by ZSTD) fits a workstation but would need
provisioning math on Fargate, and (c) the backfill is a one-shot
event, not an ongoing workload — paying for cloud compute for
something that runs once is over-investment.

### Not waiting on BE's pilot success criteria

BE ADR 0044's Q6 (pilot PASS/FAIL criteria) remains open and is
gated on first measurements. This ADR deliberately does **not**
wait for that resolution. The prices-api consumer treats BE's CH
as a snapshot tool, not a pilot subject. Whatever BE concludes
about migrating PG → CH does not change prices-api's Tranche 1
plan.

---

## Alternatives Considered

### Alternative 1: Cross-account live query against BE's production ClickHouse

**Description:** Open a CH client connection from prices-api
Lambda to a BE-deployed ClickHouse cluster in the shared AWS
sub-account; run swap-event queries live during Tranche 1
backfill AND as a continuous read source for go-forward AMM data.

**Pros:**

- No CH operational surface area for prices-api.
- Single source of truth, no replication lag.
- Aligns with §11's infrastructure-sharing theme.

**Cons:**

- BE has no AWS-deployed CH cluster as of 2026-05-12, and ADR
  0044 §6 defers that decision to a still-open follow-up. Option A
  depends on infrastructure that may or may not materialize.
- Introduces hard runtime coupling — every Lambda invocation
  touching Soroban AMM data depends on BE CH cluster availability.
- BE has not committed to a public/SLA'd read interface for CH.

**Decision:** REJECTED — depends on unfounded infra.

### Alternative 2: Prices-api operates its own ClickHouse replica of BE data

**Description:** Stand up a prices-api-owned CH cluster (Fargate
or managed CH-as-a-service); receive a continuous data feed from
BE; serve all prices analytics from it.

**Pros:**

- Full data ownership; no BE runtime dependency.
- Could become the analytical store for prices-api itself, not
  just a backfill source.

**Cons:**

- Adds CH as a long-running second engine alongside PG, expanding
  prices-api's operational scope materially.
- §10 cost estimates do not cover this.
- Replication pipeline BE → prices CH is its own project.
- Over-scoped for a single Tranche 1 backfill stream.

**Decision:** REJECTED — over-scoped vs the actual need.

### Alternative 3: Status quo — archive reads only, drop the CH integration

**Description:** Both streams (SDEX and Soroban AMM) go through
the same archive-read Fargate task pattern. Soroban AMM swap
events are extracted by parsing `SorobanTransactionMeta.events`
from `LedgerCloseMeta` XDR, same as SDEX `offersClaimed[]`.

**Pros:**

- Maximum simplicity: one Fargate task pattern.
- Zero coupling to BE's CH plans.

**Cons:**

- Loses the §5.6 "Tranche 1 fast path" promise entirely.
  Tranche 1 demo of Soroswap pair prices becomes Tranche 2/3 work.
- Re-implements Soroban event parsing logic that BE already wrote
  and maintains.

**Decision:** REJECTED — materially worse on Tranche 1 timing.

---

## Consequences

### Positive

- **Tranche 1 hours-not-weeks promise preserved** for Soroban
  AMM history.
- **No runtime BE coupling.** prices-api's live path depends only
  on prices-api-owned infrastructure. The CH dependency is
  time-boxed to the backfill window.
- **Reuses BE's existing tooling.** BE's `backfill-runner`
  (task 0205) and CH schema (task 0204) are consumed as-is — no
  parallel implementation.
- **First local ADR in prices-api.** Establishes the precedent
  for prices-api-side architectural records, links cleanly to BE
  ADRs without duplicating them.

### Negative

- **Two ingestion paths to build:** the local-CH backfill consumer
  AND the live-network Soroban event parser. Higher engineering
  cost up front than a single live read path.
- **Coupling to BE task 0206 quality.** The backfill runner's
  CH writer must be correctness-stable enough for prices-api to
  consume. Mitigation: BE's task 0117 (local backfill benchmark)
  is the proxy signal.
- **Decision tied to a developer laptop.** The Tranche 1 backfill
  runs on okarcz's machine; if that machine is unavailable the
  backfill is gated. Mitigation: documented in task 0017 with a
  Fargate fallback path if the laptop is impractical.
- **Schema drift risk during the backfill window.** Any change to
  BE's CH schema (driven by post-0206 ADRs) needs the prices-api
  consumer updated. Bounded by the backfill duration.

---

## References

- [BE task 0204](../../../soroban-block-explorer/lore/1-tasks/archive/0204_FEATURE_clickhouse-pilot-crate-docker-schema/) — CH crate + schema landing
- [BE task 0205](../../../soroban-block-explorer/lore/1-tasks/archive/0205_FEATURE_backfill-runner-clickhouse-target-flag.md) — `--target=clickhouse` CLI flag
- [BE task 0206](../../../soroban-block-explorer/lore/1-tasks/active/0206_FEATURE_clickhouse-persist-real-inserts/README.md) — real CH writer (active)
- [ClickHouse production schema](../../docs/database-schema/clickhouse-prod-schema.sql) — canonical DDL
- [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) — for related commit format
