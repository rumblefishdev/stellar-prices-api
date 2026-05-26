---
title: 'SDEX per-variant decode + pair canonicalisation + price math + 1m bucket UPSERT contract'
type: generation
status: mature
spawned_from: ../README.md
spawns: []
tags: [sdex, decode, claim-atom, ohlcv, bucket, upsert, spec, stream-2]
links:
  - '../README.md'
  - './G-sdex-filter-strategy.md'
  - '../../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/R-sdex-operation-xdr-shape.md'
  - '../../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/G-sdex-trade-extraction-design.md'
  - '../../../2-adrs/0002_stream2-sdex-archive-backfill-independent-of-be.md'
  - '../../../../docs/database-schema/database-schema-overview.md'
  - '../../../../docs/prices-api-general-overview.md'
  - './profile/examples/order_book.json'
  - './profile/examples/liquidity_pool.json'
  - './profile/examples/manage_offer_multi_claim.json'
history:
  - date: 2026-05-13
    status: mature
    who: okarcz
    note: >
      Closes task 0022 points (2) pair canonicalisation, (3) per-variant
      decode, (4) price + precision, (5) 1m bucket UPSERT contract.
      Worked examples sourced from real mainnet ledgers via the
      profile harness; V0 synthesized from protocol XDR spec.
---

# SDEX per-variant decode + pair canonicalisation + price math + 1m bucket UPSERT contract

This note pins the **extract + bucket** half of the SDEX backfill
contract. The filter + control-plane half is in
[G-sdex-filter-strategy](./G-sdex-filter-strategy.md). Both notes
are scoped so that task 0012's Rust submodules map 1:1 onto
sections here.

## TL;DR

| Concern                    | Decision                                                                                                                                                                                                                                                                           |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pair canonicalisation      | Quote-asset preference (USDC, USDT, native XLM in that order); lexicographic fallback on `(asset_type, asset_code, issuer_str_key)` for everything else. Stored in `asset_pairs.canonical_base_id / canonical_quote_id` once, reused.                                              |
| `ClaimAtom` variant decode | All three variants (V0 / ORDER_BOOK / LIQUIDITY_POOL) carry the same trade-shaped fields. Counterparty differs (informational, preserved). One decoder; per-variant pattern match.                                                                                                 |
| Price math                 | `price = amount_bought / amount_sold` in (bought-units-per-sold-unit), inverted as needed to match canonical pair orientation. `NUMERIC(28,14)` precision. Both amounts are stroops (10⁻⁷); divide by 10⁷ to normalise. Skip claim entirely on `amount_sold == 0`.                 |
| Backfill bucket UPSERT     | **Whole-row replacement** (matches schema doc L362–365). Aggregate one minute's trades **in memory** per `(canonical_pair_id, minute)`, write the completed 1m candle once per minute per pair, `ON CONFLICT (timestamp, asset_id, granularity) DO UPDATE SET col = EXCLUDED.col`. |
| Volume in USD              | `volume_base` is authoritative (sum of base-side stroops normalised). `volume_quote_usd` left to a downstream enrichment pass when the quote-side USD reference is available; backfill writes 0 if unknown.                                                                        |

## 1. The five SDEX-relevant op results

Restating task 0020's R-note §1 for self-containedness; the
extractor patterns directly on this:

| `OperationResultTr` variant            | `success.{field}` carrying claim atoms |
| -------------------------------------- | -------------------------------------- |
| `ManageSellOffer(Success(_))`          | `offers_claimed: VecM<ClaimAtom>`      |
| `ManageBuyOffer(Success(_))`           | `offers_claimed: VecM<ClaimAtom>`      |
| `CreatePassiveSellOffer(Success(_))`   | `offers_claimed: VecM<ClaimAtom>`      |
| `PathPaymentStrictReceive(Success(_))` | `offers: VecM<ClaimAtom>`              |
| `PathPaymentStrictSend(Success(_))`    | `offers: VecM<ClaimAtom>`              |

The field is named `offers_claimed` for manage-offer-shaped results
and `offers` for path-payment-shaped results; otherwise the slice
element type is identical (`ClaimAtom`). All five funnel into the
same extractor.

## 2. Pair canonicalisation

