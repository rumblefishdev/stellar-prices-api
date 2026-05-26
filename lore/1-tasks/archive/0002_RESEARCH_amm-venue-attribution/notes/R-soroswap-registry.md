---
title: 'Soroswap canonical mainnet contract registry and event format'
type: research
status: developing
spawned_from: ../README.md
spawns: []
tags: [soroban, amm, soroswap, venue-attribution]
links:
  - 'https://github.com/soroswap/core/blob/main/public/mainnet.contracts.json'
  - 'https://github.com/soroswap/aggregator/blob/main/public/mainnet.contracts.json'
  - 'https://github.com/soroswap/core/blob/main/contracts/factory/src/event.rs'
  - 'https://docs.soroswap.finance/'
  - 'https://api.stellar.expert/explorer/public/contract/CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2'
history:
  - date: 2026-05-08
    status: seed
    who: claude
    note: 'Initial registry compiled from soroswap/core + soroswap/aggregator GitHub repos and verified via stellar.expert API.'
---

# Soroswap canonical mainnet contract registry

## Canonical addresses

Source of truth: `public/mainnet.contracts.json` in
[soroswap/core](https://github.com/soroswap/core/blob/main/public/mainnet.contracts.json)
and [soroswap/aggregator](https://github.com/soroswap/aggregator/blob/main/public/mainnet.contracts.json)
(retrieved 2026-05-08 via `gh api`). All WASM hashes cross-checked against
`api.stellar.expert/explorer/public/contract/<id>` — all match.

| Role                         | Contract ID                                                | WASM hash                                                          | Source                                                   |
| ---------------------------- | ---------------------------------------------------------- | ------------------------------------------------------------------ | -------------------------------------------------------- |
| **SoroswapFactory**          | `CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2` | `5db738b05d…d4fad5a4`                                              | core/mainnet.contracts.json `ids.factory`                |
| **SoroswapRouter**           | `CAG5LRYQ5JVEUI5TEID72EYOVX44TTUJT5BQR2J6J77FH65PCCFAJDDH` | `4c3db3eb…6c4ba07`                                                 | core/mainnet.contracts.json `ids.router`                 |
| **SoroswapAggregator**       | `CAYP3UWLJM7ZPTUKL6R6BFGTRWLZ46LRKOXTERI2K6BIJAWGYY62TXTO` | `5e0bff5a…62447d2c`                                                | aggregator/mainnet.contracts.json `ids.aggregator`       |
| Aggregator: Soroswap adapter | `CC6KQUATUBCIFZRDJL5X5PHCYGOHLPHKZQPUOTZTQTASGU5AUQ6DS7SC` | `e2c5b7018a…804e1094`                                              | aggregator/mainnet.contracts.json `ids.soroswap_adapter` |
| Aggregator: Phoenix adapter  | `CCEBUGFV3D73OMV7MUXXA43AREY53MUHVD5SMUM7YZODNGY4NZBA2TSC` | `ab68de68…2374ee3a0a01c82b`                                        | aggregator/mainnet.contracts.json `ids.phoenix_adapter`  |
| Aggregator: Aqua adapter     | `CDHDUKHFZB6FORHEBZCNYI3GGVNVOLEITSGI7OKU4UIQND5QG75KGSRR` | `e26face3…157c8d29`                                                | aggregator/mainnet.contracts.json `ids.aqua_adapter`     |
| Aggregator deployer          | `CALUZAZZ6FHENZJBQLSTPBZBZOZMACRIHMAEGSCGJYRC35RA5ZDXTKP2` | `8454147…f10694`                                                   | aggregator/mainnet.contracts.json `ids.deployer`         |
| **Pair WASM hash**           | (varies — pools deployed by factory)                       | `18051456816b66f12e773a56f77c5794fac1b1fb7ab6e22d4fad5a412770f73e` | core/mainnet.contracts.json `hashes.pair`                |

**Pool enumeration:** Soroswap pools are _not_ a fixed list. They are
deployed deterministically by the factory. Each Soroswap pool contract
shares the same WASM hash (`18051456816b66f12e773a56f77c5794fac1b1fb7ab6e22d4fad5a412770f73e`).
The indexer can either:

1. Scan `SoroswapFactory` for `("SoroswapFactory", "new_pair")` events and
   extract `pair: Address`, OR
2. Match deployed contracts by WASM hash via `stellar.expert` /
   `getLedgerEntries` lookup of `ContractCode`.

`stellar.expert` shows the SoroswapFactory contract was created by
`GAYPUMZFDKUEUJ4LPTHVXVG2GD5B6AV5GGLYDMSZXCSI4QILQKSY25JI` (the same G-account
also deployed the router 36 ledgers later) at unix `1710174189`
(2024-03-11), confirming a single-deployer rollout.

Validation note: all canonical addresses report `validation.status:
"unverified"` on stellar.expert (no `repository` field tagged for
verification). Soroswap has not pushed verified-build metadata to the
explorer; identity is established by WASM-hash match against
`mainnet.contracts.json` in the public GitHub repos.

## Factory event format (pair creation)

Source: [soroswap/core/contracts/factory/src/event.rs](https://github.com/soroswap/core/blob/main/contracts/factory/src/event.rs)
(branch `main`).

The factory publishes a `new_pair` event when a new pool is created:

```rust
e.events().publish(
    ("SoroswapFactory", symbol_short!("new_pair")),
    NewPairEvent { token_0, token_1, pair, new_pairs_length }
);
```

**Topic vector (length 2):**

```
topics = [
    String("SoroswapFactory"),
    Symbol("new_pair"),
]
```

**Data shape:**

```
data = NewPairEvent {
    token_0:           Address,   // C-address (token contract)
    token_1:           Address,   // C-address (token contract)
    pair:              Address,   // C-address (newly deployed pool)
    new_pairs_length:  u32,       // running pool count
}
```

The factory also emits `("SoroswapFactory", "init")`,
`("SoroswapFactory", "fee_to")`, `("SoroswapFactory", "setter")`, and
`("SoroswapFactory", "fees")`, but those are governance events and not
relevant to pool enumeration.

**Note on topic shape:** consistent with the wider-sample observation
in `R-swap-topic-shapes.md` — Soroswap uses a _2-topic_ form where
`topics[0]` is a `String` (`"SoroswapFactory"` here, `"SoroswapPair"` /
`"SoroswapRouter"` / `"SoroswapAggregator"` elsewhere) and `topics[1]`
is a `Symbol` naming the operation. The §7 filter in
`amm-trades-schema.md` must accommodate this `String + Symbol` shape.

## Spot-check vs observed emitters

### `String("SoroswapPair")` pool sample — CONFIRMED Soroswap

- **Pool contract:** `CAM7DY53G63XA4AJRS24Z6VFYAFSSF76C3RZ45BE5YU3FQS5255OOABP`
  (sample from `evidence/soroswap_pair_swap_sample.json`).
- **WASM on stellar.expert:** `18051456816b66f12e773a56f77c5794fac1b1fb7ab6e22d4fad5a412770f73e`
  — **exact match** to `hashes.pair` in `soroswap/core` mainnet contracts.
- **Verdict:** This is a canonical Soroswap pool, deployed by the
  Soroswap factory. The wider-sample finding that `String("SoroswapPair")`
  topic is Soroswap is verified by WASM identity, not just topic name.
- Acceptance criterion "At least one `String("SoroswapPair")` emitter
  cross-checked against a Soroswap factory derivation" — **MET** by
  WASM-hash equality with the factory's published `pair` template.

### Top 5 `Symbol("swap")` emitters — NONE are Soroswap

For all five top emitters, the contract ID and WASM hash do **not**
match any Soroswap canonical address or the Soroswap pair WASM. So the
literal `Symbol("swap")` events from the wider sample are **not**
emitted by Soroswap — Soroswap's swap events are wrapped in
`String("SoroswapPair")` / `String("SoroswapRouter")` /
`String("SoroswapAggregator")` topics, exactly as
`R-swap-topic-shapes.md` reported.

| Observed contract        | stellar.expert verdict | Notes                                                                                                                                                |
| ------------------------ | ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CBQDHNBFBZYE…` (11,947) | **VERIFIED Aquarius**  | `validation.status: "verified"`, repository `github.com/AquaToken/soroban-amm`, package `soroban-liquidity-pool-router-contract`, commit `c4d842de…` |
| `CBHCRSVX3ZZ7…` (4,128)  | unverified             | wasm `167ab414…f506c`, creator `GCNPDMUMRX…` (same as `CBCZGGNO…`); not Soroswap                                                                     |
| `CCR2CH4G…` (2,706)      | unverified             | wasm `48b28121…07c117`, creator `GCFB64LD…`; not Soroswap                                                                                            |
| `CDMIM23W…` (2,480)      | unverified             | wasm `4edd745f…32b6c8d`, creator `GC7RIP4D…`; not Soroswap                                                                                           |
| `CBCZGGNO…` (440)        | unverified             | wasm `167ab414…f506c` — **same WASM as CBHCRSVX3ZZ7…**; same creator `GCNPDMUMRX…`; both look like a paired Aquarius family member                   |

**Cross-task signal:** the verified Aquarius hit on `CBQDHNBFBZYE…`
positively attributes the _original_ sole `Symbol("swap")` emitter
from the 3.5-day window (per `R-swap-topic-shapes.md` it was
`CBQDHNBFBZYE…` then too) to **Aquarius's
`soroban-liquidity-pool-router-contract`**. That answers acceptance
criterion 1 _via the Aquarius fork_, not Soroswap. Recording it here
because the spot-check produced the evidence — the
`R-aquarius-registry.md` note should re-cite this stellar.expert
verification.

The two contracts sharing wasm `167ab414…f506c` (`CBHCRSVX3ZZ7…` and
`CBCZGGNO…`) and creator `GCNPDMUMRX…` are a strong signal that they
are part of a different protocol family, possibly Aquarius pools — to
be confirmed in `R-aquarius-registry.md`.

## Sources

- [soroswap/core public/mainnet.contracts.json](https://github.com/soroswap/core/blob/main/public/mainnet.contracts.json) (retrieved 2026-05-08)
- [soroswap/aggregator public/mainnet.contracts.json](https://github.com/soroswap/aggregator/blob/main/public/mainnet.contracts.json) (retrieved 2026-05-08)
- [soroswap/core contracts/factory/src/event.rs](https://github.com/soroswap/core/blob/main/contracts/factory/src/event.rs)
- [Soroswap.Finance Docs — Deployments](https://docs.soroswap.finance/01-protocol-overview/03-technical-reference/deploy-soroswap-yourself/04-deployments) (page redirected; values matched via GitHub instead)
- [stellar.expert API: contract/CA4HEQTL2…](https://api.stellar.expert/explorer/public/contract/CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2)
- [stellar.expert API: contract/CAG5LRYQ5…](https://api.stellar.expert/explorer/public/contract/CAG5LRYQ5JVEUI5TEID72EYOVX44TTUJT5BQR2J6J77FH65PCCFAJDDH)
- [stellar.expert API: contract/CAM7DY53G…](https://api.stellar.expert/explorer/public/contract/CAM7DY53G63XA4AJRS24Z6VFYAFSSF76C3RZ45BE5YU3FQS5255OOABP) — Soroswap pair sample WASM match
- [stellar.expert API: contract/CBQDHNBFBZYE…](https://api.stellar.expert/explorer/public/contract/CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK) — Aquarius router (verified)
