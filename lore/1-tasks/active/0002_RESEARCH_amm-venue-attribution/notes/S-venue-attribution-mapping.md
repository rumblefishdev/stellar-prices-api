---
title: "Contract → venue mapping for observed AMM emitters"
type: synthesis
status: mature
spawned_from: ../README.md
spawns: []
tags: [soroban, amm, venue-attribution, schema-validation]
links:
  - "../../../archive/0001_RESEARCH_dump-amm-swap-events/notes/R-swap-topic-shapes.md"
  - "R-soroswap-registry.md"
  - "R-aquarius-registry.md"
  - "R-phoenix-registry.md"
  - "../../../../docs/database-schema/amm-trades-schema.md"
history:
  - date: 2026-05-08
    status: mature
    who: claude
    note: >
      Synthesised three parallel registry research notes into a single
      contract → venue mapping. Confirms Aquarius router as the
      11,947-event Symbol("swap") emitter, attributes two of the next
      four to Phoenix XYK pools, and confirms Soroswap pool WASM via
      factory-derivation.
---

# Contract → venue mapping

## Canonical mainnet addresses (the registries)

| Venue | Role | Contract |
|---|---|---|
| Soroswap | Factory | `CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2` |
| Soroswap | Router | `CAG5LRYQ5JVEUI5TEID72EYOVX44TTUJT5BQR2J6J77FH65PCCFAJDDH` |
| Soroswap | Aggregator | `CAYP3UWLJM7ZPTUKL6R6BFGTRWLZ46LRKOXTERI2K6BIJAWGYY62TXTO` |
| Soroswap | Pair WASM hash | `18051456816b66f12e773a56f77c5794fac1b1fb7ab6e22d4fad5a412770f73e` |
| Aquarius | Router | `CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK` |
| Aquarius | Constant-product pool WASM hash | `ae0da5a84b15805c5c7931ac567a8d1b34be3f26b483993d9ff80cb2c3de9852` |
| Phoenix | Factory | `CB4SVAWJA6TSRNOJZ7W2AWFW46D5VR4ZMFZKDIKXEINZCZEGZCJZCKMI` |
| Phoenix | Multihop (router) | `CCLZRD4E72T7JCZCN3P7KNPYNXFYKQCL64ECLX7WP5GNVYPYJGU2IO2G` |

Sources for each address are cited in the per-venue R-notes.

## Observed emitter attribution

### Top 5 `Symbol("swap")` emitters (from R-swap-topic-shapes.md wider sample)

| Events | Contract | Venue | Role | Evidence |
|---:|---|---|---|---|
| 11,947 | `CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK` | **Aquarius** | Router | stellar.expert label "Aquarius Router"; package `soroban-liquidity-pool-router-contract`; documented in `docs.aqua.network` |
| 4,128 | `CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX` | **Phoenix** | XYK pool (XLM/USDC) | stellar.expert label "Phoenix Pool"; matches Phoenix factory's deployed pool list |
| 2,706 | `CCR2CH4GQVCZHG7CHFVMNANCK45CU5DVKXZIIITDZQAU3CEJZ7RQH2MQ` | unknown | — | unlabelled on stellar.expert; not Aquarius (different WASM/creator), not Phoenix (not in factory list), not Soroswap (different topic shape) |
| 2,480 | `CDMIM23WOUL5CZBKX3GOA3V5R5AMVIMTCP52KCDQORWELAPLJ27WZCHL` | unknown | — | same as above |
| 440 | `CBCZGGNOEUZG4CAAE7TGTQQHETZMKUT4OIPFHHPKEUX46U4KXBBZ3GLH` | **Phoenix** | XYK pool (XLM/PHO) | stellar.expert label "Phoenix Pool"; same WASM as `CBHCRSVX...` |

The two unknowns share neither WASM nor creator with Aquarius or Phoenix — they are likely a fourth Soroban DEX outside the {Soroswap, Aquarius, Phoenix} target set, or a stale/internal contract. See `## Future work` below.

### `Symbol("trade")` emitters (29 distinct)

All 29 are **Aquarius constant-product pools**. Evidence:

- Topic vector matches Aquarius `LiquidityPoolEvents::trade` exactly: `[Symbol("trade"), Address(sold_asset), Address(bought_asset), Address(trader)]`, body `Vec<i128>(in, out, fee)`.
- Sample emitter `CA6PUJLBYKZKUEKLZJMKBZLEKP2OTHANDEOWSFF44FTSYLKQPIICCJBE` has the canonical Aquarius constant-product WASM hash.
- 29/29 also emit `update_reserves` — matches Aquarius pool semantics (reserves event paired with each trade).
- No other observed venue emits this exact 4-topic / 3-i128-data shape.

The full list of 29 contract IDs lives in the wider-sample evidence from task 0001; per-pool spot-check is unnecessary because the topic shape is venue-distinctive.

### `String("SoroswapPair")` emitters (79 distinct)

All 79 are **Soroswap pool contracts**. Evidence:

- Topic shape `[String("SoroswapPair"), Symbol(<op>)]` is unique to Soroswap (Soroswap is the only observed venue using `String` in `topics[0]`).
- Sampled pool `CAM7DY53G63XA4AJRS24Z6VFYAFSSF76C3RZ45BE5YU3FQS5255OOABP` deploys the Soroswap factory's pinned pair WASM hash (`18051456...0f73e`).
- Soroswap pools can be authoritatively enumerated by scanning the Soroswap factory (`CA4HEQTL...`) for `[String("SoroswapFactory"), Symbol("new_pair")]` events; data carries `pair: Address`.

