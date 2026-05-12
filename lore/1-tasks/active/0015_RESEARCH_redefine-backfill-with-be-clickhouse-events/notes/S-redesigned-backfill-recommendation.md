---
title: "Synthesis — redesigned §5.6 backfill and BE-CH-derived token price calculation"
type: synthesis
status: mature
spawned_from: ../README.md
spawns: []
tags: [synthesis, recommendation, backfill, clickhouse, price-calculation]
links:
  - "./R-be-clickhouse-schema-and-status.md"
  - "./G-ch-tables-for-price-calculation.md"
  - "./I-integration-options.md"
history:
  - date: 2026-05-12
    status: mature
    who: okarcz
    note: "Final recommendation; open questions and spawned follow-ups listed."
---

# Synthesis — redesigned §5.6 backfill + token price calculation

## TL;DR recommendation

**Adopt Option B from the I-note** — a one-time ETL pull from a
locally-run BE ClickHouse instance for the Soroban AMM historical
backfill (Tranche 1), with prices-api owning live go-forward Soroban
event ingestion through its own Ledger Processor Lambda. Keep
Stream 2 (SDEX archive reads) **unchanged** — CH does not save us
from archive reads for SDEX, but it can pre-filter the ledger set
modestly (see G-note Requirement 2).

This preserves the §5.6 "Tranche 1 fast path" promise without
introducing runtime coupling to BE infrastructure that does not yet
exist (BE has no AWS-deployed CH cluster as of 2026-05-12 —
production CH lives only as the target of BE's local backfill
runner per task 0206).

## What changes in §5.6 of `docs/prices-api-general-overview.md`

### Stream 1 — Soroban AMM (Tranche 1, fast)

**Original**: query BE RDS PostgreSQL `soroban_events` table for
decoded JSONB topics/data.

**Wrong on two counts** (per task 0009 + task 0010 + this task):

1. BE RDS PG does not store full event content — only an appearance
   index (`soroban_events_appearances`, ADR 0033) pointing into the
   S3 archive.
2. The "decoded JSONB" expectation was never realized in BE PG —
   even where ADR 0033's appearances exist, the payload is XDR
   in archive, not JSONB in DB.

**Refactored**: query a locally-run BE ClickHouse instance,
populated by running BE's `backfill-runner --target=clickhouse`
(BE task 0205, ships in BE today). The CH `soroban_events` table
holds full per-event rows with `topics_xdr` + `data_xdr` as ZSTD-
coded XDR bytes and a hoisted `signature LowCardinality(Nullable(String))`
first-topic Symbol column.

Prices-api Tranche-1 backfill consumer:

1. Stands up a local CH instance (Docker compose, mirroring BE's
   compose service from task 0204).
2. Runs BE's `backfill-runner --target=clickhouse` for the
   Soroban-activation-onward ledger range (~ledger 48.5M → tip,
   ~8.5M ledgers).
