---
title: "ClickHouse schema → prices-api token-price-calculation mapping"
type: generation
status: mature
spawned_from: ../README.md
spawns: []
tags: [schema, mapping, price-calculation, clickhouse, design]
links:
  - "./R-be-clickhouse-schema-and-status.md"
  - "../../../../../docs/database-schema/clickhouse-prod-schema.sql"
  - "../../../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-05-12
    status: mature
    who: okarcz
    note: "Per-requirement table mapping with example queries."
---

# Schema → price-calculation mapping

This note answers the user's core ask: **define how the new BE
ClickHouse schema can be used by prices-api to calculate token
prices.** Each price-calculation requirement gets a CH-side answer:
which table(s), which columns, an example query sketch, and the
caveat that applies.

The mapping presumes the consumer can compute the
`cityhash64(StrKey)` surrogate IDs locally (using BE's same Rust
`cityhash-rs::cityhash_102_128` lower-64-bits derivation per
[R-note §Surrogate IDs](./R-be-clickhouse-schema-and-status.md)).

## Six price-calculation requirements

| # | Requirement | Primary CH table(s) | Method |
|---|-------------|--------------------|--------|
| 1 | Soroban AMM swap events → OHLCV trade points | `soroban_events` + `ledgers` | Filter by `signature='swap'` & known AMM `contract_id`s; decode `topics_xdr`/`data_xdr` |
| 2 | SDEX trades → OHLCV trade points | `operations_appearances` + `ledgers` (+ S3 archive for full XDR) | Granule-prune by `ledger_sequence`; CH `operations_appearances` lacks `offersClaimed[]` — still needs archive read |
| 3 | Classic-Stellar LP reserves → instant constant-product price | `liquidity_pools` + `liquidity_pool_snapshots` | Pair lookup → per-ledger reserve snapshot → `price = reserve_b / reserve_a` |
| 4 | Asset identity (disambiguate USDC variants, etc.) | `assets` | 4-tuple lookup `(asset_type, asset_code, issuer_id, contract_id)` |
| 5 | Ledger-time resolution (`ledger_sequence` → wall-clock `closed_at`) | `ledgers` | JOIN on every fact-table query; cache as Dictionary if hot |
| 6 | Contract labelling (`contract_id` Int64 → human StrKey + SAC flag) | `soroban_contracts` | JOIN for display only — IDs are computable locally for filtering |

## Requirement 1 — Soroban AMM swaps (Stream 1 of §5.6)

**This is the requirement that drove the original §5.6 Stream 1
fast-path.** With CH's full-content `soroban_events`, it is now
implementable as a database query — no archive S3 reads needed for
the swap events themselves.

### Approach

Soroban AMMs (Soroswap, Aquarius, Phoenix) emit contract events with
a `"swap"` symbol topic. The full topic list and data payload are
ScVal-encoded XDR — typically:

- Topics: `["swap", <from_address>, <to_address>]` or
  `["swap", <pool>, <token_in>, <token_out>]` (varies by AMM).
- Data: a `ScVal::Map` or `ScVal::Vec` containing
  `amount_in`, `amount_out`, possibly `token_in`, `token_out`.

Exact event shape per AMM contract requires verification (BE has not
documented per-AMM signatures; sample decoding from a Soroswap router
event will pin down the shape). This is the open spike from BE task
0009 row 8 / task 0010 — partially answered (the data is there),
fully answered once we decode a real sample.

### Example query — extract Soroswap swap events for a ledger range

```sql
-- Pre-compute on prices-api side (constants in Rust):
-- SOROSWAP_ROUTER_ID  = cityhash64("CA…")     -- Int64
-- AQUARIUS_POOLS      = [cityhash64("CA…"), …]
-- PHOENIX_FACTORY_ID  = cityhash64("CA…")

SELECT
    e.contract_id,
    e.ledger_sequence,
    l.closed_at,                -- wall-clock for OHLCV bucket
    e.transaction_id,
    e.event_index,
    e.topics_xdr,               -- ScVal XDR — decode on prices-api side
    e.data_xdr                  -- ScVal XDR — decode on prices-api side
FROM soroban_events FINAL AS e
INNER JOIN ledgers AS l
    ON l.sequence = e.ledger_sequence
WHERE e.contract_id IN (?, ?, ?)        -- AMM router/pool contract IDs
  AND e.signature = 'swap'              -- LowCardinality predicate
  AND e.ledger_sequence BETWEEN ? AND ? -- granule-prunes by partition
ORDER BY e.ledger_sequence, e.transaction_id, e.event_index;
```

Notes:

- `FINAL` ensures `ReplacingMergeTree` dedup is applied at read time.
  Performance cost is real on hot ranges; the alternative is an
  `argMax(...)` pattern keyed on `(contract_id, ledger_sequence,
  transaction_id, event_index)`.
- The `WHERE contract_id IN (…) AND signature = 'swap'` predicate
  hits the primary key prefix → granule-pruned + LowCardinality-cheap.
- `topics_xdr` and `data_xdr` are `String CODEC(ZSTD(3))` — wire
  format from CH is the raw decoded bytes. Prices-api decodes them
  with `stellar-xdr` (Rust crate).

### Decoded price extraction (prices-api side)

After fetching the rows, prices-api decodes `data_xdr` as `ScVal`
and extracts `amount_in` / `amount_out` (denoted in stroops or
contract-defined precision). Combined with the token pair from
`topics_xdr`, this gives a trade tick:
`price = amount_out / amount_in` (in the `out → in` direction).

OHLCV bucketing groups these ticks by `closed_at` truncated to the
target granularity (e.g. 5-minute, hourly).

## Requirement 2 — SDEX trades (Stream 2 of §5.6)

**CH does not save prices-api from the archive-read for SDEX.**

The CH `operations_appearances` table mirrors the PG appearance
shape — it stores `(transaction_id, application_order, type,
source_id, destination_id, contract_id, asset_code, asset_issuer_id,
pool_id, amount, ledger_sequence)`. This is **operation appearance
metadata, not operation result detail**. The SDEX `offersClaimed[]`
array lives inside the operation's `OperationResult` XDR — which BE
does not unfold into a table column on either side (PG or CH).

So Stream 2 keeps its archive-read shape from §5.6. CH does add a
modest convenience: `operations_appearances` can be used to
**pre-filter** which ledgers contain `ManageSellOffer` /
`ManageBuyOffer` / `PathPayment*` ops, reducing the archive-read
volume from "every ledger" to "ledgers with at least one trade-shaped
op" (still tens of millions of ledgers, but a meaningful win).

### Example pre-filter query

```sql
SELECT DISTINCT ledger_sequence
FROM operations_appearances FINAL
WHERE type IN (
        3,   -- MANAGE_SELL_OFFER
        12,  -- MANAGE_BUY_OFFER
        2,   -- PATH_PAYMENT_STRICT_RECEIVE
        13   -- PATH_PAYMENT_STRICT_SEND
      )
  AND ledger_sequence BETWEEN ? AND ?
ORDER BY ledger_sequence;
```

The archive-read backfill then walks only this subset, fetching the
`LedgerCloseMeta` for each and extracting `offersClaimed[]`.

(Operation type integer values are Stellar protocol-defined; the
canonical mapping is in the `stellar-xdr` crate's
`OperationType` enum. Verify the integer values against the
current protocol version when implementing.)

## Requirement 3 — Classic-Stellar LP instant price

For classic Stellar liquidity pools (the protocol-native LP construct,
not Soroban AMMs), price can be read directly from reserves at any
ledger without parsing events.

### Approach

`liquidity_pools` gives the pool identity and asset pair.
`liquidity_pool_snapshots` gives the per-ledger reserves. For a
constant-product pool: `price(A in terms of B) = reserve_b /
reserve_a`. Stellar's classic LPs are constant-product
(`liquidity_pool_constant_product`), so this is the only formula
needed.

### Example query — latest reserves for a pool

```sql
SELECT
    lps.pool_id,
    lps.ledger_sequence,
    l.closed_at,
    lps.reserve_a,
    lps.reserve_b,
    lps.total_shares,
    lps.tvl
FROM liquidity_pool_snapshots FINAL AS lps
INNER JOIN ledgers AS l
    ON l.sequence = lps.ledger_sequence
WHERE lps.pool_id = ?         -- specific pool's 32-byte hash
  AND lps.ledger_sequence <= ?
ORDER BY lps.ledger_sequence DESC
LIMIT 1;
```

For a price history series: drop `LIMIT 1` and bucket by
`l.closed_at` in the calling code.

### Caveat: classic LPs only

This requirement **only covers classic Stellar LPs**. Soroswap /
Aquarius / Phoenix pool reserves live in Soroban contract storage,
which BE does not mirror as table columns. For Soroban AMM
instant-price-from-reserves, prices-api would need to either:

(a) Reconstruct reserves by replaying swap events from
    `soroban_events` (expensive but possible).
(b) Query the live network's contract storage via Soroban RPC at a
    target ledger (precise but live-only, no historical replay).