A `ClaimAtom` is directional: `assetSold → assetBought`. The
extractor sees both perspectives of the trade per claim. For
OHLCV, we need a canonical `(base, quote)` orientation per pair so
that prices and volumes accumulate consistently across claims that
go in either direction (Alice sells XLM for USDC, Bob sells USDC
for XLM — same pair, opposite directions, must aggregate
together).

### 2.1 Asset 4-tuple canonical identity

(Restated from task 0020 G-note §3.4.)

```rust
#[derive(Hash, PartialEq, Eq, Clone, Debug)]
enum AssetIdentity {
    Native,                              // XLM
    Credit { code: SmolStr, issuer: AccountId },
}
```

Maps 1:1 to the `assets` table 4-tuple `(asset_type, asset_code,
issuer, contract_address)`. SDEX claims only ever reference
`Native | Credit` (never `PoolShare`, never SAC `Contract` — see
filter spec §3.3); the decoder may treat `PoolShare` / `Contract`
asset types in claim atoms as a hard error (XDR spec says these
are not valid in trade contexts).

### 2.2 Pair canonicalisation rule

Per pair (set `{a, b}`), pick one as base and one as quote, once,
deterministically, and cache the choice in `asset_pairs`:

```text
canonicalise(a, b) -> (base, quote):
    quote_preference = [
        USDC: GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN,
        USDT: GCQTGZQQ5G4PTM2GL7CDIFKUBIPEC52BROAQIAPW53XBRJVN6ZJVTG6V,
        Native,
    ]
    for q in quote_preference:
        if a == q: return (b, a)
        if b == q: return (a, b)
    # Neither side is a preferred quote — lexicographic on the
    # 4-tuple key. Lower one is base, higher is quote.
    if key(a) < key(b): return (a, b)
    else:                return (b, a)

key(asset):
    asset_type   ||   asset_code   ||   issuer_str_key   (joined as bytes)
    Native -> "\x00" || "" || ""
```

The USDC / USDT issuer addresses are Circle's and Tether's mainnet
issuers. Hard-coded today; if more stable quote candidates are
adopted, extend the list in priority order and prepend / re-sort.
(Existing pairs keep their cached canonical orientation; new pairs
pick from the new list.)

### 2.3 What `asset_pairs` stores

```sql
CREATE TABLE asset_pairs (
    id           SERIAL PRIMARY KEY,
    base_id      INT NOT NULL REFERENCES assets(id),
    quote_id     INT NOT NULL REFERENCES assets(id),
    CONSTRAINT asset_pair_ordered CHECK (base_id < quote_id),  -- not literal: orientation by quote-preference logic, not surrogate ordering. The CHECK is on `LEAST(base,quote) < GREATEST(...)` shape — pick the form that fits.
    UNIQUE (base_id, quote_id)
);
```

(Schema is illustrative; the prices-api schema doc may evolve the
exact form. The semantic is one row per `{a, b}` unordered pair,
with `base_id`/`quote_id` carrying the canonical orientation.)

`asset_pairs.id` becomes the surrogate key the OHLCV writer uses
to bucket trades. Note: `price_ohlcv.asset_id` is the **base
asset's** id per the existing schema, not a pair id. That means
the schema as documented today **doesn't capture the quote choice
on the row itself** — the convention is "asset_id is the base; the
quote is implied by `source` plus the asset's natural pair". For
SDEX trades this is ambiguous when an asset trades against
multiple quotes (e.g. USDC/XLM and USDC/USDT). **This is a
real schema gap that this spec surfaces; resolution is task 0012's
call** (one of: add `quote_asset_id` to `price_ohlcv`, or use a
separate `asset_pair_id` surrogate). See §6.

For the rest of this spec, treat the OHLCV target row as
`(asset_id == canonical_base.asset_id, timestamp, granularity)` and
flag the quote-side ambiguity for task 0012 to resolve.

## 3. Per-`ClaimAtom`-variant decode

