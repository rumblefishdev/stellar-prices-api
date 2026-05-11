# AMM Trades Table — Schema Specification

> **Design history.** The first iteration of this spec proposed a generic
> `soroban_events` table on the Block Explorer RDS that mirrored CAP-67
> shape (`topics JSONB`, `data JSONB`, full event stream). At the medium
> volume scenario it sized to ~970 GB and carried every Soroban contract
> event — most of which the Prices API never reads. This document replaces
> that design with a storage-optimised, write-time-filtered AMM trades
> table (~10 GB at the same scenario, ~100× smaller).

> **External-dependency schema, dedicated to the Prices API.** This table lives
> on the **Soroban Block Explorer RDS**, but is created and populated by the
> Block Explorer Ledger Processor *specifically* to serve the Prices API's
> Soroban AMM backfill (Tranche 1, see `docs/prices-api-general-overview.md`
> §5.6 and `database-schema-overview.md` §7.3, §10).
>
> **Scope.** The Block Explorer indexer applies a write-time filter and
> persists only what the Prices API needs to compute swap-derived prices and
> OHLCV. All other Soroban events (transfers, deposits, withdraws, mints,
> liquidity changes, diagnostic events, etc.) are dropped at decode time and
> never reach this table. Token addresses, amounts, and the AMM venue are
> the only payload — no raw topics, no raw `data` JSONB, no transaction
> hash.
>
> **This is not a general Soroban events store.** If the Block Explorer
> needs one for its own UI, that is a separate table outside the Prices API
> contract.

---

## 1. Purpose

One row per AMM swap on Stellar mainnet, restricted to the three venues the
Prices API consumes:

- Soroswap
- Aquarius
- Phoenix

The Prices API uses this table as the input for its **Soroban AMM
Backfill** ECS task (Tranche 1, `database-schema-overview.md` §7.3). The
backfill aggregates rows here into 1-minute OHLCV candles and upserts them
into the Prices RDS `price_ohlcv` table. Once the backfill completes
(`backfill_progress.task_name = 'soroban_amm'` → `status = 'completed'`),
this table is no longer read by the Prices API. The live Prices Ledger
Processor (overview §5.2) decodes XDR directly and does not touch it.

---

## 2. Business cases the schema must satisfy

Drawn from `prices-api-general-overview.md` and `database-schema-overview.md`:

| # | Business case | Source | Schema implication |
|---|---|---|---|
| 1 | Persist **only** swap events from Soroswap / Aquarius / Phoenix; drop everything else at write time | overview §5.6 ("Block Explorer `soroban_events` … the Prices API connects read-only … extracts token pair + amounts") + user requirement to keep table minimal | Filter applied in BE Ledger Processor; no `topics` or generic `data` columns kept |
| 2 | Bucket each swap into a 1-minute OHLCV candle by ledger close time | overview §5.2, §3.2 | `timestamp TIMESTAMPTZ` per row, partition key |
| 3 | Identify which AMM produced the trade so VWAP can attribute per-source volume | overview §3.3 (`current_prices.sources` JSONB), §5.5 (VWAP per source) | `venue` column with CHECK constraint |
| 4 | Compute the executed price for the swap (price = `amount_out / amount_in`, sided by the in/out token) | overview §5.5 | `token_in`, `token_out`, `amount_in`, `amount_out` |
| 5 | Distinguish each hop of a multi-hop / routed swap | a single transaction can emit N swap events (one per hop) | `tx_index` + `event_index` in PK |
| 6 | Idempotent re-ingest by the BE Ledger Processor (replay-safe) | matches the project's UPSERT convention (schema §3.2 "Write semantics — UPSERT, not INSERT") | Stable natural primary key |
| 7 | Detect coverage gaps (contiguous Nov 2023 → present) so the AMM backfill task can fall back to archive reads for missing ledger ranges | overview §11.4, schema §10.1 | Orderable `ledger_seq BIGINT` |
| 8 | Time-range scans hit only relevant months | matches conventions in `price_ohlcv` and `oracle_prices` (schema §3.2, §3.4) | `PARTITION BY RANGE (timestamp)`, monthly |

---

## 3. DDL

