---
title: "SDEX operation XDR shape — what an SDEX op looks like after parsing"
type: research
status: mature
spawned_from: ../README.md
spawns: []
tags: [sdex, xdr, stellar-protocol, ledger-close-meta, claim-atom]
links:
  - "https://github.com/stellar/stellar-xdr"
  - "https://developers.stellar.org/docs/learn/encyclopedia/sdex/liquidity-on-stellar-sdex-liquidity-pools"
history:
  - date: 2026-05-12
    status: mature
    who: okarcz
    note: "Distilled from the Stellar protocol XDR spec (current at protocol 22)."
---

# SDEX operation XDR shape

**Answers user question 1:** *how does an SDEX operation look after
parsing from XDR?*

Headline: an SDEX-relevant operation has two distinct XDR shapes —
the **operation input** (what the submitter asked for) and the
**operation result** (what actually happened). For token-price
calculation, the **result is what matters**: the operation input
tells you intent, the result tells you which trades executed at
which prices. The result lives inside the ledger's
`TransactionResult` envelope, which travels in archive
`LedgerCloseMeta` objects.

## 1. Which ops are "SDEX-relevant"

Five operation types produce on-chain trades:

| OpType | Numeric | Result wrapper | Trades land in |
|--------|---------|----------------|----------------|
| `MANAGE_SELL_OFFER` | 3 | `ManageSellOfferResult` | `success.offersClaimed<>` |
| `CREATE_PASSIVE_SELL_OFFER` | 4 | `ManageSellOfferResult` (shared) | `success.offersClaimed<>` |
| `MANAGE_BUY_OFFER` | 12 | `ManageBuyOfferResult` | `success.offersClaimed<>` |
| `PATH_PAYMENT_STRICT_RECEIVE` | 2 | `PathPaymentStrictReceiveResult` | `success.offers<>` |
| `PATH_PAYMENT_STRICT_SEND` | 13 | `PathPaymentStrictSendResult` | `success.offers<>` |

All five funnel matched-trade information through a single union:
**`ClaimAtom`**. One match (an offer eaten, fully or partially) = one
`ClaimAtom` entry. One op can produce 0, 1, or many `ClaimAtom`s
depending on how deep it walks the order book.

## 2. The walk from archive bytes to `ClaimAtom`

Public archive objects are XDR-serialized `LedgerCloseMeta` unions.
The walk from raw bytes to a trade tick:

```
LedgerCloseMeta::v2 (modern protocol)
  ├── ledgerHeader.header.scpValue.closeTime  ← wall-clock for OHLCV
  ├── ledgerHeader.header.ledgerSeq           ← ledger_sequence
  └── txProcessing<>                          ← Vec<TransactionResultMeta>
       └── for each tx:
             ├── result.transactionHash           ← tx hash (32 bytes)
             ├── result.result.result.code        ← txSUCCESS | txFAILED | …
             │     (only txSUCCESS produces real trades; txFAILED reverts all)
             └── result.result.result.results<>   ← Vec<OperationResult>
                   └── for each op:
                         ├── code = opINNER (else skip)
                         └── tr (union switch by OperationType):
                               ├── manageSellOfferResult
                               │     └── code = MANAGE_SELL_OFFER_SUCCESS
                               │     └── success.offersClaimed<>   ← Vec<ClaimAtom>
                               ├── manageBuyOfferResult
                               │     └── success.offersClaimed<>
                               ├── createPassiveSellOfferResult
                               │     └── success.offersClaimed<>
                               ├── pathPaymentStrictReceiveResult
                               │     └── success.offers<>          ← Vec<ClaimAtom>
                               └── pathPaymentStrictSendResult
                                     └── success.offers<>
```

Two gotchas worth pinning down because they're easy to get wrong:

1. **`txSUCCESS` only.** A `txFAILED` transaction's op results may
   still carry "would-have-run" data, but the ledger reverted the
   ops; those are not real trades. Filter at the transaction layer.
2. **Each op's result has its own code.** `opINNER` means the op
   ran; the inner result code (e.g. `MANAGE_SELL_OFFER_SUCCESS` vs
   `MANAGE_SELL_OFFER_UNDERFUNDED`) determines whether trades
   actually executed. Only `*_SUCCESS` variants carry the trades
   list.

## 3. `ClaimAtom` — the trade-tick unit

```xdr
enum ClaimAtomType {
    CLAIM_ATOM_TYPE_V0           = 0,    // pre-protocol-19 legacy
    CLAIM_ATOM_TYPE_ORDER_BOOK   = 1,    // modern offer match
    CLAIM_ATOM_TYPE_LIQUIDITY_POOL = 2   // classic-LP swap match (from path-payment)
};

struct ClaimOfferAtom {
    AccountID sellerID;
    int64     offerID;
    Asset     assetSold;
    int64     amountSold;     // stroops (Decimal128(7) when normalised)
    Asset     assetBought;
    int64     amountBought;   // stroops
};

struct ClaimLiquidityAtom {
    PoolID  liquidityPoolID;
    Asset   assetSold;
    int64   amountSold;
    Asset   assetBought;
    int64   amountBought;
};

struct ClaimOfferAtomV0 {
    uint256 sellerEd25519;    // legacy account-key form
    int64   offerID;
    Asset   assetSold;
    int64   amountSold;
    Asset   assetBought;
    int64   amountBought;
};

union ClaimAtom switch (ClaimAtomType type) {
    case CLAIM_ATOM_TYPE_V0:             ClaimOfferAtomV0      v0;
    case CLAIM_ATOM_TYPE_ORDER_BOOK:     ClaimOfferAtom        orderBook;
    case CLAIM_ATOM_TYPE_LIQUIDITY_POOL: ClaimLiquidityAtom    liquidityPool;
};
```

