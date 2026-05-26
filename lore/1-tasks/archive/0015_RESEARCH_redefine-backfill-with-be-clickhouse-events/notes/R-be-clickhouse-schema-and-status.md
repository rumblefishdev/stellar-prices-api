---
title: "BE ClickHouse — production schema and population status as of 2026-05-12"
type: research
status: mature
spawned_from: ../README.md
spawns: []
tags: [clickhouse, schema, block-explorer, source-of-truth]
links:
  - "../../../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md"
  - "../../../../../soroban-block-explorer/lore/1-tasks/active/0206_FEATURE_clickhouse-persist-real-inserts/README.md"
  - "../../../../../docs/database-schema/clickhouse-prod-schema.sql"
history:
  - date: 2026-05-12
    status: mature
    who: okarcz
    note: "Distilled from ADR 0044, BE task 0206 active state, and clickhouse-prod-schema.sql header comments."
---

# BE ClickHouse — production schema and population status

## Headline

BE's ClickHouse store is **no longer the read-empty pilot** described in
ADR 0044 (proposed 2026-05-08). Between that proposal and today
(2026-05-12), the pilot graduated through:

| BE task | Status | What it did |
|---------|--------|-------------|
| 0204 (FEATURE) | archived | Stood up `crates/db-clickhouse`, Docker compose service, idempotent `init.sql` mirroring 17 PG tables + 1 Dictionary. |
| 0205 (FEATURE) | archived | Added `--target=clickhouse` flag to the backfill runner with stub `persist_ledger_clickhouse` (no-op writer). |
| 0206 (FEATURE) | **active** | Replaces the stub with a real writer that populates all 17 tables + Dictionary. Targets the 11M-ledger public-archive backfill against local Docker CH. |
| 0208 (referenced) | — | Folded `liquidity_pools` into `ReplacingMergeTree` (was `MergeTree` in pilot). Schema-only amendment. |

The canonical schema lives at
[`docs/database-schema/clickhouse-prod-schema.sql`](../../../../../docs/database-schema/clickhouse-prod-schema.sql)
and is self-declared "ClickHouse production schema (task 0206 + 0208 + ADR 0044
amendments)" in the header comment.

## Caveats — what is and isn't true today

- **BE production AWS RDS is still PostgreSQL.** The CH schema is the
  declared target of BE task 0206 against **local Docker CH** during
  the 11M-ledger backfill. There is no AWS-deployed ClickHouse cluster
  in BE's production runtime as of 2026-05-12.
- **ADR 0044 §6 still holds in its original spirit**: no indexer
  dual-write to CH in BE production today; the CH copy is currently
  populated only by the offline backfill runner.
- **Pilot success criteria (ADR 0044 Q6) remain open.** No PASS/FAIL
  threshold has been set for "ClickHouse outperforms PG enough to
  justify migration."

What this means for prices-api: **CH is a viable backfill source**
(local CH instance produced from BE's backfill runner, snapshotted or
queried in bulk), but it is **not yet a viable live-streaming
read-only API source** in BE's production runtime. Any prices-api
plan that assumes "live cross-account query against BE's CH" is
betting on infra that does not exist yet.

## Schema shape — the bits that matter for prices

### Surrogate `Int64` IDs via `cityhash64(natural_key)`

Three central FK hubs carry a deterministic surrogate `id Int64`:

| Table | Natural key → `id` | Derivation |
|-------|--------------------|------------|
| `accounts` | `account_id` (StrKey) | `cityhash64(StrKey bytes)` |
| `soroban_contracts` | `contract_id` (StrKey) | `cityhash64(StrKey bytes)` |
| `transactions` | `hash` (32 bytes) | `cityhash64(hash bytes)` |

Derivation rules (from header comment lines 31–51):

1. **Deterministic** — same StrKey → same `id`, replay-safe.
2. **Hash algo** — `cityhash-rs::cityhash_102_128` lower 64 bits.
   **Not bit-equivalent** to CH SQL `cityHash64()` (different
   algorithm). Prices-api computing IDs on its side must use the
   same Rust crate.
3. Every FK column across the schema (`source_id`, `account_id`,
   `deployer_id`, `contract_id` FKs, `transaction_id` FKs, etc.)
   is the cityhash64 of the referenced natural key.