```sql
CREATE TABLE prices_amm_trades (
    -- Position — uniquely identifies one swap event for idempotent UPSERT.
    timestamp        TIMESTAMPTZ NOT NULL,           -- ledger close time, drives partitioning
    ledger_seq       BIGINT      NOT NULL,           -- ledger sequence number
    tx_index         SMALLINT    NOT NULL,           -- transaction position within the ledger
    event_index      SMALLINT    NOT NULL,           -- swap-event position within the transaction

    -- Venue + routing.
    venue            VARCHAR(10) NOT NULL
                     CHECK (venue IN ('soroswap', 'aquarius', 'phoenix')),
    pair_contract_id VARCHAR(56) NOT NULL,           -- AMM pair / pool C-address

    -- Trade economics.
    token_in         VARCHAR(56) NOT NULL,           -- C-address of the sold token
    token_out        VARCHAR(56) NOT NULL,           -- C-address of the bought token
    amount_in        NUMERIC(28,14) NOT NULL,        -- amount of token_in sold
    amount_out       NUMERIC(28,14) NOT NULL,        -- amount of token_out received

    PRIMARY KEY (timestamp, ledger_seq, tx_index, event_index)
) PARTITION BY RANGE (timestamp);

-- Monthly partitions, starting at Soroban activation (Nov 2023).
CREATE TABLE prices_amm_trades_2023_11 PARTITION OF prices_amm_trades
    FOR VALUES FROM ('2023-11-01') TO ('2023-12-01');
CREATE TABLE prices_amm_trades_2023_12 PARTITION OF prices_amm_trades
    FOR VALUES FROM ('2023-12-01') TO ('2024-01-01');
-- ... one per month, created two months ahead by the BE indexer
--     (mirrors the Cleanup Worker pattern in schema §4).
```

No secondary indexes. The Prices API backfill scans by time range and the
PK already starts with `timestamp`, so partition pruning + the local PK
index serve the only query path. (See §5 "Index choice" below for
rationale.)

---

## 4. Column rationale

| Column | Type | Why kept |
|---|---|---|
| `timestamp` | `TIMESTAMPTZ` | Drives `PARTITION BY RANGE` (matches `price_ohlcv`/`oracle_prices`) and lets the AMM backfill bucket each trade into the correct 1m candle without a join. |
| `ledger_seq` | `BIGINT` | Natural ordering across the entire stream; required for gap detection ("contiguous coverage from Soroban activation", schema §10.1). 8 bytes; current tip ~57M. |
| `tx_index` | `SMALLINT` | Disambiguates events when one ledger contains multiple swap transactions. `SMALLINT` (max 32767) is far above any realistic per-ledger transaction count. |
| `event_index` | `SMALLINT` | Disambiguates events within one transaction — a routed multi-hop swap emits one swap event per hop. Same width logic. |
| `venue` | `VARCHAR(10) CHECK …` | The Prices API VWAP attributes volume per source (overview §5.5, `current_prices.sources` JSONB §3.3). The `venue` value flows into `price_ohlcv.source` and into the `sources` JSONB key. CHECK constraint matches the project's pattern in `assets.asset_type` (schema §3.1) and `price_ohlcv.granularity`. |
| `pair_contract_id` | `VARCHAR(56)` | The AMM pair / pool contract C-address. Lets the Prices API match the trade against its pair registry to resolve `(token_in, token_out)` to internal `assets.id`s, and lets us recover all trades for a single pool for debugging. Width matches `assets.contract_address`. |
| `token_in` | `VARCHAR(56)` | C-address of the sold token. Carried per-row (not derived from `pair_contract_id`) because Aquarius pools can hold > 2 tokens, so the pair contract alone does not identify the pair traded in a given event. |
| `token_out` | `VARCHAR(56)` | Same reasoning as `token_in`. |
| `amount_in` | `NUMERIC(28,14)` | Matches the precision used in `price_ohlcv` (overview §3.2). Together with `amount_out` it gives the executed price (`amount_out / amount_in`) and one side of the volume calculation. |
| `amount_out` | `NUMERIC(28,14)` | Other side of the swap; gives `volume_base` in whichever token the Prices API treats as base. |

### What is deliberately omitted