(c) Wait for BE to add per-Soroban-AMM reserve snapshots — not
    currently on BE's roadmap.

The swap-event approach (Requirement 1) is the only fully-historical
path for Soroban AMMs today.

## Requirement 4 — Asset identity disambiguation

USDC has three relevant incarnations on Stellar:
1. Classic credit issued by Circle (`USDC` + Circle issuer G…).
2. SAC wrap of the classic asset (same code + issuer, plus a
   `contract_id` for the SAC).
3. Native-Soroban USDC contracts (different `contract_id`, often no
   classic mapping).

The `assets` 4-tuple `(asset_type, asset_code, issuer_id,
contract_id)` is the unique identity. Prices-api MUST carry this
4-tuple, not just `asset_code`, to avoid conflating them.

### Example query — full identity lookup

```sql
SELECT
    asset_type,
    asset_code,
    issuer_id,                  -- cityhash64 of issuer StrKey; 0 for native
    contract_id,                -- cityhash64 of contract StrKey; 0 for classic-credit
    name,
    total_supply,
    holder_count,
    icon_url
FROM assets FINAL
WHERE asset_code = 'USDC'
  AND asset_type IN (1, 2);     -- credit_alphanum4 or _alphanum12
```

## Requirement 5 — Ledger-time resolution

Every CH fact-table query that needs wall-clock time joins
`ledgers.closed_at`. ADR 0044 §4b makes this mandatory by dropping
`created_at` from fact tables.

Two patterns:

1. **JOIN on every query** (shown in Requirements 1, 2, 3 above).
   Simple, costs one PK lookup per row on the `ledgers` table.
2. **CH Dictionary** — define a `ledger_time_dict` sourced from
   `ledgers` with `LIFETIME(MIN 300 MAX 360) LAYOUT(HASHED())`
   for RAM-resident lookups. Then queries use
   `dictGet('ledger_time_dict', 'closed_at', tuple(ledger_sequence))`.

ADR 0044 §4e already established this pattern for
`transaction_hash_dict` (cache-key layout, bounded RAM). A
`ledger_time_dict` is a natural follow-on; prices-api can request
it of BE or define it as part of its own CH consumer schema if
running a local CH instance.

## Requirement 6 — Contract labelling

Prices-api needs the StrKey form of a `contract_id` for two reasons:
(a) returning human-readable contract addresses in API responses;
(b) operator/admin debugging.

Since the surrogate ID is deterministic, prices-api can:

1. Maintain a **local registry** of well-known AMM contract StrKeys
   → Int64 IDs (computed once with `cityhash-rs::cityhash_102_128`).
2. For arbitrary contracts, JOIN to `soroban_contracts` for the
   reverse lookup.

### Example query — reverse lookup for arbitrary contracts

```sql
SELECT
    id,
    contract_id,                -- StrKey form
    wasm_hash,
    deployer_id,
    deployed_at_ledger,
    contract_type,
    is_sac,
    name
FROM soroban_contracts FINAL
WHERE id IN (?, ?, ?);          -- Int64 IDs from soroban_events.contract_id
```

For known AMMs, skip the join entirely — store a static
`Vec<(StrKey, Int64)>` in the prices-api Rust binary.

## Cross-cutting: ZSTD decode cost on `topics_xdr` / `data_xdr`

These columns are `String CODEC(ZSTD(3))`. The CH server transparently
decodes ZSTD on read, so the client receives raw decoded bytes. The
decode is CPU-bounded on the CH side; for a Tranche 1 backfill across
~8.5M ledgers worth of events (low millions of swap events filtered
down from a much larger event total), this is comfortably in the
"hours, not weeks" envelope from §5.6 as long as the consumer can
ingest at the CH server's read rate.

What is **not** automatic: ScVal/XDR parsing of the decoded bytes.
That happens on the prices-api side after the bytes arrive. Use the
`stellar-xdr` Rust crate; a single Soroban event decode is sub-µs,
so for low-millions of events the parsing cost is well under an hour
of single-threaded CPU.

## Open spike (deferred to a future task)

Per-AMM swap event signatures need to be decoded from real samples
before the Stream 1 implementation can pin down the topic/data
extraction logic. Concretely:

- Sample 1 real Soroswap swap event → record topic list + data shape.
- Sample 1 real Aquarius swap event → record same.
- Sample 1 real Phoenix swap event → record same.

These can be obtained either by (a) querying a populated BE CH
instance directly once 0206 lands, or (b) decoding a few events
from the public archive for known swap transactions on each AMM.

This is a discrete, low-risk spike — recommended as a follow-up
backlog task once the consumer-pattern decision (I-note) is made.