Practical consequence for prices: **prices-api can pre-compute the
`id` for known Soroswap router / Aquarius pool / Phoenix factory
contracts at compile time** and query CH `soroban_events` by `Int64`
contract_id directly — no JOIN to `soroban_contracts` needed for the
filter, only for display labelling.

### Engine assignment

- **Append-only fact tables** → `ReplacingMergeTree` (dedup by ORDER BY).
- **State tables** → `ReplacingMergeTree(version_column)`.
- **Immutable lookup** → plain `MergeTree`.

Every partitioned table uses `PARTITION BY intDiv(ledger_sequence,
500000)` (~29 days at 5 s/ledger).

### Tables relevant to prices-api (full list)

| Table | Engine | Relevance |
|-------|--------|-----------|
| `soroban_events` | RMT | **Primary AMM-swap source.** Full XDR per event. |
| `liquidity_pools` | RMT(last_updated_ledger) | AMM pool registry (classic Stellar LPs). |
| `liquidity_pool_snapshots` | RMT | Per-ledger reserve history → constant-product price. |
| `assets` | RMT | Asset identity / metadata. |
| `soroban_contracts` | RMT(wasm_uploaded_at_ledger) | Contract registry; map `id` ↔ StrKey + SAC flag. |
| `transactions` | RMT | Ledger-time correlation, success flag, soroban flag. |
| `ledgers` | MergeTree | **Only source of `closed_at` wall-clock time.** |
| `operations_appearances` | RMT | SDEX trade ops (Path payments + offer ops). |
| `transaction_participants` | RMT | Account-side join only (less relevant for prices). |
| `wasm_interface_metadata` | MergeTree | Optional contract metadata for labelling. |

Tables NOT relevant to prices-api: `account_balances_current`, `nfts`,
`nft_ownership`, `lp_positions`, `soroban_invocations_appearances`,
`transaction_hash_index`, `transaction_hash_dict`.

### Critical column shape: `soroban_events`

```
soroban_events (
    contract_id     Int64,          -- cityhash64(StrKey) of emitting contract
    transaction_id  Int64,          -- cityhash64(tx hash)
    ledger_sequence Int64,
    event_index     Int16,
    event_type      Int16,          -- 0=System, 1=Contract, 2=Diagnostic
    signature       LowCardinality(Nullable(String)),  -- HOISTED first-topic Symbol
    topics_xdr      String CODEC(ZSTD(3)),             -- full topics list as XDR bytes
    data_xdr        String CODEC(ZSTD(3))              -- event data as XDR bytes
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (contract_id, ledger_sequence, transaction_id, event_index);
```

The `signature` hoist is the design choice that makes this table
right-shaped for AMM swap extraction. It is the **first topic's
Symbol value** (e.g. `"swap"`, `"transfer"`, `"deposit"`,
`"withdraw"`) lifted out of `topics_xdr` for cheap predicate use.
`Nullable` covers diagnostic / non-symbol-topic events.

The `(contract_id, ledger_sequence, …)` ORDER BY means
`WHERE contract_id = <Soroswap router id>` granule-prunes
aggressively, and the `LowCardinality(String)` signature lets
`AND signature = 'swap'` filter cheaply on top.

`topics_xdr` and `data_xdr` are the **decoded events** the original
prices-api §5.6 design expected to read as JSONB — except they're
ScVal XDR bytes, not JSON. The prices-api will need an XDR parser
(or call into BE's `xdr-parser` crate) to decode them at read time.
The ZSTD(3) codec keeps them cheap to store but does not make them
queryable without decoding.

### Critical column shape: `liquidity_pools` + `liquidity_pool_snapshots`

```
liquidity_pools (
    pool_id              FixedString(32),
    asset_a_type         Int16,
    asset_a_code         LowCardinality(String),
    asset_a_issuer_id    Int64,        -- cityhash64(issuer StrKey), 0 for native
    asset_b_type         Int16,
    asset_b_code         LowCardinality(String),
    asset_b_issuer_id    Int64,
    fee_bps              Int32,
    last_updated_ledger  Int64
)
ENGINE = ReplacingMergeTree(last_updated_ledger)
ORDER BY (pool_id);

liquidity_pool_snapshots (
    pool_id         FixedString(32),
    ledger_sequence Int64,
    reserve_a       Decimal128(7),
    reserve_b       Decimal128(7),
    total_shares    Decimal128(7),
    tvl             Nullable(Decimal128(7)),
    volume          Nullable(Decimal128(7)),
    fee_revenue     Nullable(Decimal128(7))
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (pool_id, ledger_sequence);
```

Notable: this is **Stellar classic liquidity pools** (the protocol's
native LP construct), not Soroban AMMs. Soroswap / Aquarius /
Phoenix are Soroban smart contracts — their state lives in
`soroban_events` + the contracts' own storage (which BE does not
mirror as table columns). So `liquidity_pools` answers price for
classic Stellar LPs only.