| Field | Why not |
|---|---|
| `topics` JSONB | The whole table is pre-filtered to swap events, and `venue` already encodes which of the four decoder shapes produced the row (see §7). `topics[1..N]` carries trader addresses (`from`, `to`) which the Prices API does not consume. |
| `data` JSONB | All Prices-relevant fields from the event payload are already lifted into typed columns above (`token_in`, `token_out`, `amount_in`, `amount_out`). Storing the original JSONB would be ~250 B/row of dead weight. |
| `transaction_hash BYTEA(32)` | No Prices API query references it. The `(ledger_seq, tx_index, event_index)` triple already gives uniqueness for the UPSERT. |
| `event_type` (system / contract / diagnostic) | Pre-filtering means every row is a contract swap event by definition. |
| `fee` / `protocol_fee` | Prices API computes executed price from `amount_in` / `amount_out`, which already reflects fees as observed by the user. Per-AMM fee accounting is not in scope. |
| `from_address` / `to_address` | Trader identity is not used by any Prices API endpoint or worker. |
| Computed `price` column | Derivable in the consumer (`amount_out / amount_in` sided by which token is base). Storing it would force a write-time choice of "price of what in what" that doesn't exist on-chain — direction depends on the Prices API's pair registry. |
| FK from `pair_contract_id` to a pairs table | Foreign keys to high-write time-series tables are expensive and the BE owns whatever pair registry it maintains; the reference is logical only. Same reasoning as `assets.id` ↔ `price_ohlcv.asset_id` in the Prices RDS (schema §3.0 note). |
| Surrogate `BIGSERIAL id` | Natural composite PK is already 20 bytes and stable under replay. |

---

## 5. Primary key, partitioning, indexes

### Primary key
`(timestamp, ledger_seq, tx_index, event_index)` — `timestamp` first so
partition pruning is effective on the only hot query (range scan in time
order). The remaining three columns guarantee uniqueness within a
partition. Mirrors the `price_ohlcv` PK convention (schema §3.2).

### Partitioning
- `PARTITION BY RANGE (timestamp)`, monthly.
- First partition: `prices_amm_trades_2023_11` (Soroban activation).
- New partitions created **two months ahead** of the current date, same
  cadence as `price_ohlcv` and `oracle_prices`.
- Retention is the BE's call. The Prices API only reads this table during
  the one-time Tranche-1 backfill; partitions older than ~Tranche-1
  completion can be dropped without affecting the Prices API.

### Index choice
**No secondary indexes.** The single hot query — the Prices AMM Backfill
task scanning all trades in a time range, oldest → newest — is served by
partition pruning plus the local PK index. Adding a `(pair_contract_id,
timestamp)` index would cost write throughput and storage for an access
pattern that doesn't exist in the Prices API contract.

If the BE team finds it useful for ops/debugging, an optional index can be
added later without breaking the Prices API:
```sql
-- Optional, ops-only:
CREATE INDEX idx_prices_amm_trades_pair
    ON prices_amm_trades (pair_contract_id, timestamp DESC);
```

---

## 6. Write semantics

The BE Ledger Processor inserts into `prices_amm_trades` using
`INSERT … ON CONFLICT (timestamp, ledger_seq, tx_index, event_index)
DO UPDATE` — same pattern the Prices API uses for `price_ohlcv` (schema
§3.2 "Write semantics — UPSERT, not INSERT"). This is required for two
reasons that match the Prices API design:

1. **Re-processing the same ledger** (e.g. BE indexer restart from
   checkpoint after a crash) must be idempotent.
2. **Replay tolerance** — if the BE re-decodes a ledger because the swap
   filter was updated to recognise a new AMM event variant, existing rows
   for that ledger are overwritten cleanly.

**Merge rule:** all non-PK columns are replaced (`SET col = EXCLUDED.col`).
There is no incremental merge here — each row corresponds to a single,
already-finalised swap event.

---

## 7. Scope of the BE indexer's filter

The BE Ledger Processor must persist a row **iff** the event satisfies all of:

1. The event is a Soroban contract event (CAP-67) emitted from
   `SorobanTransactionMeta.events` in the ledger's `LedgerCloseMeta` XDR.
2. The emitting `contract_id` belongs to the **per-venue pool / router
   registry** maintained by the BE (one of `soroswap`, `aquarius`,
   `phoenix`). The registry is enumerated by factory events — see
   "Pool enumeration" below.
