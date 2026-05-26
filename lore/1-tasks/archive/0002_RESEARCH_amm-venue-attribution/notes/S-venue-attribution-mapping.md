---
title: "Contract → venue mapping for observed AMM emitters"
type: synthesis
status: mature
spawned_from: ../README.md
spawns: []
tags: [soroban, amm, venue-attribution, schema-validation]
links:
  - "../../0001_RESEARCH_dump-amm-swap-events/notes/R-swap-topic-shapes.md"
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

### Top 5 `topic_0=swap` emitters (from R-swap-topic-shapes.md wider sample)

> **Note:** The R-swap-topic-shapes.md ranking conflated `Symbol("swap")`
> and `String("swap")` because it grouped on `topic_0` after
> normalisation. The table below preserves that ranking; the corrected
> per-kind breakdown is in §"Cross-check vs 0001 sample".

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
| Phoenix XYK pool | `String("swap") + String(<field>)` × **8** | scalar-per-event | **8 events = 1 trade** (6 for Phoenix stable pool) |

Phoenix's 8-event group is the surprising finding. The indexer must group by `(tx_hash, op_index, contract_id)` and reassemble one `prices_amm_trades` row from the eight `String("swap")` events. Source: `phoenix-contracts/contracts/pool/src/contract.rs:1172-1185`. (Note: source uses `&str` tuple `("swap", "sender")` which compiles to `String` ScVal, not `Symbol`. See §"Cross-check vs 0001 sample" for empirical verification.)

This means filtering on `topic_0 = swap` (any kind) alone is insufficient — the indexer cannot row-by-row map a Phoenix swap event to a trade, and must additionally distinguish Phoenix's `String("swap")` from Aquarius's `Symbol("swap")` to dispatch the right decoder.

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

## Cross-check vs 0001 sample (verification 2026-05-08)

The user requested verification that registry addresses match the actual
event emitters from task 0001's sample data. Re-ran
`dump-swap-events --symbol swap` against `.temp/FC47D9FF--62400000-62463999`
and split emitters by `topics[0]` ScVal kind. Results:

### Direct emitter matches (canonical address = observed `contract_id`)

| Registry address | Role | 0001 evidence | Match |
|---|---|---|---|
| `CBQDHNBFBZYE...` | Aquarius router | `swap_event_sample.json` `contract_id`; #1 `Symbol("swap")` emitter (11,947 events) in wider sample | ✅ |
| `CA6PUJLBYK...` | Aquarius pool sample | `trade_event_sample.json` `contract_id` | ✅ |
| `CAG5LRYQ...` | Soroswap router | `soroswap_router_swap_sample.json` `contract_id` | ✅ |
| `CAYP3UWL...` | Soroswap aggregator | `soroswap_aggregator_swap_sample.json` `contract_id` | ✅ |
| `CAM7DY53G...` | Soroswap pool sample | `soroswap_pair_swap_sample.json` `contract_id`; matches factory's pair WASM | ✅ |
| 9 of 11 Phoenix pools | Phoenix XYK pool | `String("swap")` emitter list — see next sub-section | ✅ |

### Phoenix pools — 9 of 11 attested in the 4-day sample (corrected topic kind)

| Phoenix pool address | Pair | Events in sample | Topic kind |
|---|---|---:|---|
| `CBHCRSVX3ZZ7...` | XLM/USDC | 4,128 | `String("swap")` |
| `CBCZGGNOEUZG...` | XLM/PHO | 440 | `String("swap")` |
| `CB5QUVK5GS3IU...` | XLM/EURX | 272 | `String("swap")` |
| `CD5XNKK3B6BEF...` | PHO/USDC | 224 | `String("swap")` |
| `CC6MJZN3HFOJK...` | EURX/USDC | 224 | `String("swap")` |
| `CDMXKSLG5GIT...` | XLM/USDX | 152 | `String("swap")` |
| `CBISULYO5ZGS...` | XLM/EURC | 144 | `String("swap")` |
| `CCUCE5H5CKW3...` | GBPX/USDC | 72 | `String("swap")` |
| `CCKOC2LJTPDB...` | XLM/GBPX | 48 | `String("swap")` |
| `CDQLKNH3725...` | USDC/VEUR | **0** | not observed (low volume) |
| `CBW5G5SO5SDY...` | USDC/VCHF | **0** | not observed (low volume) |

**Total: 5,704 `String("swap")` events from 9 distinct emitters — 100% are Phoenix pools.** This is much stronger evidence for Phoenix attribution than the stellar.expert directory check alone: the on-chain emitter list and the `phoenix-contracts` repo's pool list match exactly.

### Topic-kind correction (load-bearing for indexer §7)

**Phoenix pool swap events use `topics[0] = ScVal::String("swap")`,
not `ScVal::Symbol("swap")`.**

