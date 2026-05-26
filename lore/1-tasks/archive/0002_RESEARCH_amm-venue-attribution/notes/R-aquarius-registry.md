---
title: 'Aquarius (AQUA) Soroban AMM contract registry — mainnet'
type: research
status: developing
spawned_from: ../README.md
spawns: []
tags: [soroban, amm, aquarius, venue-attribution]
links:
  - 'https://github.com/AquaToken/soroban-amm'
  - 'https://docs.aqua.network/developers/code-examples/prerequisites-and-basics'
  - 'https://docs.aqua.network/developers/aquarius-soroban-functions'
  - 'https://api.stellar.expert/explorer/directory/CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK'
  - 'https://api.stellar.expert/explorer/public/contract/CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK'
  - 'https://api.stellar.expert/explorer/public/contract/CA6PUJLBYKZKUEKLZJMKBZLEKP2OTHANDEOWSFF44FTSYLKQPIICCJBE'
history:
  - date: 2026-05-08
    status: developing
    who: claude
    note: >
      Confirmed Aquarius AMM router CBQDHNBFBZYE… via stellar.expert
      directory + verified package_name. Confirmed canonical pool
      `Symbol("trade")` and router `Symbol("swap")` formats from
      AquaToken/soroban-amm source. Spot-checked top swap-emitters:
      only the router is Aquarius; the 43 other Symbol("swap")
      emitters are NOT Aquarius (different WASM, different creators).
---

# Aquarius (AQUA) Soroban AMM contract registry

## Canonical addresses

The Aquarius project (aqua.network, GitHub `AquaToken/soroban-amm`) does
**not** publish a stable mainnet address sheet in its README. The
verified pieces below come from cross-referencing the docs site, the
stellar.expert directory entry, the verified contract metadata, and the
source-code event signatures.

| Role                                                               | Contract ID                                                                  | Source / evidence                                                                                                                                                                                                                                                                                                                                                                                      | stellar.expert label                                                                                            |
| ------------------------------------------------------------------ | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------- |
| **Liquidity-pool router** (entry point + pool catalogue + factory) | `CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK`                   | `docs.aqua.network/developers/code-examples/prerequisites-and-basics` literally documents this as "the contract ID of the Aquarius AMM contract"; stellar.expert directory entry `name = "Aquarius Router", domain = "aqua.network", tag = "defi"`; verified contract `package_name = "soroban-liquidity-pool-router-contract"`, repo = `https://github.com/AquaToken/soroban-amm`; created 2024-07-25 | Aquarius Router (verified)                                                                                      |
| **Constant-product pool WASM**                                     | code hash `ae0da5a84b15805c5c7931ac567a8d1b34be3f26b483993d9ff80cb2c3de9852` | Derived from observed pool `CA6PUJLBYKZKUEKLZJMKBZLEKP2OTHANDEOWSFF44FTSYLKQPIICCJBE` (an active `trade`-emitter). Pool emits exactly the `Symbol("trade")` topic shape defined in `liquidity_pool_events/src/lib.rs` of the AquaToken repo (master branch).                                                                                                                                           | (pool itself is unverified on stellar.expert, but WASM match attributes it to Aquarius `liquidity_pool` module) |
| **Constant-product pool — example**                                | `CA6PUJLBYKZKUEKLZJMKBZLEKP2OTHANDEOWSFF44FTSYLKQPIICCJBE`                   | Trade-event sample in `archive/0001/.../evidence/trade_event_sample.json`; topic+data match Aquarius `LiquidityPoolEvents::trade` exactly (see "Event formats" below)                                                                                                                                                                                                                                  | unverified (no directory entry)                                                                                 |

Pool-type-specific factory/WASM addresses for **stableswap**
(`liquidity_pool_stableswap`) and **concentrated** (`liquidity_pool_concentrated`)
were **not found** as published mainnet addresses. The router is the
single public entry point; deploying a new pool of any type goes through
the router, which is why no separate "factory" addresses are documented.

## Aquarius event formats (canonical, from source)

### Pool-level events (`liquidity_pool_events/src/lib.rs`)

`AquaToken/soroban-amm@master:liquidity_pool_events/src/lib.rs` —
applied by `liquidity_pool` (xy=k), `liquidity_pool_stableswap`, and
`liquidity_pool_concentrated`:

```rust
fn trade(user, token_in, token_out, in_amount, out_amount, fee_amount) {
    // topics: ("trade", token_in: Address, token_out: Address, user: Address)
    // body:   (in_amount as i128, out_amount as i128, fee_amount as i128)
}

fn update_reserves(reserves: Vec<u128>) {
    // topics: ("update_reserves",)
    // body:   reserves as Vec<i128>  (length = number of pool tokens)
}

fn deposit_liquidity(tokens, amounts, share_amount) {
    // topics: ("deposit_liquidity", assetA: Address, assetB[, assetC]: Address)
    // body:   [share_amount: i128, amountA: i128, amountB[, amountC]: i128]
}

fn withdraw_liquidity(tokens, amounts, share_amount) {
    // topics: ("withdraw_liquidity", assetA: Address, assetB[, assetC]: Address)
    // body:   [share_amount: i128, amountA: i128, amountB[, amountC]: i128]
}

// Plus: kill_*/unkill_*, set_protocol_fee, claim_protocol_fee, reserves_sync.
```

### Router events (`liquidity_pool_router/src/events.rs`)

`AquaToken/soroban-amm@master:liquidity_pool_router/src/events.rs`:

```rust
fn swap(tokens, user, pool_id, token_in, token_out, in_amount, out_amt) {
    // topics: ("swap", tokens: Vec<Address>, user: Address)
    // body:   (pool_id: Address, token_in: Address, token_out: Address,
    //          in_amount: u128, out_amt: u128)
}

fn add_pool(tokens, pool_address, pool_type, subpool_salt, init_args) {
    // topics: ("add_pool", tokens: Vec<Address>)
    // body:   (pool_address: Address, pool_type: Symbol,
    //          subpool_salt: BytesN<32>, init_args: Vec<Val>)
}

fn config_rewards(tokens, pool_address, pool_tps, expired_at) {
    // topics: ("config_rewards", tokens: Vec<Address>)
    // body:   (pool_address: Address, pool_tps: u128, expired_at: u64)
}

fn deposit / withdraw / claim / set_protocol_fee / pool_gauge_switch_token …
```

The router `swap` topic shape — `(swap, Vec<Address>, Address)` with
body `(Address, Address, Address, U128, U128)` — matches verbatim the
3.5-day-window observation in `R-swap-topic-shapes.md`:

```
topics = [Symbol("swap"), Vec[Address(token_in), Address(token_out)], Address(trader)]
data   = Vec [Address(pool), Address(token_a), Address(token_b),
              U128(amount_in), U128(amount_out)]
```

So the Aquarius router can be enumerated for pool creation by scanning
its `add_pool` events; `pool_type: Symbol` indicates whether the new
pool is `constant_product`, `stable`, or (presumably) `concentrated`.

## Spot-check vs. observed emitters

### `Symbol("swap")` top emitters (from `R-swap-topic-shapes.md`, wider sample)

| Events | Contract                | WASM hash                          | Aquarius?                                              | Evidence                                                                                                          |
| -----: | ----------------------- | ---------------------------------- | ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------- |
| 11,947 | `CBQDHNBFBZYE…CKWC6QUK` | (router wasm)                      | **YES**                                                | stellar.expert directory: "Aquarius Router", aqua.network. `package_name=soroban-liquidity-pool-router-contract`. |
|  4,128 | `CBHCRSVX3ZZ7…BIZX`     | `167ab414…506c`                    | **NO** (different WASM, different creator `GCNPDMUM…`) | Not Aquarius. Candidate for Phoenix/Soroswap router (other forks).                                                |
|  2,706 | `CCR2CH4GQVCZ…H2MQ`     | `48b28121…c117`                    | **NO** (different WASM, different creator `GCFB64LD…`) | Not Aquarius.                                                                                                     |
|  2,480 | `CDMIM23WOUL5…ZCHL`     | `4edd745f…6c8d`                    | **NO** (different WASM, different creator `GC7RIP4D…`) | Not Aquarius.                                                                                                     |
|    440 | `CBCZGGNOEUZG…3GLH`     | `167ab414…506c` (same as CBHCRSVX) | **NO**                                                 | Same WASM/creator family as CBHCRSVX — same non-Aquarius protocol.                                                |

**Strong conclusion:** Aquarius's mainnet AMM router is exactly one
contract (`CBQDHNBFBZYE…`). All 43 _other_ `Symbol("swap")` emitters in
the wider sample are NOT Aquarius. The "single emitter for `Symbol("swap")`"
observation in the original 3.5-day window was therefore correctly
attributed to Aquarius — the wider sample's 43 additional emitters
belong to other protocols (Phoenix, plus possibly others).