3. The event matches one of the **four decoder shapes** in
   "Per-venue decoder reference" below, dispatched on
   `(topics[0].kind, topics[0].value)` and (for Soroswap) `topics[1]`.
   ScVal kind matters: `Symbol("swap")` is the Aquarius router and
   `String("swap")` is a Phoenix XYK pool — they are different decoders
   and must not be merged on the normalised string `"swap"`.
4. The decoded payload yields a usable `(token_in, token_out, amount_in,
   amount_out)` quadruple. For Phoenix, this requires grouping eight
   `String("swap")` events into one logical trade (see
   "Per-venue payload notes" below). Events the indexer cannot decode
   are logged and skipped, not persisted.

Liquidity events (deposit / withdraw / add_liquidity / remove_liquidity),
transfers, mints, burns, oracle update events, and Soroswap pool `sync`
events are **not** persisted — they do not contribute to executed-trade
prices.

### Per-venue decoder reference

Empirical findings from lore 0001 + 0002 (4-day mainnet sample, ledger
range 62400000–62463999):

| `topics[0].kind` | `topics[0].value` | `topics[1]` | Venue | Decoder | Data shape | Trade rows |
|---|---|---|---|---|---|---:|
| `Symbol` | `swap` | router-specific | **Aquarius** | router | `Vec<i128>(in, out, fee)` payload variant | 1 event = 1 trade |
| `Symbol` | `trade` | `Address(sold)`, `Address(bought)`, `Address(trader)` | **Aquarius** | constant-product pool | `Vec<i128>(in, out, fee)` | 1 event = 1 trade |
| `String` | `swap` | `String(<field>)` × varying | **Phoenix** | XYK / stable pool | scalar-per-event; reassemble | **8 events = 1 trade** (6 for stable) |
| `String` | `SoroswapPair` | `Symbol("swap")` | **Soroswap** | pool | uniswap-v2 `Map{amount_0_in, amount_0_out, amount_1_in, amount_1_out, to}` | 1 event = 1 trade |

User-facing wrapper events from Soroswap (`String("SoroswapRouter")`
and `String("SoroswapAggregator")`) are emitted in addition to the
underlying `SoroswapPair` pool event. The indexer keeps only the
`SoroswapPair` row to avoid double-counting.

The `Symbol("swap")` router decoder applies **only** to the Aquarius
router `CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK`.
Other `Symbol("swap")` emitters observed in the same window
(~5,200 events / 4 days from `CCR2CH4G...`, `CDMIM23W...`, and a long
tail) were manually verified as **non-target** — not Soroswap, not
Aquarius, not Phoenix — and are dropped. The indexer applies a strict
known-target allowlist: emitters outside the per-venue registry are
skipped, not bucketed as `venue: unknown`. Evidence:
`lore/1-tasks/archive/0005_RESEARCH_unknown-symbol-swap-emitters/notes/S-unknown-emitters-non-target.md`.

### Pool enumeration

The per-venue pool / router registry is populated by replaying factory
events from genesis. Canonical mainnet addresses verified against task
0001 emitter `contract_id`s:

| Venue | Factory / router | Pool-creation topic | Data |
|---|---|---|---|
| Soroswap | `CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2` (factory) | `[String("SoroswapFactory"), Symbol("new_pair")]` | `NewPairEvent { token_0: Address, token_1: Address, pair: Address, new_pairs_length: u32 }` |
| Aquarius | `CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK` (router) | `Symbol("add_pool")` | `(pool_address: Address, pool_type: Symbol)` — `pool_type` is `constant_product` / `stable` / `concentrated` |
| Phoenix | `CB4SVAWJA6TSRNOJZ7W2AWFW46D5VR4ZMFZKDIKXEINZCZEGZCJZCKMI` (factory) | `[Symbol("create"), Symbol("liquidity_pool")]` | `Address(pool_id)` |

Additional canonical addresses verified in the same sample:

| Venue | Role | Address / hash |
|---|---|---|
| Soroswap | Router | `CAG5LRYQ5JVEUI5TEID72EYOVX44TTUJT5BQR2J6J77FH65PCCFAJDDH` |
| Soroswap | Aggregator | `CAYP3UWLJM7ZPTUKL6R6BFGTRWLZ46LRKOXTERI2K6BIJAWGYY62TXTO` |
| Soroswap | Pair WASM hash | `18051456816b66f12e773a56f77c5794fac1b1fb7ab6e22d4fad5a412770f73e` |
| Aquarius | Constant-product pool WASM hash | `ae0da5a84b15805c5c7931ac567a8d1b34be3f26b483993d9ff80cb2c3de9852` |
| Phoenix | Multihop (router) | `CCLZRD4E72T7JCZCN3P7KNPYNXFYKQCL64ECLX7WP5GNVYPYJGU2IO2G` |

Aquarius is unique: its router is both the swap entry-point and the
registry. The indexer must read `pool_type: Symbol` from each
`add_pool` event and dispatch each pool's trade events to the right
per-type decoder.

### Per-venue payload notes

**Phoenix — multi-event grouping.** Phoenix XYK pools emit **8 separate
events per swap** (6 for stable pools), all with topic shape
`[String("swap"), String(<field>)]` and scalar-per-event payloads
(`offer_amount`, `ask_amount`, `spread_amount`, `total_fee_bps`,
`sender`, etc.). The indexer must group by
`(tx_hash, op_index, contract_id)` to reassemble one logical trade
into one `prices_amm_trades` row. Filtering on the topic alone yields
N rows per actual trade, which is incorrect.

**Soroswap — two-topic filter.** The `SoroswapPair` pool contract emits
multiple `topics[0] = String("SoroswapPair")` event types per pool
(`swap`, `sync`, `deposit`, `withdraw`). The indexer must require
`topics[1] = Symbol("swap")` to keep only swap events; the other three
are non-trade and must be dropped.

**Soroswap — no inline tokens.** The `SoroswapPair` swap event payload
carries `amount_0_in`, `amount_0_out`, `amount_1_in`, `amount_1_out`,
`to` — **no token addresses**. The indexer resolves `token_0` /
`token_1` from each pool's `NewPairEvent` (captured at pool-discovery
time) and caches the `(pair, token_0, token_1)` triple, then joins on
`pair_contract_id` to fill `token_in` / `token_out` in
`prices_amm_trades`. Without the cache, every Soroswap trade would
need an extra chain read.

**Phoenix — fee shape (informational; not stored).** `prices_amm_trades`
does not carry a `fee` column (see §4 "What is deliberately omitted").
For completeness: Phoenix XYK pools emit `spread_amount` but **not**
`commission_amount` — the commission is settled by a direct token
transfer from the pool to `fee_recipient`. If a future revision of this
schema adds a `fee` column, the recommended Phoenix fill strategy is
`total_fee_bps × offer_amount`, with `total_fee_bps` read once from
each pool's config at pool-discovery time and cached. Aquarius and
Soroswap emit fee inline in the swap event payload. No action required
for the current schema.

---

## 8. Storage estimate

Per-row footprint:

| Component | Bytes |
|---|---|
| Heap tuple header + null bitmap + alignment | ~24 |
| `timestamp` (8) + `ledger_seq` (8) + `tx_index` (2) + `event_index` (2) | 20 |
| `venue` (varlena, ~10) | ~12 |
| `pair_contract_id`, `token_in`, `token_out` (3 × ~60 B varlena C-address) | ~180 |
| `amount_in`, `amount_out` (NUMERIC(28,14), typical magnitudes) | ~32 |
| Padding | ~10 |
| **Heap subtotal** | **~280 B** |
| PK index entry | ~36 |
| **Per-row total (no secondary index, +15% bloat)** | **~360 B** |

Row count is bounded by **AMM swap volume**, which is a tiny fraction of
total Soroban contract events. Public Stellar telemetry over Nov 2023 →
2026-05 suggests tens of thousands of AMM swaps per day across the three
venues.

| Scenario | Swaps / day | Rows (Nov 2023 → May 2026, ~915 days) | Storage |
|---|---|---|---|
| Low (early Soroban, low DEX activity) | ~10,000 | ~9.2 M | **~3.3 GB** |
| Medium (realistic) | ~30,000 | ~27 M | **~10 GB** |
| High (sustained busy) | ~100,000 | ~92 M | **~33 GB** |

Forward growth at the medium scenario: ~11 M rows / year ≈ **~4 GB / year**.

This is **roughly two orders of magnitude smaller** than a full
`soroban_events` table — see "Storage comparison" in this conversation's
predecessor design.