All three variants carry the same `(asset_sold, amount_sold,
asset_bought, amount_bought)` trade-shaped fields. They differ
only in counterparty metadata. The decoder pattern-matches on the
variant and emits a uniform `TradeTick` (task 0020 G-note §"Output
unit"):

```rust
struct TradeTick {
    ledger_sequence:  i64,
    closed_at:        chrono::DateTime<Utc>,
    transaction_hash: [u8; 32],
    operation_index:  u16,           // 0-based within the tx
    claim_index:      u16,           // 0-based within the op's claim slice

    asset_sold:       AssetIdentity,
    amount_sold:      Decimal128_7,  // stroops, normalised to 7-decimal
    asset_bought:     AssetIdentity,
    amount_bought:    Decimal128_7,

    counterparty:     TradeCounterparty,
}

enum TradeCounterparty {
    OrderBook    { seller_id: AccountId,         offer_id: i64 },
    LiquidityPool { pool_id: [u8; 32] },
    V0Legacy     { seller_ed25519_pk: [u8; 32], offer_id: i64 },
}
```

`counterparty` is informational (filtering / debugging /
post-hoc analytics). Price math does not consult it. The decoder
writes it through verbatim from the XDR.

### 3.1 ORDER_BOOK (modern, ≥ protocol 18) — worked example

Source: real ledger 62 442 947 (mainnet, 2026-05-02), op
`ManageSellOffer`, 4-deep book walk on XLM → SCOP. Full XDR via
[`profile/examples/manage_offer_multi_claim.json`](./profile/examples/manage_offer_multi_claim.json).

The atom (claim 0 of 4):

```json
{
  "variant": "order_book",
  "seller_id": "GDT4MRDHYOLKYDYDZTTIGMB6NLN6ESEVG3ON6T3JLYKKEGQIJ3CMNAUE",
  "offer_id": 1837081240,
  "asset_sold": { "type": "native" },
  "amount_sold_stroops": 22423,
  "asset_bought": {
    "type": "credit_alphanum4",
    "code": "SCOP",
    "issuer": "GC6OYQJIZF3HFXCYPFCBXYXNGIBQ4TNSFUBUXQJOZWIP6F3YZK4QH3VQ"
  },
  "amount_bought_stroops": 1648943
}
```

Decoder maps to `TradeTick`:

```rust
TradeTick {
    ledger_sequence:  62_442_947,
    closed_at:        2026-05-02T15:24:05Z,  // close_time_unix 1_778_061_845
    transaction_hash: 0x1f65_e3af_…,         // 1f65e3affebb…
    operation_index:  0,
    claim_index:      0,

    asset_sold:    AssetIdentity::Native,
    amount_sold:   Decimal128_7::from_stroops(22_423),         // = 0.0022423
    asset_bought:  AssetIdentity::Credit {
        code: "SCOP",
        issuer: "GC6OYQJIZF3HFXCYPFCBXYXNGIBQ4TNSFUBUXQJOZWIP6F3YZK4QH3VQ"
    },
    amount_bought: Decimal128_7::from_stroops(1_648_943),      // = 0.1648943

    counterparty:  TradeCounterparty::OrderBook {
        seller_id: "GDT4MRDHYOLKYDYDZTTIGMB6NLN6ESEVG3ON6T3JLYKKEGQIJ3CMNAUE",
        offer_id:  1_837_081_240,
    },
}
```

Pair canonicalisation: neither side is USDC/USDT; Native is in the
quote-preference list → SCOP is base, XLM is quote.

Price (SCOP per XLM, since SCOP is base and XLM is quote, and the
canonical direction is "how much quote per one base"):

```text
price_quote_per_base = amount_quote / amount_base
                    = amount_sold (XLM) / amount_bought (SCOP)
                    = 22_423 / 1_648_943
                    = 0.013598… XLM per SCOP
```

But the natural read direction in the atom is **inverted** for
this trade: the op submitter _sold XLM, bought SCOP_. The
canonical pair is (base=SCOP, quote=XLM). So price is
`amount_sold (XLM) / amount_bought (SCOP)` = how much XLM was paid
per SCOP received.

`price_ohlcv` row delta for this claim (preliminary — see §4 for
multi-claim aggregation):

```text
asset_id      = id(SCOP)
timestamp     = 2026-05-02T15:24:00Z         (minute bucket)
granularity   = '1m'
open          = 0.013598…   (first observation in this minute)
high          = 0.013598…
low           = 0.013598…
close         = 0.013598…
volume_base   = 0.1648943   (SCOP)
volume_quote_usd = 0        (no USD reference available at backfill time — §5.2)
vwap          = 0.013598…
trade_count   = 1
source        = 'sdex'
```

The full op produces four claim atoms, all at essentially the
same price (~0.013598 XLM/SCOP) — the offer walked through 4
sequential maker orders at the same price level. The merged 1m
bucket aggregates all four (see §4).

### 3.2 LIQUIDITY_POOL — worked example

Source: real ledger 62 435 141, op `PathPaymentStrictSend`. Full
XDR via [`profile/examples/liquidity_pool.json`](./profile/examples/liquidity_pool.json).

```json
{
  "variant": "liquidity_pool",
  "pool_id_hex": "2d6442c3d3c0eeec077f3ef9054bf26ca4eac7ea5c53063dcf3a6dc45f08fc14",
  "asset_sold": {
    "type": "credit_alphanum12",
    "code": "3qualiT",
    "issuer": "GAVVNJKEM4XFXBPYITFCDVKOZRI3PAXJHDH666MDQIXJAJ4H7HO3722C"
  },
  "amount_sold_stroops": 1200406,
  "asset_bought": {
    "type": "credit_alphanum12",
    "code": "aTTaiN",
    "issuer": "GDKS7XTNEVCPGVUT2ZPPOU5CHHF3NH6NX7P3ROSFSDAO6NQDS3C74Y6C"
  },
  "amount_bought_stroops": 10000000
}
```

Decoder maps to `TradeTick` with `counterparty = LiquidityPool {
pool_id: 0x2d64_42c3_…}` and the same `(asset_sold, amount_sold,
asset_bought, amount_bought)` fields. The decode pattern is
**identical** to ORDER_BOOK; only the `counterparty` variant
differs.

Pair canonicalisation: neither side is in quote-preference list →
lexicographic fallback. Lower key wins as base. Both are
`credit_alphanum12`, codes are "3qualiT" vs "aTTaiN" — code-byte
lex compare: '3' (0x33) < 'a' (0x61), so "3qualiT" is base.

Price (aTTaiN per 3qualiT):

```text
price = amount_bought / amount_sold = 10_000_000 / 1_200_406
      = 8.3305…
```

The classic LP price is reproducible from `(reserve_a, reserve_b)`
in the pool state; this `ClaimAtom` is the _executed_ trade and is
what we record for OHLCV. Pool-state reconstruction is out of
scope for this backfill (it lives in BE's `liquidity_pools` table).

### 3.3 V0 (legacy, ≤ protocol 17) — synthesized example

V0 does not appear in the modern sample range. Per protocol XDR
([R-note §3](../../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/R-sdex-operation-xdr-shape.md#3-claimatom--the-trade-tick-unit)), the only structural difference is the
counterparty field: V0 stores the seller's raw 32-byte ed25519
public key (`uint256 sellerEd25519`) instead of the modern
`AccountID`. Trade-shaped fields are identical.

Synthesized example, modelled on a typical 2018-vintage classic
XLM/USDC trade:

```json
{
  "variant": "v0",
  "seller_ed25519_hex": "5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b",
  "offer_id": 12345678,
  "asset_sold": { "type": "native" },
  "amount_sold_stroops": 1000000000,
  "asset_bought": {
    "type": "credit_alphanum4",
    "code": "USDC",
    "issuer": "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN"
  },
  "amount_bought_stroops": 25000000
}
```

Decode maps to `TradeTick` with `counterparty =
TradeCounterparty::V0Legacy { seller_ed25519_pk:
0x5a6b_…, offer_id: 12_345_678 }`. To present V0 counterparty info
through any UI / API, encode the ed25519 bytes as a G… StrKey
(version byte 6 << 3, payload, CRC16-XModem checksum) — same
StrKey transformation the modern `AccountID::PublicKey` round-trips
through.

Pair canonicalisation: USDC is in the quote-preference list (rank

1. → USDC is quote, XLM is base.

Price (USDC per XLM, quote_per_base):

```text
amount_quote = amount_bought = 25_000_000 stroops = 2.5 USDC
amount_base  = amount_sold   = 1_000_000_000 stroops = 100 XLM
price        = 2.5 / 100 = 0.025 USDC per XLM
```

Decoder support for V0 is required for early-history backfill;
extractor cost is negligible (one extra match arm).

## 4. Price math + precision

### 4.1 Amount normalisation

All `int64 amount_*` are stroops. Convert to a 7-decimal fixed
type before any price math:

```rust
let units: Decimal128 = Decimal128::from(amount_stroops_i64) / Decimal128::from(10_000_000_i64);
```

Equivalently, `NUMERIC(28,14)` in PG with explicit `/ 10^7`. Never
floating-point. `NUMERIC(28,14)` provides 14 fractional digits
which preserves stroop-level precision (7 digits) and also keeps
~7 digits of ratio precision after division — see §4.3.

### 4.2 Price formula

```text
price_quote_per_base =
    if asset_sold == canonical_base:
        amount_bought / amount_sold       # natural direction
    else:
        amount_sold / amount_bought       # invert
```

Computed in `Decimal128`/`NUMERIC(28,14)` arithmetic. Result is
the unit-price of base in quote terms — the value stored in
`price_ohlcv.open/high/low/close`.

### 4.3 Precision considerations

Both amounts are 7-decimal stroops. The ratio
`amount_bought / amount_sold` carries up to 14 fractional digits
of meaningful precision (worst case: `amount_bought = 9e18`
stroops, `amount_sold = 1` stroop produces a price up to ~1e19 —
still inside `NUMERIC(28,14)`'s headroom).

Real Stellar amounts are bounded by `int64::MAX ≈ 9.2e18` stroops
≈ `9.2e11` whole units. For the canonical case
(amount_base ~ 1e9 stroops, amount_quote ~ 1e7 stroops), the ratio
is ~1e-2 with both numerator and denominator below 1e10 — well
inside double-precision (but we don't use double).

### 4.4 Edge cases

| Case                        | Spec                                                                                                                                        |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `amount_sold == 0`          | Reject claim (log warning, increment a counter, skip). Should never happen on-chain (protocol-rejected pre-application) but defend in case. |
| `amount_bought == 0`        | Same: reject + log. Same protocol invariant.                                                                                                |
| Asset type 3 (`POOL_SHARE`) | Hard error. SDEX trades never reference pool shares. Log at error level and abort the ledger (corrupt XDR).                                 |
| Asset type 4+ (`CONTRACT`)  | Same: hard error. SAC contracts don't appear in SDEX `ClaimAtom`s.                                                                          |
| `seller_id` is muxed        | Doesn't happen — `ClaimAtom`'s seller is `AccountID` (not `MuxedAccount`). No special handling.                                             |

## 5. 1m bucket UPSERT contract

The backfill aggregates trades into 1-minute candles **in
memory**, then UPSERTs one row per `(canonical_pair, minute)` to
`price_ohlcv` per minute. This is "whole-row replacement" mode
per [schema doc L362–365](../../../../docs/database-schema/database-schema-overview.md). The spec below is the in-memory aggregation
algorithm + the SQL write.

### 5.1 In-memory aggregation

For each ledger, group its `TradeTick`s by `(canonical_pair,
minute)` where `minute = floor(closed_at / 60s)`. Maintain an
in-memory map:

```rust
struct CandleAccumulator {
    // Identity
    asset_id_base:  AssetId,     // canonical-base surrogate id from `assets`
    minute_start:   DateTime<Utc>,

    // OHLCV state
    open_price:     Decimal128,  // price of first tick in lowest (ledger, op, claim) lex order
    open_lex:       (i64, u16, u16),  // (ledger, op_idx, claim_idx) for tie-breaking
    close_price:    Decimal128,
    close_lex:      (i64, u16, u16),
    high_price:     Decimal128,
    low_price:      Decimal128,

    volume_base:    Decimal128,  // sum of base-side amounts
    volume_quote:   Decimal128,  // sum of quote-side amounts (NOT volume_quote_usd — see §5.2)
    trade_count:    u32,
}

impl CandleAccumulator {
    fn merge(&mut self, tick: &TradeTick, price: Decimal128) {
        let lex = (tick.ledger_sequence, tick.operation_index, tick.claim_index);
        if lex < self.open_lex { self.open_price = price; self.open_lex = lex; }
        if lex > self.close_lex { self.close_price = price; self.close_lex = lex; }
        self.high_price = self.high_price.max(price);
        self.low_price  = self.low_price.min(price);

        let (base_amt, quote_amt) = tick.canonical_amounts();
        self.volume_base  += base_amt;
        self.volume_quote += quote_amt;
        self.trade_count  += 1;
    }
}
```

The `(ledger, op_idx, claim_idx)` lex tuple is the canonical
order-of-occurrence within a minute and resolves the
"which tick is open / close" question deterministically. Two
claims in the same `(ledger, op)` are ordered by `claim_idx` (the
order they appear in `offers_claimed`/`offers`), which is the
protocol's execution order.

### 5.2 When to flush

A candle is **complete** when the backfill has processed a ledger
whose `closed_at` is in the next minute. Flush rule:

```text
on each ledger L processed:
    cur_min = floor(L.closed_at / 60s)
    for each accumulator A in memory where A.minute_start < cur_min:
        flush(A) to price_ohlcv     # one UPSERT per (asset_id, minute) pair
        drop A from memory
    # in-flight accumulators with A.minute_start == cur_min remain
```

At end-of-stream (target_ledger processed), flush all remaining
in-memory accumulators.

Memory bound: ~10–12 ledgers worth of trades per in-flight minute,
across all active pairs. The 2 000-ledger profile saw a max of
396 claim-atoms in one ledger (P95 = 182), and recent mainnet
ledger close time is ~5–6s so one minute holds ~10–12 ledgers
≈ a few thousand atoms at the high end. Distinct pairs per minute
≈ a few hundred. Memory budget is bounded by a few MB.

### 5.3 UPSERT SQL

```sql
INSERT INTO price_ohlcv (
    asset_id, timestamp, granularity,
    open, high, low, close,
    volume_base, volume_quote_usd, vwap, trade_count, source
)
VALUES ($1, $2, '1m', $3, $4, $5, $6, $7, 0, $8, $9, 'sdex')
ON CONFLICT (timestamp, asset_id, granularity) DO UPDATE SET
    open             = EXCLUDED.open,
    high             = EXCLUDED.high,
    low              = EXCLUDED.low,
    close            = EXCLUDED.close,
    volume_base      = EXCLUDED.volume_base,
    volume_quote_usd = EXCLUDED.volume_quote_usd,
    vwap             = EXCLUDED.vwap,
    trade_count      = EXCLUDED.trade_count,
    source           = EXCLUDED.source;
```

`vwap` is computed in-memory before the write:
`vwap = volume_quote / volume_base` (in _quote-asset units_, not
USD — pre-enrichment).

`volume_quote_usd` is **0** until a downstream pass enriches it:

- For pairs with `quote = USDC` or `quote = USDT`, the
  enrichment is the identity: `volume_quote_usd = volume_quote *
oracle_price_of_quote_in_usd_at_minute`. With USDC/USDT pegged
  to ~$1 the multiplier is ~1.
- For pairs with `quote = native XLM`, the enrichment requires
  `oracle_price_of_XLM_in_USD_at_minute`. The Oracle Fetcher
  Lambda's `oracle_prices` table is the source.
- Two-hop pairs (e.g. SCOP/XLM, where neither side is USD-pegged)
  resolve via the XLM/USD oracle.

The enrichment pass is a follow-up concern; this spec leaves
`volume_quote_usd = 0` from the backfill writer and surfaces a
spawned task to implement the enrichment (see §6).

### 5.4 Why whole-row replacement (not incremental merge) for backfill

The task 0022 README point (5) initially asked for incremental
merge ("preserve open (lowest ledger-time wins within the minute),
overwrite close (highest ledger-time wins), GREATEST(high) / LEAST(low),
sum volumes and `trade_count`, recompute `vwap`"). That contract
is the _live ingestion_ contract (one ledger at a time, per
S3-event) and is documented as such in
[database-schema-overview L362](../../../../docs/database-schema/database-schema-overview.md).

Backfill writes are different: the backfill task aggregates **the
whole minute** in memory and writes the finished candle once.
That matches the schema doc's row for `SDEX Backfill`:

> **Whole-row replacement.** The backfill task aggregates all
> SDEX trades for one historical minute in memory and writes the
> finished candle once.

Trade-offs:

| Aspect            | Incremental (live-ingestion style)     | Whole-row (backfill style — **this spec**)   |
| ----------------- | -------------------------------------- | -------------------------------------------- |
| Writes per minute | ~10–12 (one per contributing ledger)   | 1 (per pair)                                 |
| Convergence proof | Requires UPSERT-merge formula audit    | Trivial (in-memory aggregation is just code) |
| Crash semantics   | Partial candle in DB; resume re-merges | Lost in-memory state; resume re-aggregates   |
| Memory pressure   | None (no in-flight state)              | A few MB (bounded — see §5.2)                |
| DB write rate     | 10× higher                             | 1× baseline                                  |

Whole-row is the right choice for backfill because:

1. The schema doc commits to it.
2. Backfill is single-task (per §5.6), not event-driven; per-event
   writes are not needed.
3. The memory budget is trivially bounded.
4. Restart-from-checkpoint produces identical writes — see §5.5.

### 5.5 Restart-from-checkpoint convergence

On crash, the in-memory accumulators are lost. Resume restarts
from `current_ledger + 1` (per filter spec §2). If the lost
in-memory minute had already been partially flushed (improbable —
flush happens at minute boundaries, not mid-minute — but possible
if the crash happened during a flush write), the next resume
re-aggregates the same trades from the same ledgers and emits an
identical row via the same UPSERT-replace clause. Idempotent.

The one edge case: if the **target_ledger** was advanced
mid-minute and the backfill crashed before flushing the final
in-flight minute, that minute's candle is lost at first. But the
catch-up loop (filter spec §2.4) re-reads the same ledgers on
restart, accumulates the same trades, and flushes correctly. Net:
no data loss as long as ledgers are re-readable, which they are
(archives are immutable).

### 5.6 Multiple-source aggregation

A 1-minute candle for asset_id X may be written by multiple
sources: SDEX backfill (this spec), Soroban AMM backfill (Stream
1, separate spec), live Prices Ledger Processor (live mode). All
write to the same row `(X, minute, '1m')`.

Per schema doc §"Source attribution":

> When the same `(timestamp, asset, granularity)` is written by
> multiple distinct sources … the writer uses `source =
'aggregated'` and merges across sources. Single-source candles
> keep their original source label.

For the backfill: write `source = 'sdex'` on initial write. The
"aggregation across sources" step is a downstream concern owned
by the Current Price Updater (§5.3) or a dedicated aggregator —
not the backfill. The backfill claims the row as SDEX-source;
follow-up runs from other sources are expected to:

1. Detect the existing `source = 'sdex'` row on conflict.
2. Up-aggregate by reading the existing values, merging with its
   own minute aggregation, and writing back `source = 'aggregated'`.

This needs to be specced separately for the live writers (it's
not a backfill concern); flag in §6.

## 6. Open items + follow-up tasks

This spec surfaces three concrete follow-up needs. These should
spawn backlog tasks (numbers TBD):

1. **OHLCV row identity: base-only vs base+quote.** Schema today
   keys on `(timestamp, asset_id, granularity)` where `asset_id`
   is the base. Pairs like USDC/XLM and USDC/USDT both map to
   `asset_id = USDC.id` and would collide. Either:
   - add `quote_asset_id INT NOT NULL` to the PK,
   - or use `asset_pair_id` as the row key.
     This is **load-bearing for SDEX correctness** (an asset
     trading against multiple quotes is normal). Decision goes in
     a schema-change ADR; backfill spec applies after it lands.

2. **`volume_quote_usd` enrichment.** §5.3 above leaves it 0.
   A follow-up pass (likely a function of the Current Price
   Updater, or a dedicated backfill enrichment task) reads
   `oracle_prices` for the quote asset at minute-bucketed
   resolution and computes `volume_quote_usd = volume_quote *
oracle_price`. Until then, `current_prices.volume_24h_usd`
   computed from SDEX-only candles is undercounted by the
   non-USD-quoted pair fraction.

3. **Multi-source merge contract.** §5.6 describes the
   detect-then-merge pattern but the _live_ contract for "I see
   `source = 'sdex'`, I'm Stream 1, I merge in my own values
   without losing SDEX's contribution" needs a separate spec
   (lives outside the backfill, in the live writer path).

## 7. Mapping to task 0012 Rust modules

```text
crate::sdex_backfill::filter        (filter spec §1 — TxSuccess gate, OpResultTr discriminant)
crate::sdex_backfill::checkpoint    (filter spec §2 — per-ledger BEGIN/COMMIT)
crate::sdex_backfill::asset_resolve (filter spec §3 — insert-on-fly + LRU)
crate::sdex_backfill::extract       (this spec §1 §3 — walk + per-variant decode)
crate::sdex_backfill::canonicalise  (this spec §2 — pair orientation)
crate::sdex_backfill::price         (this spec §4 — Decimal128 math, edge cases)
crate::sdex_backfill::bucket        (this spec §5 — in-memory aggregator + UPSERT)
```

Each module is gradeable clause-by-clause against the
section of this spec (or filter spec) cited in parentheses.