### Critical column shape: `assets`

```
assets (
    asset_type      Int16,            -- 0=native, 1=credit_alphanum4, 2=credit_alphanum12, ?=contract
    asset_code      LowCardinality(String),
    issuer_id       Int64,            -- 0 for native / soroban-native
    contract_id     Int64,            -- 0 for native / classic-credit
    name            Nullable(String),
    total_supply    Nullable(Decimal128(7)),
    holder_count    Nullable(Int32),
    icon_url        Nullable(String)
)
ENGINE = ReplacingMergeTree
ORDER BY (asset_type, asset_code, issuer_id, contract_id);
```

The 4-tuple identity matters: native XLM is `(asset_type=0, '', 0, 0)`;
classic credit is `(1|2, code, issuer_id, 0)`; SAC is
`(anything, code|'', issuer_id|0, contract_id)`. Prices-api will
need this same 4-tuple to disambiguate the same asset code across
issuers/contracts (USDC has multiple incarnations).

### Time resolution: `ledgers`

ADR 0044 §4b drops `created_at` from every CH fact table except
`ledgers`. Wall-clock recovery is JOIN to `ledgers.closed_at` (or via
a Dictionary if hot). Prices-api OHLCV bucketing requires wall-clock
time, so every CH query must join through `ledgers`.

```
ledgers (
    sequence          Int64,
    hash              FixedString(32),
    closed_at         DateTime64(3, 'UTC'),
    protocol_version  Int32,
    transaction_count Int32,
    base_fee          Int64
)
ENGINE = MergeTree
PARTITION BY intDiv(sequence, 500000)
ORDER BY (sequence);
```

## Constraint enforcement caveat

CH has no FK / CHECK constraints (ADR 0044 §4 "Cosmetic /
non-translatable PG features"). The schema is **structurally
denormalized** by surrogate IDs but **logically dependent** on the
writer (BE task 0206) producing consistent IDs. Any prices-api
consumer must treat the CH data as an eventually-consistent snapshot,
not as a relationally-validated source.

## Replay idempotency caveat

`ReplacingMergeTree` dedups by ORDER BY in background. Reads against
a freshly-written partition may see duplicate rows until merge runs.
Production CH queries use the `FINAL` modifier (or per-table
materialized views) to get the deduplicated view. Prices-api queries
must account for this — either use `FINAL` or `argMax(...)` patterns.

## What this note answers vs leaves open

**Answers:**

- Does BE have decoded Soroban event payloads anywhere queryable?
  **Yes — in CH `soroban_events.topics_xdr` + `data_xdr`, ZSTD-coded,
  one row per event. The "signature" Symbol is hoisted as a
  LowCardinality column for cheap filtering.** (Folds in the
  open question from task 0010.)
- Is the schema stable enough for prices-api to design against?
  **Yes for the pilot/backfill bucket. BE ADR 0044 §4 lists exactly
  five divergences from the PG schema and freezes them. Schema drift
  risk lives in the PG-mirroring contract, not the CH side itself.**

**Leaves open (handled in S-note + I-note):**

- Where does prices-api actually access this data? (BE has no
  AWS-deployed CH cluster yet.)
- What is the support contract from BE for cross-team consumption?
- Does the ZSTD-coded XDR-decoding cost change the Tranche 1
  "hours, not weeks" claim from §5.6?
