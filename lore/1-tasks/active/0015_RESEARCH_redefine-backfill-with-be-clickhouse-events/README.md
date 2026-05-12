---
id: "0015"
title: "Redefine backfill plan and define price-calculation use of BE's ClickHouse full-content soroban_events"
type: RESEARCH
status: active
related_adr: []
related_tasks: ["0009", "0010", "0012", "0013", "0014"]
tags: [priority-high, effort-medium, infra, block-explorer, schema, clickhouse, backfill, price-calculation]
links:
  - "../../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md"
  - "../../../../soroban-block-explorer/lore/1-tasks/active/0206_FEATURE_clickhouse-persist-real-inserts/README.md"
  - "../../../../soroban-block-explorer/lore/1-tasks/archive/0204_FEATURE_clickhouse-pilot-crate-docker-schema/"
  - "../../../../soroban-block-explorer/lore/1-tasks/archive/0205_FEATURE_backfill-runner-clickhouse-target-flag.md"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-05-12
    status: active
    who: okarcz
    note: >
      Created in response to BE's ClickHouse graduation past the
      read-empty pilot (BE ADR 0044 + archived tasks 0204/0205 + active
      task 0206 wiring real writes; production schema declared in
      /home/oski/Projects/stellar/clickhouse-prod-schema.sql).
      Supersedes task 0010 — that task's "do decoded Soroban events live
      anywhere in BE's DB?" question is now answered definitively (yes,
      in CH `soroban_events` with full topics_xdr + data_xdr per row).
      This task replaces it with the broader refactor: redesign §5.6
      Stream 1 of `docs/prices-api-general-overview.md` against the new
      reality, and produce a table-by-table mapping from the CH schema
      onto the prices-api's token-price-calculation requirements.
---

# Redefine backfill plan and define price-calculation use of BE's ClickHouse full-content soroban_events

## Summary

BE's data architecture shifted between when the prices-api design doc was
written and today. The PG-side compromise that drove §5.6's "Stream 1
queries BE RDS `soroban_events` for decoded JSONB" assumption (which was
already known to be wrong against BE reality — task 0009 row 8, task
0010) is now resolved on the BE side by a **different table in a
different engine**: ClickHouse holds full-content `soroban_events`
(per-event row, `topics_xdr` + `data_xdr` inlined, ZSTD-coded,
partitioned by `ledger_sequence`). The CH copy is no longer the
"read-empty pilot" of ADR 0044 — it is being populated by BE's active
task 0206 against the production schema at
`/home/oski/Projects/stellar/clickhouse-prod-schema.sql`.

This research task does two things:

1. **Refactor §5.6 backfill.** Redesign Stream 1 (Soroban AMM) against
   CH-as-the-source instead of PG-as-the-source. Decide whether to
   collapse it back into one stream with SDEX or keep the two-stream
   shape; decide on the consumption pattern (cross-account live read,
   ETL pull, replica, etc.).
2. **Define schema-to-price-calculation mapping.** Concrete
   table-by-table mapping showing which CH tables answer which
   price-calc requirement (swap event extraction, AMM reserve-based
   instant price, asset identity, ledger-time resolution, contract
   labelling). Example queries included.

## Status: Active

**Current state:** Research notes drafted in this session. Synthesis
recommendation produced. Open questions for human input listed in
`notes/S-redesigned-backfill-recommendation.md`.

## Context

What changed since task 0009 closed (2026-05-11):

- **BE ADR 0044** (proposed 2026-05-08) declared a parallel CH store
  with a full-content `soroban_events` table — but the ADR was scoped
  as read-empty pilot, local-only, gated on a follow-up ADR before
  cross-project consumption.
- **BE task 0204** (archived) stood up `crates/db-clickhouse` with the
  schema + Docker service + idempotent `init.sql`.
- **BE task 0205** (archived) added a `--target=clickhouse` flag to
  the backfill runner with stub writes.
