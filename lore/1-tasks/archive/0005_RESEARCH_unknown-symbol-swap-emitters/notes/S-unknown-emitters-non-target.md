---
title: 'Unknown Symbol("swap") emitters confirmed non-target — exclude from indexer'
type: synthesis
status: mature
spawned_from: ../README.md
spawns: []
tags: [soroban, amm, venue-attribution, indexer-policy]
links:
  - '../../0002_RESEARCH_amm-venue-attribution/notes/S-venue-attribution-mapping.md'
  - '../../0002_RESEARCH_amm-venue-attribution/notes/R-aquarius-registry.md'
  - '../../0002_RESEARCH_amm-venue-attribution/notes/R-soroswap-registry.md'
  - '../../0002_RESEARCH_amm-venue-attribution/notes/R-phoenix-registry.md'
history:
  - date: 2026-05-11
    status: mature
    who: okarcz
    note: >
      Manual verification confirms all unknown Symbol("swap") emitters
      from 0002 are non-target (not Soroswap, not Aquarius, not Phoenix).
      Policy decision: exclude from the indexer.
---

# Unknown `Symbol("swap")` emitters — confirmed non-target

## Decision

**Exclude. Do not track.**

All `Symbol("swap")` contract emitters that task 0002 left unattributed
have been manually verified by the developer (okarcz) and confirmed to
NOT belong to any of the three target venues:

- **Soroswap** — not present in `soroswap/core` or
  `soroswap/aggregator` `mainnet.contracts.json`; WASM does not match
  the canonical Soroswap pair hash
  (`18051456816b66f12e773a56f77c5794fac1b1fb7ab6e22d4fad5a412770f73e`).
- **Aquarius** — not the Aquarius router
  (`CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK`); WASM
  hash does not match the canonical Aquarius constant-product pool
  hash (`ae0da5a84b15805c5c7931ac567a8d1b34be3f26b483993d9ff80cb2c3de9852`);
  creator does not match Aquarius deployer.
- **Phoenix** — not in the Phoenix factory's deployed-pool list
  (factory: `CB4SVAWJA6TSRNOJZ7W2AWFW46D5VR4ZMFZKDIKXEINZCZEGZCJZCKMI`).

## Excluded addresses

These addresses are explicitly **excluded** from per-venue tracking by
the BE indexer. They emit `Symbol("swap")` like a router/aggregator but
belong to neither Soroswap, Aquarius, nor Phoenix.

|     Events (4-day window) | Contract                                                   | Verified   |
| ------------------------: | ---------------------------------------------------------- | ---------- |
|                     2,706 | `CCR2CH4GQVCZHG7CHFVMNANCK45CU5DVKXZIIITDZQAU3CEJZ7RQH2MQ` | non-target |
|                     2,480 | `CDMIM23WOUL5CZBKX3GOA3V5R5AMVIMTCP52KCDQORWELAPLJ27WZCHL` | non-target |
|                       335 | `CCXRRORTOXXP53HEKJ6RCG7CDRWZAJHIS4N7PDL32PUNMNN7VWPJVQWS` | non-target |
|                       229 | `CAUF4DFYSX52L2KJ4J7OFW3WDQMEUDVXNB7PG5VIC4VVOA3BCLWXDO2E` | non-target |
| (+ ~30 more in long tail) | (see 0002 S-note §"Cross-check vs 0001 sample")            | non-target |

Source for the full ranked list: `../../0002_RESEARCH_amm-venue-attribution/notes/S-venue-attribution-mapping.md`.

## Indexer policy

The indexer applies a **known-target allowlist** for `Symbol("swap")`
emitters: only the Aquarius router
(`CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK`) qualifies
for that topic kind.

Concretely:

- `Symbol("swap")` from the Aquarius router → decode as Aquarius
  (see 0002 §"Cross-check vs 0001 sample").
- `Symbol("swap")` from any other emitter → **drop**. Do not emit a
  trade row, do not record as `venue: unknown`.
- `String("swap")` from Phoenix XYK pools → decode as Phoenix
  (unchanged from 0002).
- `String("SoroswapPair")` / `String("SoroswapRouter")` /
  `String("SoroswapAggregator")` → decode as Soroswap (unchanged).
- `Symbol("trade")` from Aquarius constant-product pools → decode as
  Aquarius (unchanged).

This keeps the `prices_amm_trades.venue` column constrained to the
three required values (`soroswap`, `aquarius`, `phoenix`) with no
`unknown` bucket.

## Rationale for excluding rather than indexing as `venue: unknown`

The original 0005 backlog brief left two options open:

1. Skip (do not index).
2. Index as `venue: unknown` for visibility.

Manual verification chose **(1) skip**. Reasons:

- The unknown contracts are not part of the indexer's product scope
  (Soroswap / Aquarius / Phoenix). Indexing them would expand scope
  without a stakeholder ask.
- Allowing a `venue: unknown` bucket complicates the schema constraint
  on `prices_amm_trades.venue` and risks future drift if additional
  unknown emitters appear.
- The 4-day event volume from these unknowns (~5,200 events) is not
  load-bearing for any current downstream consumer.

## Effect on prior tasks

- **0002** — per-venue mapping is now closed. The "unknown bucket"
  noted in `S-venue-attribution-mapping.md` §"Future work" item 1 is
  resolved as **exclude**.
- **0003** (DOCS) — when the schema doc is updated, it should state
  that the indexer's emitter allowlist is strict and that unknown
  `Symbol("swap")` emitters are dropped (not bucketed).