All three variants carry the same trade-shaped fields:
`(asset_sold, amount_sold, asset_bought, amount_bought)`. They differ
in counterparty identity:

- `V0` / `ORDER_BOOK` → a specific seller account + offer ID
  (counterparty is the resting offer's maker).
- `LIQUIDITY_POOL` → a pool ID (counterparty is the classic LP).
  These show up inside path-payment results because path payments
  route through both order book AND classic LPs.

For prices, the **counterparty distinction is informational**, not
required for price extraction. `(asset_sold, amount_sold,
asset_bought, amount_bought)` is sufficient to compute a tick price.

## 4. `Asset` — the canonical identity

```xdr
enum AssetType {
    ASSET_TYPE_NATIVE              = 0,
    ASSET_TYPE_CREDIT_ALPHANUM4    = 1,
    ASSET_TYPE_CREDIT_ALPHANUM12   = 2,
    ASSET_TYPE_POOL_SHARE          = 3   // not valid in trade contexts
};

struct AlphaNum4  { AssetCode4  assetCode; AccountID issuer; };
struct AlphaNum12 { AssetCode12 assetCode; AccountID issuer; };

union Asset switch (AssetType type) {
    case ASSET_TYPE_NATIVE:            void;
    case ASSET_TYPE_CREDIT_ALPHANUM4:  AlphaNum4  alphaNum4;
    case ASSET_TYPE_CREDIT_ALPHANUM12: AlphaNum12 alphaNum12;
};
```

For SDEX `ClaimAtom`s, `asset_type` is always 0, 1, or 2 (never 3 —
pool shares don't trade on SDEX directly). The asset's canonical
identity for the prices-api `assets` table is the 4-tuple
`(asset_type, asset_code, issuer_id, contract_id=0)`. Native XLM
is `(0, "", 0, 0)`.

## 5. Amounts: stroops, precision, signs

- All `int64 amount*` values are **stroops** (i.e. units of 10^-7 of
  the asset's whole unit, for both native and classic-credit assets).
- Always non-negative for valid trades.
- Convert to human/price math by dividing by `10^7` or using
  `Decimal128(7)` directly — never use floating-point for prices.

## 6. Example: a typical decoded `ClaimAtom`

A real (anonymised) order-book match from a `ManageSellOfferOp` that
crossed a resting USDC buyer:

```rust
ClaimAtom::OrderBook(ClaimOfferAtom {
    seller_id:     AccountId::from_str("GA…XYZ").unwrap(),    // counterparty
    offer_id:      318_842_159,                                // counterparty offer ID
    asset_sold:    Asset::Native,                              // XLM
    amount_sold:   1_000_000_000_i64,                          // 100.0000000 XLM
    asset_bought:  Asset::CreditAlphanum4(AlphaNum4 {
                       asset_code: AssetCode4(*b"USDC"),
                       issuer:     AccountId::from_str("GA5Z…CIRCLE").unwrap(),
                   }),
    amount_bought: 25_000_000_i64,                              // 2.5000000 USDC
})
```

Interpretation:

- The op submitter sold 100 XLM and received 2.5 USDC in this
  particular match.
- Tick price (USDC per XLM): `amount_bought / amount_sold` =
  `2.5 / 100` = **0.025 USDC per XLM**.
- Same op may include several more `ClaimAtom`s at different prices
  (the submitter's offer walked deeper into the book and matched
  each level separately) — each is its own tick.

## 7. Pair direction

Each `ClaimAtom` has a built-in direction: `assetSold → assetBought`.
For canonical OHLCV the prices-api side chooses a base/quote
orientation per pair (typically quote in USD-pegged stables) and
inverts when needed:

```
if asset_sold == canonical_base:
    price = amount_bought / amount_sold   # natural direction
else:                                      # asset_sold is quote
    price = amount_sold / amount_bought   # invert
```

Pair canonicalization rules live with the prices-api consumer; the
XDR doesn't pick a side.

## 8. Where this differs from Soroban AMM events

Stream 1 (Soroban AMM swaps via `soroban_events`) gets trade data
**as semantic ContractEvents** — a `"swap"` symbol topic + a data
payload whose shape is **AMM-contract-defined**. Decoding requires
per-AMM convention knowledge (task 0018 covers this).

Stream 2 (SDEX trades via `ClaimAtom`) gets trade data **as
protocol-defined typed structs** — the shape is fixed by the Stellar
protocol XDR spec, not by any application contract. One decoder
fits every SDEX trade across all 10+ years of history. Much
simpler. The cost is the archive read itself, not the decoding.

## 9. Tooling

The `stellar-xdr` Rust crate (the one Q4 of task 0015 confirmed
will be built for prices-api use) carries:

- `LedgerCloseMeta::from_xdr_bytes(&bytes)` (or equivalent) — decode
  archive bytes into the typed union.
- Generated `enum`/`struct` types for the entire hierarchy above.
- `Asset` carries `Display` / `FromStr` for canonical-key
  serialisation; pair identity work is then on the consumer.

No third-party parser is needed. The same crate covers Soroban
event ScVals (Stream 1) and SDEX classic results (Stream 2).

## References

- Stellar XDR (canonical spec): https://github.com/stellar/stellar-xdr
- Files of interest in `stellar-xdr/Stellar-transaction.x`:
  `ManageSellOfferResult`, `ManageBuyOfferResult`,
  `PathPaymentStrictReceiveResult`,
  `PathPaymentStrictSendResult`,
  `ClaimAtom`, `Asset`.
- `Stellar-ledger.x`: `LedgerCloseMeta`, `TransactionResultMeta`,
  `TransactionResultPair`.
- Network "Horizon" effects API documents the same data in a
  REST-friendly shape — useful for sanity-checking decoded results
  against a known-good source.
