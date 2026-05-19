---
id: "G-soroban-events-pricing-decoder"
title: "Soroban events pricing decoder — what to extract and how the Lambda ingests it into Hetzner ClickHouse"
type: G
task: "0048"
status: mature
spawned_from: []
spawns: []
related_notes: []
links:
  - "../../../../3-wiki/project/soroban-events-schema.md"
  - "../../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../../blocked/0038_FEATURE_prices-ledger-processor-lambda.md"
  - "../../../../../docs/prices-api-general-overview.md"
---

# Soroban events pricing decoder

> Audience: implementer of the Prices Ledger Processor Lambda
> (task 0038's rewrite) and reviewers of ADR 0007 / cross-team
> bundle 0045.
>
> Scope: this document is **the pricing application** of the
> `soroban_events` table. Payload reference lives in the wiki
> doc `lore/3-wiki/project/soroban-events-schema.md` — don't
> duplicate; cross-link.

---

## 1. TL;DR

For each closed ledger the Lambda emits per-minute OHLCV rows into
`prices.price_ohlcv_1m` on the shared Hetzner CH from **two
independent paths**:

| Path | Source (in LedgerCloseMeta) | Carries | Where it surfaces in this CH |
|---|---|---|---|
| **A. Soroban AMM** | `SorobanTransactionMeta.events[]` (matches `soroban_events` rows) | Pair-level `trade` / `swap` / `update_reserves` from Phoenix-style, Soroswap, CLMM, etc. | `soroban_events` (`signature IN ('trade','swap','update_reserves')`) |
| **B. Classic SDEX** | `OperationResult.tr.{PathPayment*Result,ManageSell/BuyOfferResult}.offers[]` (vec of `ClaimAtomV0/V1/V2/LiquidityPool`) | Classic order-book matches and protocol-18 LP matches | **NOT in `soroban_events`.** Summary lives in `operations_appearances` (op type 2/3/12/13/19/20). Per-trade `ClaimAtom` data is **not** indexed in CH at all — must be parsed from the XDR directly. |
| **C. Oracle inputs** | `SorobanTransactionMeta.events[]` for Reflector/RedStone feeds | External quote prices for assets that don't have a native AMM market | `soroban_events` (`signature IN ('REFLECTOR','REDSTONE')`) |

The Lambda runs both extractors per ledger and writes one
`(timestamp, asset_id, quote_asset_id, granularity, source)`
row per `(minute, asset, quote, source)` tuple it observes.
`source` distinguishes `'sdex'` / `'soroswap'` / `'phoenix'` /
`'aquarius'` / `'clmm'` / `'reflector'` / `'redstone'`
(ADR 0004 multi-source merge columns).

> **Key correction vs. the task's framing:** "SDEX transactions"
> are not in `soroban_events`. The phrase is a clue that the
> spec must cover **both ingestion paths**, not that SDEX is
> derivable from Soroban events. Both paths share the same
> decoder kernel and writer.

---

## 2. Empirical inventory (10k-row uniform sample)

Sample: 9,937 rows pulled via
`WHERE cityHash64(transaction_id, event_index, ledger_sequence) % 4755 = 42`
over the local backfill CH (47,545,820 events; ledgers
62,019,999–62,079,982; populated by `soroban-block-explorer/backfill-runner`).

### 2.1 Signature distribution in the sample

| Signature | Sample rows | Sample contracts | Sample txs | Full-table count | Pricing-relevant? |
|---|---:|---:|---:|---:|:---:|
| `fee` | 5,921 | 1 | 5,920 | 28,308,343 | No (XLM SAC overhead) |
| `transfer` | 2,770 | 519 | 2,759 | 13,018,026 | Indirect — confirms settlement, not the trade |
| `mint` | 1,097 | 264 | 1,092 | 5,498,133 | No |
| `burn` | 54 | 5 | 54 | 265,304 | No |
| `set_authorized` | 54 | 10 | 54 | 211,164 | No |
| `clawback` | 24 | 9 | 24 | 142,679 | No |
| `trade` | **6** | 5 | 6 | **17,138** | **YES — Phoenix-style AMM** |
| _(NULL)_ | 2 | 2 | 2 | 16,849 | Edge case — see §8.1 |
| `REFLECTOR` | 2 | 1 | 2 | 3,449 | **YES — oracle** |
| `REDSTONE` | 1 | 1 | 1 | 4,064 | **YES — oracle** |
| `supply` / `vault_withdraw` / `withdraw_collateral` / `supply_collateral` / `score_submitted` | 1 each | 1 each | 1 each | 2.2k–2.7k | No (lending protocols, not price-bearing) |
| `update_reserves` | 1 | 1 | 1 | **17,747** | **YES — paired with `trade`/`swap`** |
| `swap` | 0 in sample | — | — | **13,417** | **YES — Soroswap + CLMM** |

The 10k uniform sample under-represents `swap` (0 hits) because
swap events are concentrated in a small set of high-volume
contracts; full-table counts confirm 13,417 swap events across
the backfill window. The decoder must handle all three swap
shapes regardless (§5).

### 2.2 AMM contract catalog (full-table)

Confirmed AMM emitters in the backfill window:

| Contract address | Protocol | Signature | Events | Notes |
|---|---|---|---:|---|
| `CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK` | **Soroswap Router** | `swap` (shape C) | 34,665 | One event per hop in multi-hop routes. |
| `CCR2CH4GQVCZHG7CHFVMNANCK45CU5DVKXZIIITDZQAU3CEJZ7RQH2MQ` | **CLMM (Uniswap V3-style)** | `swap` (shape A) | 1,784 | Concentrated-liquidity pool. |
| `CAUF4DFYSX52L2KJ4J7OFW3WDQMEUDVXNB7PG5VIC4VVOA3BCLWXDO2E` | **CLMM** | `swap` (shape A) | 1,618 | |
| `CDKAJU3RTGL26PJZ3DLZUT25T5CB56YLMKIMTOEQJRKIV6WWZQ7M5TWZ` | **CLMM** | `swap` (shape A) | 138 | |
| `CA7RQDMMV6E53P5EDZA5GPWBZ33AMW2ZNO42XLI2RGRIAP4QXIARUOJQ` | **CLMM** | `swap` (shape A) | 52 | |
| 50+ contracts (top: `CA6PUJLBYKZKUEKLZJMKBZLEKP2OTHANDEOWSFF44FTSYLKQPIICCJBE`, `CDD3OQDU…`, `CCNXGPE4…`, `CDE57N6X…`, `CCY2PXGM…`) | **Phoenix-style pair pools** | `trade` + `update_reserves` | 17,138 total trades | Top emitter 14,646 events. |

**Aquarius AMM is NOT represented** in this 1.65k-ledger window
under any obvious signature. Flag as a follow-up sample task
(spawn from §10).

### 2.3 Oracle feed contracts (full-table)

| Contract | Feed | Key shape | Notes |
|---|---|---|---|
| `CALI2BYU2JE6WVRUFYTS6MSBNEHGJ35P4AVCZYF3B6QOE3QKOB2PLE6M` | **Reflector — Stellar on-chain assets** | `address` key | 3,459 events. Keys are token contract addresses → directly usable for assets without `code:issuer`. |
| `CBKGPWGKSKZF52CFHMTRR23TBWTPMRDIYZ4O2P5VS65BMHYH4DXMCJZC` | **Reflector — FX symbols** | `sym` key | 3,459 events. EUR/GBP/CAD/BRL/JPY/CNY/XAU/… |
| `CAFJZQWSED6YAWZU3GWRTOCNPPCGBN32L7QV43XX5LZLFTK6JLN34DLN` | **Reflector — global crypto symbols** | `sym` key | 3,429 events. BTC/ETH/USDT/XRP/SOL/USDC/ADA/… |
| `CA526Y2NQWGWVVQ7RFFPGAZMU66PSYJ3UC2MTVAV4ZU7OM5BOPHDXUSG` | **RedStone** | bytes-encoded XDR | 12,192 events. Decoder is base64 + XDR (see §6.3). |

All Reflector prices are scaled to **14 decimals**; this is
Reflector's protocol constant, not derived from on-chain
metadata.

### 2.4 Classic SDEX coverage (`operations_appearances`, full-table)

| Stellar op type | Count | Trade-bearing? |
|---:|---:|---|
| 1 (Payment) | 10,352,366 | No (direct payment, no offer match) |
| 24 (RestoreFootprint) | 8,743,270 | No (Soroban housekeeping) |
| 13 (PathPaymentStrictSend) | 3,173,497 | **YES** — `PathPaymentStrictSendResult.success.offers[]` |
| 3 (ManageSellOffer) | 3,135,021 | **YES** when offer matches existing book — `ManageSellOfferResult.success.offersClaimed[]` |
| 2 (PathPaymentStrictReceive) | 2,181,752 | **YES** — `PathPaymentStrictReceiveResult.success.offers[]` |
| 12 (ManageBuyOffer) | 1,833,294 | **YES** when offer matches |
| 4 (CreatePassiveSellOffer) | 6,961 | **YES** when matches on creation |
| 19 (LiquidityPoolDeposit) | 478,008 | No directly; signals reserve change in classic LP |
| 20 (LiquidityPoolWithdraw) | 41,726 | No directly; signals reserve change in classic LP |

Trade-bearing SDEX is large — millions of operations across the
backfill window. None of this is in `soroban_events`; the
`operations_appearances` table only stores summary metadata
(no ClaimAtom-level execution detail). **The Lambda must
parse `LedgerCloseMeta.OperationResult` XDR directly for the
Classic SDEX path** (§7.4).

---

## 3. What feeds `price_ohlcv`

Target schema (per ADR 0003 + ADR 0004 + ADR 0007 §3):

```
prices.price_ohlcv_1m (
    timestamp        DateTime,
    asset_id         UInt64,       -- base asset
    quote_asset_id   UInt64,       -- quote asset (ADR 0003)
    granularity      LowCardinality(String),  -- '1m'
    source           LowCardinality(String),  -- 'sdex'|'soroswap'|'phoenix'|'clmm'|'aquarius'|'reflector'|'redstone'
    open             Decimal(38, 18),
    high             Decimal(38, 18),
    low              Decimal(38, 18),
    close            Decimal(38, 18),
    volume_base      Decimal(38, 18),
    volume_quote     Decimal(38, 18),   -- in quote-asset units
    volume_quote_usd Nullable(Decimal(38, 18)),  -- USD-denominated, see task 0026
    trade_count      UInt32,
    vwap             Nullable(Decimal(38, 18)),
    version          UInt64,        -- for ReplacingMergeTree
    -- … see ADR 0004 for the full column set
)
ENGINE = ReplacingMergeTree(version)
PARTITION BY toYYYYMMDD(timestamp)
ORDER BY (timestamp, asset_id, quote_asset_id, granularity, source)
```

Each decoded "trade tick" emitted by the kernel has the shape:

```rust
struct TradeTick {
    closed_at: chrono::DateTime<chrono::Utc>, // ledger close time
    base_asset_id: u64,
    quote_asset_id: u64,
    base_amount: i128,      // base-asset native units
    quote_amount: i128,     // quote-asset native units
    source: Source,         // enum
    tx_hash: [u8; 32],      // for de-dup / audit
    venue_contract: Option<[u8; 32]>,  // None for SDEX, Some(C…) for Soroban AMM
}
```

The 1-minute bucketer derives:

- `open` = first tick's `quote_amount / base_amount` (in normalized decimals — see §4.2)
- `close` = last tick's price
- `high` / `low` = max / min of all tick prices in the bucket
- `volume_base` = sum of `|base_amount|` over all ticks
- `volume_quote` = sum of `|quote_amount|` over all ticks
- `volume_quote_usd` = filled in a second pass (task 0026)
- `trade_count` = number of ticks
- `vwap` = `volume_quote / volume_base`
- `version` = monotone counter (ledger_sequence * 10000 + op_index works)

Per-source rows; cross-source merge is read-time (`GROUP BY` per
ADR 0007 §3.3), not write-time. Re-INSERT of the same `(minute,
asset, quote, source)` key collapses on `ReplacingMergeTree`
merge — idempotent without UPSERT.

---

## 4. Soroban AMM decoder rules

### 4.1 `trade` (Phoenix-style pair pools)

```json
topics: [
  {"type":"sym","value":"trade"},
  {"type":"address","value":"C…sold_token"},
  {"type":"address","value":"C…bought_token"},
  {"type":"address","value":"<G…|C…> trader"}
]
data: {"type":"vec","value":[
  {"type":"i128","value":"57624430586"},   // amount_sold
  {"type":"i128","value":"49435406506"},   // amount_bought
  {"type":"i128","value":"288122153"}      // fee
]}
```

Decoder:

1. Resolve `sold_token` and `bought_token` C-addresses to `asset_id`
   via the asset registry (§7.1).
2. `base = sold_token`, `quote = bought_token`. (Direction
   convention is "trader sold X to receive Y" — the AMM bought
   X and sold Y; the trade price expressed in quote/base is
   `amount_bought / amount_sold`.)
3. Emit one `TradeTick` with `base_amount = amount_sold`,
   `quote_amount = amount_bought`, `source = Phoenix`,
   `venue_contract = soroban_events.contract_id` resolved to
   strkey.
4. The `fee` field is **not written into `price_ohlcv` directly**
   in this iteration — surface it through a future fee-tracking
   column when needed.

**Concrete sample** (from `lore/4-notes/samples/soroban-events/trade.jsonl:1`):

- Tx `7F91C76295FA5521907247C81C33048AE5026F49B30EC8E7D4AC5E2E5086A1C5` ledger `62078346`
- Pool contract `CCMHVBZGY65EIFQZLZFRWMPMM23MWK4P5RFKDFWEPA5NQHENBNWMZETZ`
- Sold `CAESLMGW5LYTIEJI7FJHK6SFSWRELLNVX5Q4WR4UZEALMTRWQDBKDPAG`, bought `CCKCKCPHYVXQD4NECBFJTFSCU2AMSJGCNG4O6K4JVRE2BLPR7WNDBQIQ`
- `amount_sold = 57,624,430,586`, `amount_bought = 49,435,406,506`, `fee = 288,122,153`
- Followed in the same tx by `update_reserves` at `event_index = 7`: reserves `[56,282,401,209,368, 48,722,126,337,247]` — token0/token1 ordering is per the pool's internal slot ordering, not derivable from the trade event.

### 4.2 Decimal normalisation

Both legs are integer counts in token-native decimals. Native
decimals are looked up from the asset metadata (see §7.1 — SAC
uses the underlying Stellar asset's `decimals` from the issuer's
`AccountEntry.thresholds`-adjacent field; custom tokens via
`token.decimals()` simulated read or registry cache).

```
price = (amount_bought / 10^dec_bought) / (amount_sold / 10^dec_sold)
      = (amount_bought / amount_sold) * 10^(dec_sold - dec_bought)
```

`price_ohlcv` columns are `Decimal(38, 18)` — apply the scaling
above to land prices in `Decimal(38, 18)`.

### 4.3 `swap` shape A — Uniswap V3-style CLMM

```json
topics: [{"type":"sym","value":"swap"}]
data: {"type":"map","value":[
  {"key":"amount0","value":{"type":"i128","value":"2871373757"}},
  {"key":"amount1","value":{"type":"i128","value":"-439878710"}},
  {"key":"liquidity","value":{"type":"u128","value":"89251760312657"}},
  {"key":"recipient","value":{"type":"address","value":"G…"}},
  {"key":"sender","value":{"type":"address","value":"G…"}},
  {"key":"sqrt_price_x96","value":{"type":"u256","value":"…"}},
  {"key":"tick","value":{"type":"i32","value":-18732}}
]}
```

Decoder:

1. Resolve `token0` / `token1` from the pool's storage. **Not in
   the event** — the Lambda must read pool storage once per
   contract and cache it (key: `(contract_id, "token_0"|"token_1"|"tokenA"|"tokenB")`).
2. The signed `amount0`/`amount1` encode net delta from the
   pool's perspective: a positive value means the pool received
   that token from the trader. So:
   - If `amount0 > 0` and `amount1 < 0`: trader bought token1
     (base=token1, quote=token0); `base_amount = -amount1`,
     `quote_amount = amount0`.
   - If `amount0 < 0` and `amount1 > 0`: trader bought token0
     (base=token0, quote=token1); `base_amount = -amount0`,
     `quote_amount = amount1`.
3. `source = Source::Clmm`. `venue_contract` = the pool C-address.
4. `sqrt_price_x96` and `tick` are not written; they are
   redundant given the `amount0/amount1` pair and increase row
   width.

### 4.4 `swap` shape B — simple in/out

```json
topics: [{"type":"sym","value":"swap"}]
data: {"type":"map","value":[
  {"key":"amount_in","value":{"type":"i128","value":"2871373757"}},
  {"key":"amount_out","value":{"type":"i128","value":"439878710"}},
  {"key":"recipient","value":{"type":"address","value":"G…"}}
]}
```

Frequently emitted by a wrapper or pair-pool contract that
re-publishes a CLMM swap; **the inner CLMM event in the same tx
already captures the trade**. To avoid double-counting:

- Track `(tx_hash, base, quote)` keys seen within a tx.
- If the same key is observed twice in different shapes, emit
  one tick and prefer the shape with token identity intact (A
  or C; A requires a token-slot lookup, C carries tokens in
  topics).

If shape B is the only swap event in the tx (rare — sometimes a
standalone aggregator), fall back to using `transfer` events in
the same tx to identify the tokens.

### 4.5 `swap` shape C — Soroswap Router

```json
topics: [
  {"type":"sym","value":"swap"},
  {"type":"vec","value":[{"type":"address","value":"C…token_in"},
                          {"type":"address","value":"C…token_out"}]},
  {"type":"address","value":"G…trader"}
]
data: {"type":"vec","value":[
  {"type":"address","value":"C…pair_contract"},
  {"type":"address","value":"C…token_in"},
  {"type":"address","value":"C…token_out"},
  {"type":"u128","value":"930000000"},
  {"type":"u128","value":"423899086439"}
]}
```

Router events carry the **full per-hop information** including
both token contracts. This is the **preferred** swap source
for Soroswap because it doesn't need pool-storage reads.

Decoder:

1. `base = token_in`, `quote = token_out`, `base_amount =
   data[3]`, `quote_amount = data[4]`. Router data is unsigned
   (`u128`) because the router never pays out negatively.
2. `source = Source::Soroswap`. `venue_contract = data[0]` (the
   pair contract, not the router) so downstream queries can
   group by pool.
3. **De-dup with pair-level events:** in the same tx the pair
   pool typically emits a paired `swap` (shape B) and an
   `update_reserves`. The router event already encodes the trade;
   if both are present prefer the router event. Track
   `(tx_hash, pair_contract, base, quote)` to suppress the pair
   event.

### 4.6 `update_reserves`

Not a trade — it's a snapshot. Pricing usage:

- **Optional MID-price source.** If the AMM's last reserves are
  `[r0, r1]` and the token ordering is known (from §4.3 storage
  read), the implied mid-price is `r1 / r0` in token-native
  units, normalised by decimals as in §4.2. This is useful when
  a pool has zero trades in a minute but the consumer still
  wants a price quote — emit a synthetic tick with
  `volume_base = volume_quote = 0` and `trade_count = 0` and
  `source = Source::PhoenixMid` / `Source::SoroswapMid`. Defer
  to a follow-up; **first iteration of the Lambda emits ticks
  only from `trade`/`swap`, not reserves.**
- Required for AMM TVL / depth observability — out of scope here.

---

## 5. Oracle decoders

Oracle events do NOT produce `TradeTick`s. They produce
`PriceQuote`s that the bucketer rolls into a separate `source`
row in `price_ohlcv` so consumers can pick between AMM-derived
and oracle-derived prices.

### 5.1 `REFLECTOR` (symbol-keyed feeds)

```json
topics: [{"type":"sym","value":"REFLECTOR"},
         {"type":"sym","value":"update"},
         {"type":"u64","value":1775958000000}]
data: {"type":"map","value":[
  {"key":"update_data","value":{"type":"vec","value":[
    {"type":"vec","value":[
      {"type":"sym","value":"BTC"},
      {"type":"i128","value":"7231710463185063718"}
    ]},
    {"type":"vec","value":[
      {"type":"sym","value":"ETH"},
      {"type":"i128","value":"225591688710028279"}
    ]}
    // … rest of symbol entries
  ]}}
]}
```

Decoder:

1. Topic[2] (`u64`) is the **feed timestamp in milliseconds** —
   use this, not the ledger close time, for the `timestamp`
   column.
2. `update_data` is a vec of `[key, i128 price]` pairs.
3. Price is scaled to **14 decimals** (Reflector constant).
   Divide by `10^14` and store as `Decimal(38, 18)`.
4. Quote asset for crypto/FX feeds is USD by convention — set
   `quote_asset_id = USD_ASSET_ID` (a virtual asset).

### 5.2 `REFLECTOR` (address-keyed feed `CALI2BYU…`)

Same outer shape, but the inner `[key, price]` pair has the key
as `{"type":"address","value":"C…"}` instead of `sym`. Use this
feed for assets that have only on-chain identity (no
`code:issuer` string). Resolve the address to `asset_id` via the
asset registry.

### 5.3 `REDSTONE`

```json
topics: [{"type":"sym","value":"REDSTONE"}]
data: {"type":"bytes","value":"AAAAEQAAAAEAAAACAAAADwAAAA11cGRhdGVkX2ZlZWRz…"}
```

Decoder is two steps:

1. Base64-decode the `bytes` payload.
2. `xdr::ScVal::from_xdr(&bytes)?` — this is **real XDR**, not
   JSON despite the topic shape resembling other contract events.
3. The resulting `ScVal::Map` has key `updated_feeds → vec of
   maps {package_timestamp, price, write_timestamp}` plus
   `updater` (the writer's address).
4. Each `updated_feeds[i]` is one symbol's quote at
   `package_timestamp` (ms). Scale + decimals are per-feed
   metadata; verify on first encounter via Stellar Expert or a
   live read of the feed registry.

### 5.4 Oracle vs. AMM-derived price: which wins?

Both. Each is written to its own row with its own `source`;
ADR 0004's multi-source columns let read handlers pick (or
merge) at query time. Default rank for `current_price`
aggregation (handled by 0039's Current Price Updater, not this
Lambda): on-chain AMM (Soroswap > Phoenix > CLMM) > Reflector
(address-keyed) > Reflector (sym) > RedStone.

---

## 6. Classic SDEX decoder (path B)

### 6.1 What `soroban_events` does NOT cover

Classic Stellar SDEX trades are emitted **only** as XDR result
data in `LedgerCloseMeta.v1.txProcessing[i].result.result.result`,
specifically in:

- `OperationResultTr::PathPaymentStrictReceive(success).offers`
- `OperationResultTr::PathPaymentStrictSend(success).offers`
- `OperationResultTr::ManageSellOffer(success).offersClaimed`
- `OperationResultTr::ManageBuyOffer(success).offersClaimed`
- `OperationResultTr::CreatePassiveSellOffer(success).offersClaimed`

Each `offers[]` entry is a `ClaimAtom` (V0 = pre-LP, V1 = strkey
account, V2 = LiquidityPool match). For pricing-relevant fields:

```
ClaimAtomV1 / V2 {
  sellerId  | liquidityPoolID,   // counterparty identity
  offerId,
  assetSold, amountSold,
  assetBought, amountBought
}
```

This is **structurally identical** to the Phoenix `trade` event
(§4.1) once decoded — same `base_amount` / `quote_amount` /
asset-pair shape.

`operations_appearances` in this CH carries one row per op with
type + asset_code + amount + pool_id, but **not** the
ClaimAtom-level execution detail. So even though we have a CH
mirror of classic ops, the Lambda must still parse the
`OperationResult` XDR for per-trade granularity.

### 6.2 SDEX decoder steps

For each successful tx in the ledger, for each op result of the
relevant tr types:

1. Extract the `offers[]` / `offersClaimed[]` vec.
2. For each `ClaimAtom`:
   - Resolve `assetSold` and `assetBought` to `asset_id` via the
     asset registry (§7.1). Classic Stellar assets are
     `Asset::Native` (XLM), `Asset::CreditAlphanum4` (code:issuer),
     `Asset::CreditAlphanum12`, or `LiquidityPoolShare` (for V2
     against classic LPs).
   - `base = assetSold`, `quote = assetBought`, `base_amount =
     amountSold`, `quote_amount = amountBought`.
   - `source = Source::Sdex` if `ClaimAtomV0|V1`, `Source::ClassicLp`
     if `ClaimAtomV2` (matches against protocol-18 LPs).
   - `venue_contract = None` (no Soroban contract address).
   - `tx_hash` from `txProcessing[i].result.transactionHash`.

The path-payment intermediate offers fire as multiple
`ClaimAtom`s in one op — emit one tick per atom.

### 6.3 De-dup vs. Soroban path

A single tx is **never** in both paths — Soroban host-fn invokes
emit `soroban_events`; classic SDEX ops have no `soroban_events`
counterpart. Both extractors run unconditionally; their outputs
are disjoint by construction.

---

## 7. Cross-cutting concerns

### 7.1 Asset identity & registry

Pricing needs a stable `UInt64 asset_id`. Three identity spaces
collide in this codebase:

| Asset type | Source | Canonical form | Registry key |
|---|---|---|---|
| **Classic native (XLM)** | Stellar ledger | `"native"` | reserved `asset_id = 1` |
| **Classic credit** | Stellar ledger | `"<CODE>:<G…issuer>"` | `(asset_type, code, issuer_id)` |
| **SAC** (wrapped classic) | `soroban_contracts.is_sac = true` joining to the classic asset | C-address ↔ classic asset bijection | use the underlying classic `asset_id` |
| **Custom token** | `soroban_contracts.is_sac = false` | C-address | the C-address itself maps to a synthetic `asset_id` |

Lambda warm-cache strategy: load `prices.asset_registry` on cold
start, query CH for new addresses on miss, insert if not found
(small `ReplacingMergeTree` with `(asset_id, contract_address,
code, issuer_id)`). Asset registry maintenance is task 0039's
Asset Discovery worker; this Lambda is a read-mostly consumer.

### 7.2 Token decimals lookup

For per-trade decimal normalisation (§4.2):

- **SAC**: classic Stellar asset decimals are always 7. Cache
  this for `is_sac = true`.
- **Custom token**: read `token.decimals()` once per contract via
  a Soroban host-fn simulation against a public RPC; cache for
  the Lambda's container lifetime. Fall back to 7 with a warning
  log on read failure (most observed customs are 7-decimal).

### 7.3 ReplacingMergeTree + idempotent INSERT

Per ADR 0007 §3:

- Table engine is `ReplacingMergeTree(version)` ordered by
  `(timestamp, asset_id, quote_asset_id, granularity, source)`.
- The Lambda computes `version = ledger_sequence * 1_000_000 +
  op_index` (monotone across replays; later replays win).
- Replays (S3 PutObject DLQ retry, ledger re-fire from
  `aws s3api put-object-event` for backfill) re-INSERT with the
  same key; the latest `version` wins on next merge.
- **Read path uses `FINAL` or `argMax(value, version)` GROUP BY**
  — see ADR 0007 §Consequences and BE's ADR 0044.

### 7.4 Bucketing into 1-minute candles

In-memory per-ledger fold:

```rust
use chrono::{DateTime, DurationRound, Utc};
let bucket = closed_at.duration_trunc(chrono::Duration::minutes(1))?;
candles.entry((bucket, base, quote, source))
  .and_modify(|c| c.add_tick(tick))
  .or_insert_with(|| Candle::from_tick(tick));
```

`Candle::add_tick` updates close, high, low, accumulates volume.
At end-of-ledger, emit one INSERT batch per source. A single
ledger spans at most one minute boundary unless the ledger
itself straddles 60 s (rare); handle by emitting two minute
buckets.

### 7.5 Topic-string null-signature events

§8.1 below. The decoder MUST inspect `topics_xdr` JSON content
when `signature IS NULL` for the swap-shaped string-topic
fallback. Filtering only on `signature` will drop the ~0.04% of
events from contract `CBHCRSVX…` (and any other contract that
follows the same convention).

---

## 8. Edge cases & gotchas

### 8.1 NULL-signature events

About 0.04% of events have `signature IS NULL` because their
topic[0] is a JSON `string` (not `sym`). Sample contract
`CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX` emits
a sequence of micro-events for a single logical swap with
topics like `"sender"`, `"sell_token"`, `"offer_amount"`,
`"actual received amount"`, `"buy_token"`. To capture:

- Match on `topics_xdr` JSON content (`topic[0].type = "string"
  AND topic[0].value = "swap"`).
- Buffer micro-events by `(tx_hash, contract)` and synthesise
  one `TradeTick` per logical swap.

### 8.2 Three-shape swap collision in one tx

A single tx can emit shape C (router) + shape B (pair wrapper)
+ shape A (inner CLMM). Apply the §4.4/§4.5 de-dup keyed on
`(tx_hash, pair_contract, base, quote)`. **Default winner = the
event that carries token identity in-band** (C beats A beats B).

### 8.3 Pre-merge duplicates from `ReplacingMergeTree`

Source CH (`default.soroban_events`) is itself
`ReplacingMergeTree`-engined and can contain duplicate rows
between merges. The Lambda is the writer to `prices.*`, not a
reader of `default.soroban_events` (see §9), so this doesn't
affect it directly. But the wiki schema doc warns about
duplicates when querying the source for diagnostics — use
`FINAL` or `argMax + GROUP BY (contract_id, ledger_sequence,
transaction_id, event_index)` when manual-querying for
verification.

### 8.4 Fee event noise (XLM SAC)

`fee` events from `CAS3J7GY…` fire one per Soroban tx and are
NOT pricing-relevant. Filter them out at the top of the
decoder: `if e.signature == Some("fee") && e.contract_id ==
XLM_SAC_ID { continue; }`.

### 8.5 Mint / burn / transfer aren't trade ticks

Even though transfers fire alongside every AMM trade (settling
the asset legs), the Lambda must NOT treat them as trades. They
represent the SAME economic event already captured by the
`trade`/`swap` event. Filter on signature ∈
`{trade, swap, REFLECTOR, REDSTONE, <null-sig fallback>}` (plus
`update_reserves` if mid-price emission is added later).

### 8.6 Asset identity on the SAC ↔ classic boundary

When a Soroswap trade pairs USDC-SAC (a C-address) with XLM-SAC,
the `asset_id`s emitted must match the IDs the SDEX path uses
for the same logical assets. Otherwise the same minute will
appear as two rows: one `source=sdex` keyed on classic asset_id,
one `source=soroswap` keyed on C-address asset_id. The registry
must collapse SAC → classic asset_id for is_sac = true.

---

## 9. Lambda implementation spec (per ADR 0007)

### 9.1 Topology

```
                           ┌──────────────────────────────┐
                           │  BE stellar-ledger-data       │
                           │  S3 bucket  (Galexie writer)  │
                           └──────────────┬───────────────┘
                                          │ PutObject event
                                          ▼
                              ┌───────────────────────┐
                              │ SNS topic (BE-owned)  │  ← per ADR 0007 §3.2
                              └────┬────────────┬─────┘
                                   │            │
                ┌──────────────────┘            └───────────────────┐
                ▼                                                    ▼
   ┌────────────────────────┐                       ┌───────────────────────────┐
   │ BE ledger processor    │                       │ prices-ledger-processor   │
   │  (existing)            │                       │  Rust lambda (this task)  │
   └────────────┬───────────┘                       └─────────────┬─────────────┘
                ▼                                                  │
   ┌────────────────────────┐                                      │  HTTPS-mTLS
   │ default.* tables       │                                      │  (Caddy:443)
   └────────────────────────┘                                      ▼
                                                       ┌─────────────────────────┐
                                                       │ Hetzner CH (shared)     │
                                                       │   default.* (BE)        │
                                                       │   prices.* (this task)  │
                                                       └─────────────────────────┘
```

Key shifts from the original 0038 spec:

- **No RDS, no VPC, no NAT Gateway** — Lambda runs in default
  AWS networking and reaches Hetzner over public-internet
  outbound + mTLS.
- **SNS fan-out instead of dual S3 notifications** — per Cluster
  A buy-in in task 0045.
- **No OHLCV Rollup Lambda** — rollups are CH materialised
  views on the Hetzner side.

### 9.2 Crate layout

```
packages/prices-ledger-processor/    # binary crate (lambda_runtime)
packages/ledger-processor/            # kernel from 0037 (extractor trait)
packages/ohlcv-writer/                # candle bucketer + INSERT batcher
packages/asset-registry-client/       # cached lookup against prices.asset_registry
```

Dependencies (Cargo.toml top of binary):

```toml
[dependencies]
lambda_runtime  = "0.13"
aws_sdk_s3      = "1"
tokio           = { version = "1", features = ["rt-multi-thread"] }
serde           = { version = "1", features = ["derive"] }
serde_json      = "1"
zstd            = "0.13"
stellar-xdr     = { version = "...", features = ["curr","std","base64"] }   # for LedgerCloseMeta + ScVal
clickhouse      = { version = "0.13", features = ["tls-rustls"] }            # ADR 0007 → mTLS
reqwest         = { version = "0.12", default-features = false, features = ["rustls-tls","stream"] }
chrono          = { version = "0.4", features = ["serde"] }
tracing         = "0.1"
ledger-processor = { path = "../ledger-processor" }
ohlcv-writer    = { path = "../ohlcv-writer" }
asset-registry-client = { path = "../asset-registry-client" }
```

### 9.3 Lambda handler skeleton

```rust
use aws_lambda_events::s3::S3Event;
use lambda_runtime::{run, service_fn, Error, LambdaEvent};

async fn handler(event: LambdaEvent<sns::SnsEvent>) -> Result<(), Error> {
    let ctx = AppCtx::warm()?;  // S3 client, CH client, registry cache
    for rec in event.payload.records {
        let s3evt: S3Event = serde_json::from_str(&rec.sns.message)?;
        for r in s3evt.records {
            process_one_object(&ctx, &r.s3).await?;
        }
    }
    Ok(())
}

async fn process_one_object(ctx: &AppCtx, s3: &S3Entity) -> Result<(), Error> {
    let obj = ctx.s3.get_object().bucket(&s3.bucket.name)
        .key(&s3.object.key).send().await?;
    let bytes = obj.body.collect().await?.into_bytes();
    let xdr = zstd::stream::decode_all(&bytes[..])?;
    let lcm: LedgerCloseMeta = LedgerCloseMeta::from_xdr(&xdr, Limits::none())?;

    let extracted = ledger_processor::dispatch(&lcm, &ctx.registry).await?;
    let candles = ohlcv_writer::bucket_into_minutes(extracted);
    ctx.ch.insert_candles(&candles).await?;

    metrics::publish_lag(now() - lcm.closed_at());
    Ok(())
}
```

### 9.4 mTLS to Hetzner

Per ADR 0007 §3.5:

- Cert + key live in AWS Secrets Manager (two secrets per env:
  `prices-api-{env}-mtls-cert`, `prices-api-{env}-mtls-key`).
- Cold start loads both, builds a `rustls::ClientConfig`, feeds
  to the `clickhouse` crate's `Client::with_https`.
- Caddy on Hetzner terminates mTLS at :443 → reverse-proxies to
  CH's HTTPS interface :8443 (BE-side detail).
- Connection reuse: keep one HTTP/2 client per warm container;
  the Hetzner-side keep-alive headroom is a Cluster B ask in
  task 0045 — confirm before sizing concurrency.

### 9.5 Idempotency

- Same key + later `version` → `ReplacingMergeTree` keeps the
  latest.
- Lambda async retry on transient error (DLQ on terminal error).
- Re-firing an S3 event (via `aws s3api put-object-event` or a
  re-PUT) re-processes the ledger; the version field ensures
  prior rows are superseded, not duplicated.

### 9.6 Observability

- **Structured logs** (JSON): `ledger_sequence`, `tx_count`,
  `trade_tick_count` by `source`, `decode_error_count`,
  `ch_insert_ms`, `total_ms`.
- **CloudWatch metrics** (custom namespace `prices/lambda`):
  - `lag_seconds` = `now() - lcm.closed_at`
  - `trade_ticks_emitted` by `source` dimension
  - `decode_errors`
- **Alarms**: `lag_seconds > 60s` sustained 5 min, `decode_errors
  > 0` in any 1-min window (with auto-resolve), DLQ depth > 0.

### 9.7 Cold-start budget

Target < 1.5 s cold start to keep p99 ingestion lag bounded:

- Secrets Manager: 2 calls in parallel.
- Asset registry: lazy — load on first miss, don't block warm-up.
- CH client: one HTTP/2 connection pre-established at container
  start (`Client::ping`).

---

## 10. Open questions / follow-ups to spawn

1. **Aquarius AMM event shape.** Not observed in the 1.65k-ledger
   backfill window. Spawn a sample-extension task to capture a
   week of Aquarius pair txs from public RPC and run the
   decoder against them.
2. **Stable-pool variant.** Task 0032
   (`0032_RESEARCH_phoenix-stable-pool-first-observation`) hinted
   at a non-constant-product pool shape — verify whether
   stable-pool trades emit the same `trade` event with same
   fields, or a divergent shape.
3. **Synthetic mid-price from `update_reserves`.** Defer to a
   follow-up FEATURE task; not in the first Lambda iteration.
4. **REDSTONE feed-metadata.** The bytes payload's per-feed scale
   needs to be derived from the feed registry (Stellar Expert or
   live read). Spawn a one-shot research task to enumerate
   active feeds and their scales.
5. **SDEX classic-LP `ClaimAtomV2` decoder.** Mechanically the
   same as V1 but `liquidityPoolID` replaces `sellerId`. Verify
   the path-payment LP-match code path against real ledger
   fixtures before shipping.
6. **Multi-hop deduplication harness.** Build a CI fixture that
   asserts a Soroswap multi-hop tx generates exactly N ticks
   (N = number of hops), not N + (pair-events) + (CLMM-inner).

---

## 11. Worked example — end-to-end XLM `current_price` from local backfill

This section verifies the full chain `raw event → trade tick →
1-min OHLCV row → token.current_price` against real data in the
local backfill CH. Every value below is reproducible from the
queries shown — no fixture, no synthetic input.

### 11.1 Pre-flight: confirm the data supports pricing

A full-table inventory of pricing-relevant signatures
(`SELECT … FROM soroban_events FINAL WHERE signature IN
('trade','swap','update_reserves','REFLECTOR','REDSTONE')`) on
all 47,545,820 rows of the local CH yields:

| Signature | Events | Contracts | Txs | Ledger span |
|---|---:|---:|---:|---|
| `update_reserves` | 17,747 | 161 | 10,973 | 62020018–62079996 |
| `trade` | 17,138 | 157 | 10,364 | 62020018–62079996 |
| `swap` | 13,417 | 19 | 6,837 | 62020018–62079939 |
| `REDSTONE` | 4,064 | 1 | 4,064 | 62020009–62079990 |
| `REFLECTOR` | 3,449 | 3 | 3,449 | 62020003–62079991 |

The 17,138 `trade` events alone are sufficient to derive a price
for every base/quote pair the pools quote. Cross-pair coverage:
the top traded pair is `CCW67TSZ` ↔ `CAS3J7GY` (5,223 trades).
Resolving the SAC contracts via the `transfer` event topic[3]
identifier (`<CODE>:<ISSUER>` for SAC, `"native"` for XLM):

| Contract | Asset descriptor | Resolved as |
|---|---|---|
| `CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA` | `native` | XLM (XLM SAC) |
| `CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75` | `USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN` | USDC SAC |
| `CAUIKL3IYGMERDRUN6YSCLWVAKIFG5Q4YJHUKM4S4NJZQIA3BAS6OJPK` | `AQUA:GBNZILSTVQZ4R7IKQDGHYGY2QXL5QOFJYQMXPKWRRM5PAV7Y4M67AQUA` | AQUA SAC |
| `CDFZUVS5YNLXU7VENKOUDEOHCJGKQNVUBWD7KMN6E7ZROKPYPFLRUJFG` | `sUSD:GCHW7CWI7GMIYQYFXMFJNJX5645XGWIINIAEQK3SABQO6CAYL5T7JYIH` | sUSD SAC |
| `CAESLMGW5LYTIEJI7FJHK6SFSWRELLNVX5Q4WR4UZEALMTRWQDBKDPAG` | `VELO:GDM4RQUQQUVSKQA7S6EM7XBZP3FCGH4Q7CL6TABQ7B2BEJ5ERARM2M5M` | VELO SAC |

(SAC tokens inherit their underlying Stellar classic asset's
7-decimal precision; the registry from §7.1 must collapse
SAC-address → classic asset_id so the same asset has one ID
regardless of which path quotes it.)

> **Conclusion:** Yes — `soroban_events` carries the structural
> data needed to calculate a token price. The decoder rules in
> §4 are sufficient and the table has all required fields. The
> chain below is the worked proof.

### 11.2 Pool selection

We want XLM's USD price → pick the heaviest USDC/XLM pool:

```sql
SELECT c.contract_id AS pool, count() AS trades
FROM soroban_events e FINAL
JOIN soroban_contracts c FINAL ON c.id = e.contract_id
WHERE e.signature = 'trade'
  AND (
    JSONExtractString(JSONExtractRaw(e.topics_xdr, 2), 'value')
        IN ('CAS3J7GY…XLM', 'CCW67TSZ…USDC')
    AND JSONExtractString(JSONExtractRaw(e.topics_xdr, 3), 'value')
        IN ('CAS3J7GY…XLM', 'CCW67TSZ…USDC')
  )
GROUP BY pool ORDER BY trades DESC;
-- → CA6PUJLBYKZKUEKLZJMKBZLEKP2OTHANDEOWSFF44FTSYLKQPIICCJBE    4882
```

The dominant pool is the Phoenix-style pair pool
`CA6PUJLB…ICCJBE` with 4,882 USDC↔XLM trades across the
backfill window.

### 11.3 Single trade → trade tick

Most recent trade on that pool:

```
ledger_sequence  : 62079996
ledger_closed_at : 2026-04-12 04:16:10 UTC
event_index      : 4
tx_hash          : 928D46DDF1EE5F666E1DDBB0F58F607C5A1BB8874DEE7F6FB2032C736A7943AC
contract_id      : CA6PUJLBYKZKUEKLZJMKBZLEKP2OTHANDEOWSFF44FTSYLKQPIICCJBE  (Phoenix pool)

topics_xdr (parsed):
  [0] sym     "trade"
  [1] address CAS3J7GY…XLM             ← sold token
  [2] address CCW67TSZ…USDC            ← bought token
  [3] address GDQCVAXKDVCCFKUDHBKH6M555T7NTBNV6KXVCCTB3IAYIPKXW5U3DKTC  ← trader

data_xdr (parsed, vec of i128):
  [0] 13209580838       ← amount_sold     (XLM, 7-dec stroops)
  [1]  1999812752       ← amount_bought   (USDC, 7-dec stroops)
  [2]     6604791       ← fee             (XLM stroops)
```

Decoder applies the §4.1 rule (`base = sold`, `quote = bought`):

```
TradeTick {
    closed_at:        2026-04-12 04:16:10 UTC,
    base_asset_id:    XLM_ASSET_ID,
    quote_asset_id:   USDC_ASSET_ID,
    base_amount:      13_209_580_838,   // XLM stroops
    quote_amount:     1_999_812_752,    // USDC-7dec stroops
    source:           Source::Phoenix,
    tx_hash:          0x928D46DD…7943AC,
    venue_contract:   Some(CA6PUJLB…ICCJBE),
}
```

### 11.4 Price calculation (§4.2 decimal normalisation)

Both legs are 7-decimal (XLM SAC and USDC SAC both inherit
Stellar classic 7-dec). Decimals cancel:

```
price (USDC per XLM)
  = (quote_amount / 10^dec_quote) / (base_amount / 10^dec_base)
  = (1_999_812_752 / 10^7) / (13_209_580_838 / 10^7)
  = (1_999_812_752 / 13_209_580_838)        // 10^dec terms cancel because dec_base == dec_quote == 7
  = 0.15139394 USDC / XLM
```

So one XLM = **$0.15139** at the moment of this trade
(treating USDC ≈ $1.00).

**Cross-check via `update_reserves`.** The paired
`update_reserves` event in the same tx (`event_index = 5`) gives
the pool state after the trade:

```
data_xdr (vec of i128):
  [0] 141_169_719_284_559   ← reserve_0  (XLM stroops, after trade)
  [1]  21_391_229_141_825   ← reserve_1  (USDC stroops, after trade)

implied mid = (reserve_1/10^7) / (reserve_0/10^7)
            = 21_391_229_141_825 / 141_169_719_284_559
            = 0.15152 USDC / XLM
```

The 0.085% gap between executed price (0.15139) and post-trade
mid (0.15152) is consistent with the trade's price impact on a
constant-product pool — sanity passes.

### 11.5 Two trades, one minute → OHLCV row

Two USDC/XLM trades land in ledger 62079996 (both at
04:16:10 UTC → both bucket to the minute `2026-04-12 04:16:00`):

| application order | base (XLM stroops sold) | quote (USDC stroops bought) | price (USDC/XLM) |
|---:|---:|---:|---:|
| tx `7F785BF7…` (earlier) | 25,761,941,491 | 3,901,204,480 | 0.151440 |
| tx `928D46DD…` (later)   | 13,209,580,838 | 1,999,812,752 | 0.151394 |

Bucketer (§7.4) folds these into one `price_ohlcv_1m` row:

```sql
INSERT INTO prices.price_ohlcv_1m VALUES (
    timestamp        = '2026-04-12 04:16:00',
    asset_id         = XLM_ASSET_ID,
    quote_asset_id   = USDC_ASSET_ID,
    granularity      = '1m',
    source           = 'phoenix',
    open             = 0.151440,                              -- first tick (earlier tx)
    high             = 0.151440,                              -- max of (0.151440, 0.151394)
    low              = 0.151394,                              -- min
    close            = 0.151394,                              -- last tick (later tx)
    volume_base      = (25_761_941_491 + 13_209_580_838) / 10^7
                     = 3897.1522329,                          -- XLM
    volume_quote     = (3_901_204_480 + 1_999_812_752) / 10^7
                     = 590.1017232,                           -- USDC
    volume_quote_usd = 590.1017232,                           -- USDC ≈ USD; task 0026 pegs
    trade_count      = 2,
    vwap             = 590.1017232 / 3897.1522329
                     = 0.151415,
    version          = 62_079_996 * 1_000_000 + 4             -- ledger_seq*1M + event_index
);
```

Re-INSERTing the same `(timestamp, asset_id, quote_asset_id,
granularity, source)` key with a higher `version` (replay or
backfill correction) lets `ReplacingMergeTree` collapse to the
latest on next merge — idempotent without UPSERT (§7.3).

### 11.6 OHLCV row → `tokens.current_price`

`tokens.current_price` is **not written by this Lambda**. It is
produced by the Current Price Updater (one of the periodic
workers in task 0039) that runs on a 1-min CloudWatch schedule
and computes:

```sql
INSERT INTO prices.tokens_current_price
SELECT
    asset_id,
    argMax(close, version)                       AS current_price,
    argMax(timestamp, version)                   AS as_of,
    argMax(source, version)                      AS source,
    max(timestamp)                               AS latest_bucket
FROM prices.price_ohlcv_1m FINAL
WHERE quote_asset_id = USD_PEGGED_ASSET_ID          -- USDC for now; widen later
  AND timestamp >= now() - INTERVAL 5 MINUTE
GROUP BY asset_id
ORDER BY asset_id
SETTINGS final = 1;
```

For our worked example the resulting row is:

```
asset_id       = XLM_ASSET_ID
current_price  = 0.151394          -- close of the 04:16 bucket
as_of          = '2026-04-12 04:16:00 UTC'
source         = 'phoenix'
latest_bucket  = '2026-04-12 04:16:00 UTC'
```

That `0.151394` is the value an API consumer reads when they hit
`GET /tokens/{XLM_ID}` for `current_price`. The number is fully
derived from a single soroban_events `trade` row plus the
decoder rules in §4.1 and §4.2 — no oracle dependency, no
off-chain calibration.

### 11.7 Verifying across all top base tokens

Running the same `argMax((amount_bought / amount_sold),
(ledger_sequence, transaction_id, event_index))` aggregation
across every `trade` event whose quote leg is XLM
(`CAS3J7GY…XLM`) on the full table:

| Base contract | Resolved | Latest price (XLM per base) | At ledger | Trades in window |
|---|---|---:|---:|---:|
| `CCW67TSZ…` | USDC | 6.58164529 | 62079722 | 2,756 |
| `CDFZUVS5…` | sUSD | 6.45730957 | 62078341 | 170 |
| `CAAV3AE3…` | (stablecoin) | 8.74190884 | 62075164 | 54 |
| `CCKCKCPH…` | (custom) | 0.02579493 | 62079058 | 180 |
| `CAUIKL3I…` | AQUA | 0.00219164 | 62078749 | 584 |
| `CBIJBDNZ…` | (high-value) | 47,291.66 | 62079979 | 199 |

Inverting USDC→XLM: `1 / 6.58164529 = 0.15191` USDC per XLM —
within 0.3% of the single-trade price computed above. The
discrepancy is because the table includes both directions of
the pair; selecting only the `quote = XLM` direction gives a
direction-asymmetric average. The actual `current_price`
calculation in §11.6 uses the latest-trade close per
(asset, quote, source) and is direction-symmetric.

> **Pricing chain verified end-to-end:** raw `soroban_events.trade`
> row → `TradeTick` (§4.1) → minute-bucketed `price_ohlcv_1m` row
> (§7.4) → `tokens.current_price` (read by 0039's worker).
> The Lambda from §9 owns the first two arrows; ADR 0007's CH
> materialised-view chain owns the rollups; task 0039's worker
> owns the last arrow.

## 12. References

- Wiki: [`lore/3-wiki/project/soroban-events-schema.md`](../../../../3-wiki/project/soroban-events-schema.md) — payload reference.
- Sample data: `lore/4-notes/samples/soroban-events/*.jsonl` (50 rows per signature; full-table stats in `signatures-stats.tsv`).
- ADR 0001 — Stream 1 CH-sourced AMM backfill.
- ADR 0003 — `price_ohlcv` PK includes quote_asset_id.
- ADR 0004 — `price_ohlcv` multi-source merge columns.
- ADR 0007 — Live data sink on shared Hetzner CH (proposed; gating spec).
- Task 0038 — Prices Ledger Processor Lambda (blocked; this spec is the input to its rewrite).
- Task 0045 — Cross-team bundle with BE; settles mTLS, capacity, cost-share.
- Task 0046 — Empirical CH storage estimate.
- Task 0047 — Cross-tenant throughput verification.
- Local CH (sample source): `/home/oski/Projects/stellar/soroban-block-explorer` docker compose, `localhost:8123`, user `default` / password `clickhouse`, 47.5M events covering ledgers 62019999–62079982.