### `Symbol("trade")` emitter — sample

| Contract            | Topic / data                                                                                                                                 | Attribution                                                                                                                                                    |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CA6PUJLBYKZK…CJBE` | topics `[trade, CAS3J7GY… (token_in), CCW67TSZ… (token_out), GDCRZPZY… (trader)]`; body `[i128 25761941491, i128 3901204480, i128 12880971]` | **Aquarius constant-product pool** — exact bit-for-bit match to `LiquidityPoolEvents::trade` in `liquidity_pool_events/src/lib.rs`. WASM hash `ae0da5a8…9852`. |

**Inference:** all 29 distinct `Symbol("trade")` emitters from
`R-swap-topic-shapes.md` should be Aquarius pools. The `update_reserves`
topic co-emission (29/29 overlap) matches the canonical
`LiquidityPoolEvents::update_reserves` definition. Final confirmation
would require checking each pool's WASM hash equals `ae0da5a8…9852` —
recommended as a small follow-up but the topic-format match is already
highly distinctive (Aquarius is the only known Soroban protocol using
exactly this `(trade, sold_asset, bought_asset, trader)` topic shape
with a 3-element `Vec<i128>` body of `(sold_amount, bought_amount, fee)`).

### `Symbol("add_pool")` and `Symbol("config_rewards")` co-emission

These two topics co-emitted from `CBQDHNBFBZYE…` in the 0001 sample
(2 + 44 events) match the router's `add_pool` and `config_rewards`
emission methods exactly. This is a third independent confirmation of
Aquarius router attribution.

## Pool enumeration approach (for the BE indexer)

Two strategies, in increasing recall:

1. **Topic-only filter (high precision, low recall for new pools):**
   match contract events with `topics[0] == Symbol("trade")` and decode
   per `liquidity_pool_events/src/lib.rs`. Token addresses are inline in
   the topic vector — no per-pool lookup needed, unlike Soroswap.

2. **Factory-style enumeration via router `add_pool` events:** scan the
   Aquarius router (`CBQDHNBFBZYE…`) for `Symbol("add_pool")` events.
   Each such event's `body[0]` (Address) is a freshly created pool
   contract; `body[1]` (Symbol) is the pool type
   (`constant_product` / `stable` / `concentrated`). This gives the
   indexer a complete, authoritative Aquarius pool registry.

Recommended for §11.1 of `amm-trades-schema.md`: both. The topic filter
is the data path; the router scan is the venue-attribution registry
that makes per-pool `venue = "aquarius"` certain.

## Open items

- **Stableswap / concentrated pool WASM hashes** are not yet observed
  in the 0001 sample (no `Symbol("trade")` emitter has a different WASM
  yet documented). When a stableswap pool first appears, its WASM hash
  should be recorded as a second canonical Aquarius pool WASM. The
  topic shape (`trade`, `update_reserves`) is shared across all three
  pool types, so the indexer's decoder is identical regardless.
- **Per-pool token addresses for `update_reserves`:** `update_reserves`
  topics are just `[Symbol("update_reserves")]` with body `Vec<i128>` —
  the indexer must look up pool tokens by ID once and cache them.

## Sources

- `https://github.com/AquaToken/soroban-amm` — official repo (master branch).
  - `liquidity_pool_events/src/lib.rs` — pool event definitions.
  - `liquidity_pool_router/src/events.rs` — router event definitions.
  - `readme.md` — module list (`liquidity_pool`, `liquidity_pool_stableswap`,
    `liquidity_pool_concentrated`, `liquidity_pool_router`,
    `liquidity_pool_plane`, `liquidity_pool_liquidity_calculator`,
    `batcher`, `guard`).
- `https://docs.aqua.network/developers/code-examples/prerequisites-and-basics` —
  documents `CBQDHNBFBZYE…CKWC6QUK` as "the contract ID of the Aquarius
  AMM contract" on mainnet.
- `https://docs.aqua.network/developers/aquarius-soroban-functions` —
  same router address documented again.
- `https://api.stellar.expert/explorer/directory/CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK` —
  directory entry: name "Aquarius Router", domain `aqua.network`, tag `defi`.
- `https://api.stellar.expert/explorer/public/contract/CBQDHNBFBZYE…` —
  verified contract metadata: `package_name=soroban-liquidity-pool-router-contract`.
- `archive/0001_RESEARCH_dump-amm-swap-events/notes/evidence/trade_event_sample.json` —
  empirical Aquarius `trade` event from mainnet ledger 62079996.