3. Queries the local CH per [G-note Requirement 1](./G-ch-tables-for-price-calculation.md#requirement-1--soroban-amm-swaps-stream-1-of-56):
   filter `WHERE contract_id IN (Soroswap, Aquarius, Phoenix) AND
   signature = 'swap'`; JOIN `ledgers` for `closed_at`; decode
   `topics_xdr`/`data_xdr` with the `stellar-xdr` crate on the
   prices-api side.
4. Bucket trade ticks into OHLCV; write into prices-api
   PG `price_snapshots` historical partitions.
5. Tear down the local CH instance once backfill is done.

Tranche 1 timeline expectation: **hours, not weeks** — preserved.

### Stream 2 — SDEX (archive reads, slow, runs through Tranche 3+)

**Unchanged** in flow: ECS Fargate task reads `LedgerCloseMeta`
from public archives, extracts `offersClaimed[]` from
`OperationResult`, writes OHLCV.

**Modest CH-side improvement available**: pre-filter ledger ranges
to those containing trade-shaped operations using CH
`operations_appearances` (G-note Requirement 2). This trims the
archive-read volume; concrete trim ratio requires measurement on
real data. Treat as optimization, not architectural change.

### Live go-forward ingestion

Prices-api's own Ledger Processor Lambda handles **both** SDEX
results and Soroban AMM events from new ledgers in real time
(seconds after Galexie writes them to the shared S3 bucket). No
CH involvement in the live path. This matches §5.2 of the design
doc as written.

## Why Option B vs the alternatives

- **Option A (cross-account live CH read):** depends on a BE AWS
  CH deployment that does not exist. Planning prices-api around it
  is betting on undelivered BE infra.
- **Option C (prices-owned CH replica):** over-scoped for a single
  Tranche 1 backfill stream. Don't add CH as a long-running engine
  alongside PG.
- **Option D (archive reads only):** technically works but loses
  the Tranche 1 fast path entirely — Soroswap Tranche 1 demo
  becomes Tranche 2/3 work.

Option B threads the needle: get the Tranche 1 fast path, avoid
runtime BE coupling, defer the long-term-CH question to the future
when BE has actually deployed AWS CH and the prices-api workload
might genuinely benefit from columnar analytics.

## Token price calculation — definitive answer to the user's question

Given the refactored Stream 1, **prices-api calculates token prices
from BE's ClickHouse schema as follows**:

For **Soroban AMM swaps** (Soroswap, Aquarius, Phoenix):

```
soroban_events
    .filter(contract_id IN known_AMM_ids, signature = 'swap')
    .join(ledgers ON sequence)
    → fetch topics_xdr + data_xdr + closed_at
    → decode ScVal → (token_in, token_out, amount_in, amount_out)
    → tick price = amount_out / amount_in
    → bucket by closed_at → OHLCV
```

For **classic Stellar liquidity pool prices** (if needed):

```
liquidity_pool_snapshots
    .filter(pool_id = ?, ledger_sequence <= ?)
    .join(ledgers ON sequence)
    → instant price = reserve_b / reserve_a   (constant-product)
```

For **SDEX trades** (Tranche 2/3): CH `operations_appearances`
pre-filter → archive `LedgerCloseMeta` read → `offersClaimed[]`
extraction. (Same trade-extraction logic as the existing §5.6 plan;
the only addition is the CH pre-filter step.)

For **asset identity** (across all streams): CH `assets` 4-tuple
`(asset_type, asset_code, issuer_id, contract_id)` is the canonical
key. Disambiguates USDC variants, SAC vs native-Soroban USDC, etc.

For **contract labelling** (display only): prices-api keeps a local
`Vec<(StrKey, Int64)>` registry of well-known AMM contracts —
surrogate IDs are deterministic `cityhash64(StrKey)`, computable in
Rust at compile time using `cityhash-rs::cityhash_102_128` lower
64 bits (per R-note). No JOIN needed at query time for known
contracts; `soroban_contracts` JOIN reserved for unknown-contract
display.

For **wall-clock time**: every CH query JOINs `ledgers.closed_at`.
ADR 0044 §4b dropped `created_at` from fact tables, making this
mandatory. If hot, prices-api can stand up a CH Dictionary on
`ledgers` for RAM-resident lookups.

## Open questions for human review

1. **Is BE willing to formalize "run our backfill runner with
   --target=clickhouse" as a supported workflow for prices-api?**
   This is the Option B premise. The CLI flag ships today (BE task
   0205), but cross-team "supported use" is a softer ask than a
   query API. Worth raising at the next BE/prices sync.
2. **Where does the one-shot local CH instance run?** Options:
   prices-api developer laptops (fastest to iterate, slowest to
   complete a full 8.5M-ledger backfill); a Fargate spot task with
   attached EBS (faster, costs ~$X for the backfill window); a
   one-shot EC2 spot instance. Decision is gated on actual
   benchmark numbers — BE has those in their task 0117
   (local-backfill-benchmark).
3. **Should the CH pre-filter for SDEX (G-note Requirement 2) be
   implemented in Tranche 1 or deferred?** If BE's CH is available
   at all (Option B brings it up locally anyway), pre-filtering
   SDEX ledger ranges is a low-effort optimization. If we go with
   strict Option D for SDEX, no.
4. **What ScVal decoding library does prices-api use?** Direct
   `stellar-xdr` crate parsing is the obvious answer, but BE's
   `xdr-parser` crate already does extraction work that may be
   reusable. Decision lives in task 0012's scope (Prices-owned
   Fargate backfill design).
5. **Do we need an ADR for this?** The redesign reverses a
   pre-implementation design assumption in `docs/prices-api-general-
   overview.md`. Per the project's emerging ADR convention,
   anything that overturns a design-doc commitment merits an ADR.
   Recommended: spawn `lore/2-adrs/0001_…` (or wherever the local
   ADR numbering picks up) capturing the Stream 1 decision.

## Folded-in resolutions from related tasks

### Task 0010 (verify BE soroban_events_appearances schema)

**Status:** answered → supersede with 0015.

The original verification question was "do decoded Soroban event
topics+data exist anywhere in BE's database?" The answer as of
2026-05-12 is:

- **PostgreSQL: No.** Only `soroban_events_appearances`
  (pointer-only, ADR 0033). PG-side this remains unchanged.
- **ClickHouse: Yes.** Full per-event row with `topics_xdr` +
  `data_xdr` as ZSTD-coded XDR bytes plus a hoisted `signature`
  Symbol column (R-note §"Critical column shape: soroban_events").

That definitive answer collapses 0010's open question, so 0010 is
superseded by 0015. The acceptance criterion "spawn a follow-up to
revise §5.6" from 0010 is folded into this S-note's recommendation
and the spawned follow-ups below.

## Spawned follow-up tasks

Per `/lore-framework-tasks` ("Never leave future work as prose
only"), the items below become backlog tasks:

| Slot | Title | Notes |
|------|-------|-------|
| 0016 | Update `docs/prices-api-general-overview.md` §5.6 / §2.3 / §11 to reflect Stream 1 CH-sourced reality | Fold into existing backlog task 0013 if scope is compatible; otherwise spawn new. |
| 0017 | Design one-shot local CH instance for Tranche 1 backfill | Disk sizing, where it runs (laptop / Fargate / EC2 spot), tear-down trigger. |
| 0018 | Sample-decode real Soroswap / Aquarius / Phoenix swap events | Pin down per-AMM topic + data shape. Low-risk spike per G-note open spike section. |
| 0019 | Write ADR for Stream 1 CH-sourced backfill | Captures the §5.6 reversal as a first-class architectural decision (open question 5 above). |

## Coordination notes

- This S-note's recommendation should be reviewed alongside BE's
  task 0206 progress. If 0206's writer correctness has issues at
  the time prices-api wants to pull a backfill, the local CH
  approach is gated on 0206 quality. BE's task 0117 (local
  backfill benchmark) is the proxy for "is the writer healthy
  enough to consume."
- Open question 1 (BE support contract for backfill-runner as
  prices-api workflow) is the human-input gate before any
  follow-up task in this slate can start in earnest.