## Acceptance criteria check

- [x] `swap`-emitter `CBQDHNBFBZYE...` attributed: **Aquarius router**.
- [x] At least one of the 29 `trade`-emitters attributed: all 29 are **Aquarius constant-product pools** (sample `CA6PUJLBYK...`).
- [x] Phoenix factory documented: `CB4SVAWJA6TSRNOJZ7W2AWFW46D5VR4ZMFZKDIKXEINZCZEGZCJZCKMI` (multihop router `CCLZRD4E...` also documented).
- [x] Top 5 `Symbol("swap")` emitters mapped — 3 attributed (1 Aquarius + 2 Phoenix), 2 deferred to follow-up as **unknown** with evidence.
- [x] `SoroswapPair` emitter cross-checked against factory-derivation: WASM hash matches `hashes.pair` from `soroswap/core` mainnet contracts.

## Indexer implications (load-bearing for §7 and decoder design)

These supersede the hypothetical wording in `docs/database-schema/amm-trades-schema.md` §7 step 3 and the Open Question §11.1.

### 1. Three pool decoders, not two — and Phoenix is event-multiplexed

Three observed pool-level decoders, each materially different:

| Decoder | Topic match | Data | Trade row count |
|---|---|---|---|
| Aquarius pool | `Symbol("trade")` | `Vec<i128>(in, out, fee)` | 1 event = 1 trade |
| Soroswap pool | `String("SoroswapPair") + Symbol("swap")` | uniswap-v2 `Map{amount_0_in, amount_0_out, amount_1_in, amount_1_out, to}` | 1 event = 1 trade |
| Phoenix XYK pool | `Symbol("swap") + Symbol(<field>)` × **8** | scalar-per-event | **8 events = 1 trade** (6 for Phoenix stable pool) |

Phoenix's 8-event group is the surprising finding. The indexer must group by `(tx_hash, op_index, contract_id)` and reassemble one `prices_amm_trades` row from the eight `Symbol("swap")` events. Source: `phoenix-contracts/contracts/pool/src/contract.rs:1172-1185`.

This means filtering on `topic_0 = Symbol("swap")` alone is insufficient — the indexer cannot row-by-row map a Phoenix swap event to a trade.

### 2. Pool enumeration via factory events (recommended)

For each venue, the indexer maintains a per-venue `(pool_address) → venue` lookup populated by replaying factory events from genesis:

| Venue | Factory address | Pool-creation topic | Data |
|---|---|---|---|
| Soroswap | `CA4HEQTL...` | `[String("SoroswapFactory"), Symbol("new_pair")]` | `NewPairEvent { token_0, token_1, pair, new_pairs_length }` |
| Aquarius | router `CBQDHNBFBZYE...` | `Symbol("add_pool")` | `(pool_address, pool_type: Symbol)` — distinguishes constant_product / stable / concentrated |
| Phoenix | `CB4SVAWJA6...` | `[Symbol("create"), Symbol("liquidity_pool")]` | `Address(pool_id)` |

Aquarius is special: its router emits `add_pool` and is itself the venue registry, so the indexer also reads `pool_type: Symbol` and dispatches to the right decoder per pool.

### 3. Schema gap: Phoenix `fee` column

Phoenix XYK pools do not emit `commission_amount` — only `spread_amount`. The commission goes via direct token transfer to `fee_recipient`. Options:

(a) Leave `prices_amm_trades.fee` NULL for Phoenix rows.
(b) Compute fee from `total_fee_bps × offer_amount` (requires reading pool config once + caching).
(c) Sum ledger-effects on the fee_recipient account for the same tx.

Recommend (b) — same shape as Aquarius's inline fee, consistent post-condition for downstream consumers. Document the chosen semantics in §11.x.

### 4. Soroswap pool data has no inline tokens

`SoroswapPair` swap events carry `amount_0_in/out`, `amount_1_in/out`, `to` — **no token addresses**. The indexer must read each Soroswap pool's `token_0` / `token_1` once at pool-discovery time (from factory event) and cache. Already raised in `R-swap-topic-shapes.md` §"Updated implications for the schema"; reinforced here.

## Future work

Three concrete follow-ups (each a candidate for a backlog task — see parent README §"Future work" for the spawned IDs):

1. **Identify the two unknown `Symbol("swap")` emitters** `CCR2CH4G...` and `CDMIM23W...` — they account for ~5,200 events combined and are not Aquarius / Phoenix / Soroswap. Likely a fourth Soroban DEX or a stale internal contract; either way the indexer needs a known-target allowlist or it will emit untagged trade rows. (→ candidate backlog task 0005.)

2. **Update `amm-trades-schema.md` §7 / §11.1** with the three-decoder reality, the Phoenix multi-event grouping rule, and the venue → factory-event registry strategy. Existing backlog 0003 covers most of this — extend its acceptance criteria to include Phoenix grouping + Soroswap two-topic filter rather than spawning a new task.

3. **Mark backlog 0004 as superseded** — its acceptance criteria are answered by `R-swap-topic-shapes.md` "Update: wider sample" plus this attribution. Phoenix is not low-volume; it was hidden inside `Symbol("swap")` as hypothesised.