- **BE task 0206** (active 2026-05-11) is replacing the stub
  `persist_ledger_clickhouse` with a real writer that populates all
  17 tables + the `transaction_hash_dict` Dictionary, designed for
  the 11M-ledger public-archive backfill against local Docker CH.
- **`/home/oski/Projects/stellar/clickhouse-prod-schema.sql`** is the
  canonical production DDL (task 0206 + 0208 + ADR 0044 amendments).
  Notable additions vs ADR 0044: surrogate `Int64` IDs via
  `cityhash64(natural_key)` on three central hubs (accounts,
  soroban_contracts, transactions); `signature LowCardinality(Nullable(String))`
  is hoisted as the first-topic Symbol on `soroban_events` for cheap
  `WHERE signature = 'swap'` filtering; `liquidity_pools` upgraded
  to `ReplacingMergeTree` (was MergeTree in pilot).

What this means for prices-api:

- The §5.6 Stream 1 fast path is **once again viable** — but pointed
  at CH, not PG.
- The `signature`-hoisted column on `soroban_events` is exactly what
  AMM-swap extraction needs: filter `WHERE signature = 'swap' AND
  contract_id IN (Soroswap_router_ids, Aquarius_pool_ids, …)` is
  granule-pruned + LowCardinality-cheap.
- `liquidity_pools` + `liquidity_pool_snapshots` give an alternative
  price source: constant-product instant-price from reserves at any
  ledger, without needing to parse swap events.
- Asset/contract surrogate IDs are deterministic
  `cityhash64(strkey)` — prices-api can pre-derive Soroswap/Aquarius
  router contract IDs on the prices side without an extra JOIN.

## Implementation Plan

### Step 1 — Distill BE-side state (R-note)

Pull together the canonical facts from BE ADR 0044, BE active task
0206, and `clickhouse-prod-schema.sql` into a single research note:
which tables exist, which engines, which ORDER BYs, which surrogate
IDs, what the population status is today vs ADR 0044's "read-empty
pilot" framing.

Output: `notes/R-be-clickhouse-schema-and-status.md`

### Step 2 — Map CH tables onto price-calc requirements (G-note)

Enumerate the prices-api's actual price-calculation needs (six
discrete requirements — see G-note) and for each, name the CH table
that answers it, the relevant columns, and a sketch query.

Output: `notes/G-ch-tables-for-price-calculation.md`

### Step 3 — Enumerate consumption options (I-note)

How does prices-api actually access this data? Four sketches:
cross-account live read, one-time ETL pull + own go-forward,
local replica, no-change archive-read.

Output: `notes/I-integration-options.md`

### Step 4 — Synthesis & recommendation (S-note)

Pick an option. List open questions for human review. Spawn
follow-up backlog tasks.

Output: `notes/S-redesigned-backfill-recommendation.md`

## Acceptance Criteria

- [x] BE-side state distilled in `notes/R-be-clickhouse-schema-and-status.md`
- [x] CH-tables → price-calc mapping in `notes/G-ch-tables-for-price-calculation.md`
- [x] Integration options enumerated in `notes/I-integration-options.md`
- [x] Final recommendation in `notes/S-redesigned-backfill-recommendation.md`
- [x] Task 0010 marked superseded by 0015 (its narrow verification
      question is folded into the R-note + S-note)
- [ ] Spawn follow-up: update `docs/prices-api-general-overview.md`
      §5.6 / §2.3 / §11 to match the CH-sourced Stream 1 reality
      (likely folded into existing backlog task 0013)
- [ ] Spawn follow-up: cross-account CH read plumbing design (if
      Option A is chosen) OR ETL pull job design (if Option B is
      chosen) — decision pending human review
- [ ] Open questions for human input documented in S-note

## Notes layout

```
notes/
├── R-be-clickhouse-schema-and-status.md   — external state distilled
├── G-ch-tables-for-price-calculation.md   — schema→price-calc mapping
├── I-integration-options.md               — consumption-pattern options
└── S-redesigned-backfill-recommendation.md — synthesis + recommendation
```