---

## 9. Cross-service contract

- The Prices API connects **read-only** from the Soroban AMM Backfill ECS
  task within the shared VPC.
- The connection is used **only during Tranche 1**; once the AMM backfill
  marks `backfill_progress.task_name = 'soroban_amm'` as
  `status = 'completed'`, the connection is no longer used.
- The Prices API never writes to `prices_amm_trades`.
- The BE never reads from any Prices API table.
- Schema changes to `prices_amm_trades` after Tranche 1 completes are
  harmless to the Prices API. Schema changes during Tranche 1 require
  coordination — flagged in `database-schema-overview.md` §10.1.

---

## 10. Reference query — what the Soroban AMM Backfill runs

Illustrative; the real implementation lives in the Prices API AMM backfill
ECS task.

```sql
-- Page through all AMM swap trades in time order, per partition, so the
-- writer can aggregate trades into 1m candles before upserting into
-- price_ohlcv (whole-row replacement, schema §3.2 backfill writer rule).
SELECT timestamp,
       ledger_seq,
       venue,
       pair_contract_id,
       token_in,
       token_out,
       amount_in,
       amount_out
FROM   prices_amm_trades
WHERE  timestamp >= $1 AND timestamp < $2
ORDER  BY timestamp, ledger_seq, tx_index, event_index;
```

Partition pruning eliminates months outside `[$1, $2)`. The PK index serves
the ordering directly — no sort step.

The backfill task then:
1. Resolves each `(token_in, token_out)` pair to internal `assets.id`s via
   the Prices API's own asset registry.
2. Buckets each row into the correct `(timestamp_minute, asset_id, '1m')`
   key with `source = venue`.
3. Upserts the resulting candles into `price_ohlcv` using the
   whole-row-replacement merge rule for backfill writers (schema §3.2).
4. After all partitions are processed, runs a coverage check
   (overview §11.4, schema §10.1) and falls back to archive reads for any
   ledger ranges where `prices_amm_trades` has gaps.

---

## 11. Open questions for the BE team

Items resolved by lore tasks 0001 + 0002 + 0005 are marked as such with
references. The remaining item is still pending BE input.

1. **~~Filter symbol per AMM.~~** **Resolved** by lore 0001 + 0002.
   Four decoder shapes are in use, dispatched on
   `(topics[0].kind, topics[0].value)` and (for Soroswap) `topics[1]`
   — see §7 "Per-venue decoder reference". Key surprise:
   `Symbol("swap")` (Aquarius router) and `String("swap")` (Phoenix XYK
   pool) are distinct ScVal kinds with different decoders; Phoenix is
   event-multiplexed (8 events → 1 trade) and Soroswap requires a
   two-topic filter. Evidence:
   `lore/1-tasks/archive/0001_RESEARCH_dump-amm-swap-events/notes/R-swap-topic-shapes.md`,
   `lore/1-tasks/archive/0001_RESEARCH_dump-amm-swap-events/notes/S-amm-trades-schema-§11-1-resolved.md`,
   `lore/1-tasks/archive/0002_RESEARCH_amm-venue-attribution/notes/S-venue-attribution-mapping.md`.

2. **~~Pair / pool contract registry.~~** **Resolved** by lore 0002 +
   0005. Factory-event discovery per venue, with canonical factory /
   router addresses and pool-creation topics listed in §7 "Pool
   enumeration". Aquarius is special: its router is itself the
   registry and emits `add_pool` with a `pool_type` discriminator
   (`constant_product` / `stable` / `concentrated`). Non-target
   `Symbol("swap")` emitters outside the three venues are dropped via
   a strict allowlist (no `venue: unknown` bucket) — see lore 0005.

3. **Decimals normalisation.** `amount_in` and `amount_out` are stored
   as `NUMERIC(28,14)`. Confirm whether the BE Ledger Processor will
   divide raw on-chain amounts by each token's `decimals()` before
   insertion, or write the raw integer scalar. The Prices API expects
   the **decimal-normalised** value (matching how it stores quantities
   throughout `price_ohlcv` and `current_prices`). *Still open.*

---

_Companion to `prices-api-general-overview.md` and `database-schema-overview.md`._
_Owned by the Soroban Block Explorer service; documented here because the
Prices API depends on it._