- `R-phoenix-registry.md` (and the upstream Phoenix source-code claim
  it cited) said `Symbol("swap") + Symbol(<field>)`. On-chain truth is
  **`String("swap") + String(<field>)`**. The deployed WASM evidently
  uses the String ScVal variant; either the source-code reading missed
  this or the deployed WASM was built from a branch we didn't inspect.
  The on-chain bytes are authoritative.
- `R-swap-topic-shapes.md` (task 0001) "44 distinct contracts emit
  Symbol("swap")" was an overcount — it grouped by the histogram's
  normalised `topic_0` string, mixing `Symbol("swap")` and
  `String("swap")` emissions. The correct split is:
  - **`Symbol("swap")`**: 17,863 events / **35** distinct emitters.
  - **`String("swap")`**: 5,704 events / **9** distinct emitters
    (all Phoenix in this sample).
- This is good news for the indexer: **Phoenix vs Aquarius can be
  distinguished by the ScVal kind of `topics[0]`**, not just by
  contract registry. The §7 filter must therefore branch on
  `(topics[0].kind, topics[0].value)`:

| `topics[0].kind` | value | Decoder |
|---|---|---|
| `Symbol` | `swap` | Aquarius router (`CBQDHNBFBZYE...` and friends) |
| `Symbol` | `trade` | Aquarius constant-product pool |
| `String` | `swap` | Phoenix XYK pool — group 8 events to 1 trade |
| `String` | `SoroswapPair` (with `topics[1]=Symbol("swap")`) | Soroswap pool |
| `String` | `SoroswapRouter` / `SoroswapAggregator` | Soroswap user-facing event (skip if also have pool event) |

### Re-attribution of `Symbol("swap")` emitters after correction

The full top-5 `Symbol("swap")` emitters in the 4-day sample (re-run
2026-05-08):

| Events | Contract | Attribution |
|---:|---|---|
| 11,947 | `CBQDHNBFBZYE...` | **Aquarius router** (verified) |
| 2,706 | `CCR2CH4GQVCZ...` | **unknown** — emits `Symbol("swap")` like a router; not Aquarius router (different WASM/creator), not Soroswap, not Phoenix |
| 2,480 | `CDMIM23WOUL5...` | **unknown** — same pattern |
| 335 | `CCXRRORTOXXP...` | unknown |
| 229 | `CAUF4DFYSX52...` | unknown |
| (+ 30 more in long tail) | | |

The earlier "Phoenix XLM/USDC pool" attribution for `CBHCRSVX3ZZ7...`
(at "4,128 events") in the original synthesis was correct in venue —
but the topic kind is **String("swap")**, not Symbol. The original
ranking from `R-swap-topic-shapes.md` conflated the two.

### Phoenix factory + multihop — not directly observed in 0001 sample

`CB4SVAWJA6...` (factory) and `CCLZRD4E72...` (multihop) did not emit
any `topic_0=swap` events in the 4-day window. This is consistent —
factory events are rare (only on pool creation), and the multihop emits
aggregated user-intent events that may not pass the `--symbol swap`
filter or were absent in this window. Factory attestation rests on:

1. The 9 attested pools' addresses match the factory's deployed-pool
   list verbatim (per `R-phoenix-registry.md`, sourced from
   `phoenix-contracts/scripts/upgrade_mainnet.sh`).
2. The factory address itself appears in the same source-of-truth file.

This is **second-order verification** — strong but not equivalent to
seeing the factory emit a creation event in our local sample. Closing
this loop would require either (a) a wider `--no-filter` dump scanning
for the factory's `[Symbol("create"), Symbol("liquidity_pool")]` event
in a window covering at least one Phoenix pool deployment, or (b)
independent stellar.expert API verification of the factory's contract
metadata.

## Future work

Three concrete follow-ups (each a candidate for a backlog task — see parent README §"Future work" for the spawned IDs):

1. **Identify the two unknown `Symbol("swap")` emitters** `CCR2CH4G...` and `CDMIM23W...` — they account for ~5,200 events combined and are not Aquarius / Phoenix / Soroswap. Likely a fourth Soroban DEX or a stale internal contract; either way the indexer needs a known-target allowlist or it will emit untagged trade rows. (→ candidate backlog task 0005.)

2. **Update `amm-trades-schema.md` §7 / §11.1** with the three-decoder reality, the Phoenix multi-event grouping rule, and the venue → factory-event registry strategy. Existing backlog 0003 covers most of this — extend its acceptance criteria to include Phoenix grouping + Soroswap two-topic filter rather than spawning a new task.

3. **Mark backlog 0004 as superseded** — its acceptance criteria are answered by `R-swap-topic-shapes.md` "Update: wider sample" plus this attribution. Phoenix is not low-volume; it was hidden inside `topic_0=swap`, but in the **`String("swap")`** bucket the original analysis didn't distinguish from `Symbol("swap")`.
