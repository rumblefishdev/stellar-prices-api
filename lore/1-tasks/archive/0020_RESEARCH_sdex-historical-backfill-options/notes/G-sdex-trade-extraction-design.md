---
title: 'SDEX trade-extraction design — best way to extract price data from XDR results'
type: generation
status: mature
spawned_from: ../README.md
spawns: []
tags: [sdex, extraction, design, ohlcv, archive-reads, price-calculation]
links:
  - './R-sdex-operation-xdr-shape.md'
  - '../../../archive/0015_RESEARCH_redefine-backfill-with-be-clickhouse-events/notes/G-ch-tables-for-price-calculation.md'
history:
  - date: 2026-05-12
    status: mature
    who: okarcz
    note: 'Extraction algorithm + recommended pipeline; references R-note for XDR shape.'
---

# SDEX trade-extraction design

**Answers user question 2:** _what is the best way to extract data
needed for token price calculation?_

Builds on the XDR shape pinned in
[`R-sdex-operation-xdr-shape.md`](./R-sdex-operation-xdr-shape.md).
The extraction pipeline is **per-ledger walk → ClaimAtom list →
TradeTick struct → OHLCV aggregation**. Decoding is well-shaped
(protocol-defined types, single crate). The hard cost is the
archive-read volume.

## Output unit: `TradeTick`

The smallest extraction unit. One `ClaimAtom` produces one
`TradeTick`. Pair canonicalisation, OHLCV bucketing, and asset-id
surrogation happen on the tick stream.

```rust
struct TradeTick {
    // Lineage (for traceability / deduplication on re-runs)
    ledger_sequence: i64,
    closed_at:       DateTime<Utc>,        // = LedgerHeader.scpValue.closeTime
    transaction_hash: [u8; 32],
    operation_index: u16,                  // application_order within the tx
    claim_index:     u16,                  // 0-based within the op's offers<>

    // Trade core (everything OHLCV needs)
    asset_sold:      AssetIdentity,        // 4-tuple canonical
    amount_sold:     Decimal128_7,         // stroops normalised
    asset_bought:    AssetIdentity,
    amount_bought:   Decimal128_7,

    // Counterparty (informational)
    counterparty: TradeCounterparty,       // enum: OrderBook { seller_id, offer_id }
                                           //     | LiquidityPool { pool_id }
                                           //     | V0Legacy { seller_ed25519 }
}

enum AssetIdentity {
    Native,
    Credit { code: SmolString, issuer: AccountId },
}
```

The 4-tuple form `(asset_type, asset_code, issuer_id, contract_id=0)`
matches the BE CH `assets` table's primary key — adopt it to keep
prices-api's PG and BE's CH ergonomically interoperable.

## Extraction algorithm

```text
for each archive object (one ledger):
    bytes ← read S3 object
    lcm   ← LedgerCloseMeta::from_xdr_bytes(bytes)

    ledger_sequence ← lcm.ledger_header.header.ledger_seq
    closed_at       ← lcm.ledger_header.header.scp_value.close_time
    out             ← Vec<TradeTick>::new()

    for tx_meta in lcm.tx_processing:
        if tx_meta.result.result.result.code != txSUCCESS: continue   // §3.2 below

        tx_hash ← tx_meta.result.transaction_hash

        for (op_idx, op_result) in tx_meta.result.result.result.results.enumerate():
            if op_result.code != opINNER: continue

            atoms ← match op_result.tr:
                ManageSellOffer(r)            if r.code = MANAGE_SELL_OFFER_SUCCESS            → r.success.offers_claimed
                ManageBuyOffer(r)             if r.code = MANAGE_BUY_OFFER_SUCCESS             → r.success.offers_claimed
                CreatePassiveSellOffer(r)     if r.code = MANAGE_SELL_OFFER_SUCCESS            → r.success.offers_claimed
                PathPaymentStrictReceive(r)   if r.code = PATH_PAYMENT_STRICT_RECEIVE_SUCCESS  → r.success.offers
                PathPaymentStrictSend(r)      if r.code = PATH_PAYMENT_STRICT_SEND_SUCCESS     → r.success.offers
                _                                                                              → []

            for (claim_idx, atom) in atoms.enumerate():
                out.push(TradeTick::from_atom(
                    atom,
                    ledger_sequence, closed_at, tx_hash, op_idx, claim_idx))

    emit_batch(out)
```

That's the whole extractor. ~150 lines of Rust including
`From<ClaimAtom>` constructors, error handling, and the streaming
S3 read.

### 3.1 Why per-ledger, not per-tx

Archives are organised per-ledger objects. Per-ledger is the
natural read unit. A single Fargate task can stream-decode at
roughly the rate the archive can deliver bytes (~5–10 MB/s
sustained; ~150–200 k ledgers/hour per §5.6's existing estimate).

### 3.2 `txSUCCESS` filter, not "op success"

Stellar protocol: if **any** op in a tx fails, the **whole** tx
reverts (no ops applied). The tx's individual `OperationResult`s
may still carry "would-have-run" data for non-failing ops, but
those trades never happened on-chain. Filter at the transaction
layer (txSUCCESS only) before walking ops.

### 3.3 LP-vs-orderbook tagging

Path-payment results mix `CLAIM_ATOM_TYPE_ORDER_BOOK` and
`CLAIM_ATOM_TYPE_LIQUIDITY_POOL` in the same `offers<>` Vec
(routing crosses both venues). The `counterparty` enum on
`TradeTick` preserves the distinction so prices-api consumers can
filter ("pure SDEX only") or aggregate (broader "classic on-chain
liquidity") as needed for different endpoints.

Note: **classic-LP swap trades are separate from Soroban-AMM swap
trades**. Classic LPs are the protocol-native LP construct
(LiquidityPoolDeposit/Withdraw ops, BE CH `liquidity_pools` +
`liquidity_pool_snapshots`); they appear in SDEX path-payment
results as `LIQUIDITY_POOL` `ClaimAtom`s. Soroban AMMs (Soroswap
etc., Stream 1) are smart-contract-level and emit swap events to
`soroban_events` — they do NOT appear in `ClaimAtom`s at all.

### 3.4 Asset → AssetIdentity normalisation

Stellar XDR `Asset` carries the issuer as a full `AccountID` (raw
32-byte key + a small type discriminant). prices-api should
normalise this to the BE-CH-compatible 4-tuple at extraction time:

```rust
fn asset_identity(a: &Asset) -> AssetIdentity {
    match a {
        Asset::Native => AssetIdentity::Native,
        Asset::CreditAlphanum4(an) => AssetIdentity::Credit {
            code:   SmolString::from(trim_nuls(&an.asset_code.0)),
            issuer: an.issuer.clone(),
        },
        Asset::CreditAlphanum12(an) => AssetIdentity::Credit {
            code:   SmolString::from(trim_nuls(&an.asset_code.0)),
            issuer: an.issuer.clone(),
        },
    }
}
```

For the prices-api PG `assets` table, the issuer further resolves
to the same `cityhash64(issuer-StrKey)` surrogate Int64 that BE CH
uses (per 0015 R-note §"Surrogate IDs"). Same hash crate
(`cityhash-rs::cityhash_102_128` lower-64). Pre-computed once per
unique issuer; cached.

## Tick → OHLCV aggregation

Aggregation is downstream of extraction and lives in prices-api
proper, not in the backfill task. Sketch:

```sql
-- After tick extraction writes the raw stream into a staging table:
INSERT INTO price_snapshots (asset_pair_id, granularity, bucket_start,
                             open, high, low, close, volume_quote, trade_count)
SELECT
    asset_pair_id,
    '5m' AS granularity,
    toStartOf5Minutes(closed_at) AS bucket_start,
    argMin(price, closed_at) AS open,
    max(price)              AS high,
    min(price)              AS low,
    argMax(price, closed_at) AS close,
    sum(amount_quote)       AS volume_quote,
    count()                 AS trade_count
FROM staging_sdex_trade_ticks
GROUP BY asset_pair_id, granularity, bucket_start
ON CONFLICT (asset_pair_id, granularity, bucket_start) DO UPDATE …
```

(PG-equivalent of the CH window functions. Stream 1's Soroban-AMM
ticks land in the same staging-then-aggregate path, so the SDEX
and AMM streams converge before OHLCV — which is correct, because
a user asking for the XLM/USDC OHLCV wants both SDEX trades AND
Soroswap swaps unioned.)

## Pair canonicalisation rule

Pair direction matters for OHLCV. Recommended rule:

1. If one side is in a known-quote-asset set (USDC, USDT, USD-pegged
   stables), put that side as the quote.
2. Otherwise, deterministic ordering: `(asset_type, asset_code,
issuer_id)` lexicographic. Lower one is base, higher is quote.
3. Cache the chosen orientation per pair in PG `asset_pairs` so the
   choice is stable across re-runs.
4. When ticking, if `asset_sold == base`: price = bought/sold;
   else: price = sold/bought (inverted).

This is a pure prices-api concern; the extractor doesn't need to
know. The extractor emits raw directional ticks; the canonicaliser
operates on the tick stream.

## Pre-filter optimisation via CH `operations_appearances`

The extractor's bottleneck is **archive bytes per ledger**, not
decoding. A ledger with zero trade-shaped ops still costs the same
read+decode as one with 100. Skipping ledgers with no SDEX-shaped
ops avoids that cost.

CH `operations_appearances` has the `type` column carrying the
operation type integer. Query before the archive walk:

```sql
-- Run once, locally, against the same CH instance task 0017 stands up.
-- Result becomes a Bloom filter or sorted Int64 list shipped to the
-- extractor Fargate task.
SELECT DISTINCT ledger_sequence
FROM operations_appearances FINAL
WHERE type IN (2, 3, 4, 12, 13)   -- SDEX-relevant op types per R-note §1
  AND ledger_sequence BETWEEN ? AND ?
ORDER BY ledger_sequence;
```

Trim ratio (fraction of ledgers without any trade-shaped op) is
the deciding metric for whether this optimisation is worth the
plumbing. The S-note carries the recommendation; a concrete
measurement is deferred to spawned follow-up
[task 0021](../../../backlog/0021_RESEARCH_measure-sdex-op-density.md)
(once 0017's local CH lands).

## Why this is "the best way" (vs alternatives)

- **vs Horizon REST scraping**: Horizon's `/trades` endpoint serves
  the same data but is rate-limited, latency-bound (~100 ms/page),
  and stops at a configurable retention horizon. For 57M-ledger
  history-walk you'd hit rate limits and pagination cliffs hard.
  Direct archive reads are 10–100× faster.
- **vs streaming Stellar Core / Captive Core**: Captive Core can
  replay ledgers and emit the same data — but for a one-shot
  historical backfill the archive-read approach has fewer moving
  parts (no Core daemon, no consensus state) and is parallelisable
  across ledger ranges trivially.
- **vs reusing BE's full ingestion pipeline**: BE's pipeline writes
  18 tables of denormalised state that prices-api doesn't need.
  The price-relevant subset is `ClaimAtom`s only; building a
  focused extractor is materially less work than threading
  prices-api's needs through BE's writer machinery.
- **vs waiting for BE to add an `sdex_trades` table to CH**
  (Option C in the I-note): out of prices-api's unilateral
  control; BE has not signalled interest; even if BE built it,
  the population work is the same archive walk and would still
  need to be done somewhere. Stream 2 's blocker is bytes-read
  time, and CH doesn't change that.

## Tooling summary

- **Decoder**: `stellar-xdr` Rust crate (confirmed Q4 of task 0015).
- **Archive transport**: S3-compatible reader against
  `s3://stellar-network-archive-…/` or its mirror; the same source
  BE's backfill runner uses.
- **Runtime**: ECS Fargate task per §5.6 of the design doc; ~2 vCPU
  / 4 GB RAM, single-task with `current_ledger` checkpointing in
  prices-api PG `backfill_progress`.
- **CH pre-filter** (optional refinement): local CH instance from
  task 0017; one query produces the ledger-range Bloom filter.
- **Concurrency**: single Fargate task is the §5.6 baseline; in
  principle the extractor parallelises trivially across disjoint
  ledger ranges if §5.6's 16-day estimate is too long. Multi-task
  fan-out is its own design decision; recommended only if the
  baseline timing is unacceptable.
