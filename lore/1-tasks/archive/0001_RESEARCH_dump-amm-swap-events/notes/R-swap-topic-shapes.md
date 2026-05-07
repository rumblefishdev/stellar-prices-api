---
title: "Soroban swap-like contract event topic shapes observed in mainnet ledgers 62016000–62079999"
type: research
status: developing
spawned_from: ../README.md
spawns:
  - S-amm-trades-schema-§11-1-resolved.md
tags: [soroban, amm, cap-67, schema-validation]
links:
  - "../../../../../docs/database-schema/amm-trades-schema.md"
  - "evidence/swap_event_sample.json"
  - "evidence/trade_event_sample.json"
history:
  - date: 2026-05-07
    status: seed
    who: okarcz
    note: "Captured from running tools/dump-swap-events against .temp/FC4DB5FF--62016000-62079999"
---

# Soroban swap-like contract event topic shapes (ledgers 62016000–62079999)

## Source

Sample: 614 zstd-compressed `LedgerCloseMetaBatch` files in
`.temp/FC4DB5FF--62016000-62079999/` (mainnet sequence 62016000–62079999;
~3.5-day window). Tool: `tools/dump-swap-events` (this repo, lore task 0001).

Aggregate counts (Diagnostic-source events excluded; consensus tx-level +
per-op only):

| Metric | Value |
|---|---|
| Files scanned | 614 / 614 |
| Total contract events seen | 2,738,082 |
| Distinct `topic_0` symbols | 45 |

## Swap-like topic_0 symbols observed

Top of the histogram restricted to topics that *might* represent an AMM
trade:

| `topic_0` | Hits | Distinct emitters | Pattern signal |
|---|---:|---:|---|
| `trade` | 91 | 29 | Pool-level: 29/29 of these contracts also emit `update_reserves` |
| `swap` | 53 | **1** | Factory/router: same contract also emits `add_pool` (2) and `config_rewards` (44) |
| `SwappedFromVUsd` | 1 | 1 | Different protocol — virtual-USD synthetic, not a constant-product AMM |

Phoenix-style swap topics were **not observed** in this window. This is not
proof of absence project-wide; the sample is 3.5 days. See "Open questions"
below.

## `Symbol("swap")` — single-contract router pattern

**Emitter:** `CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK`
(only one in the entire sample window).

**Topic vector** (length 3):

```
topics = [
    Symbol("swap"),
    Vec [
        Address(token_in),         // C-address
        Address(token_out),        // C-address
    ],
    Address(trader),               // G-address (account)
]
```

**Data** — `Vec` of 5 ScVal entries:

```
data = Vec [
    Address(pool),                 // C-address — pool contract for this hop
    Address(token_a),              // C-address (matches topics[1].vec[0])
    Address(token_b),              // C-address (matches topics[1].vec[1])
    U128(amount_in),
    U128(amount_out),
]
```

Concrete sample: `evidence/swap_event_sample.json` (ledger 62079939, tx
`27f3a225d6...`). The pool referenced in `data[0]` was
`CCNXGPE4AQCSNEBZO3XJDKKDI3CRLYMVS6UWBBTVDLALLWMJEXBORQ2A` — the same pool
appeared as `data[0]` in every `swap` event sampled, suggesting either a
single hot pool OR (more likely) a router that emits one consolidated swap
event referencing the underlying pool per hop.

**Co-emitted topics by the same contract in the window:**
`add_pool` (2) and `config_rewards` (44). These are factory/governance
event names, not pool-level. The emitter is therefore acting as an
AMM **factory + router**, not as a pool. Concrete venue attribution
(Aquarius router vs. Soroswap router vs. another aggregator) is a
follow-up — see `S-amm-trades-schema-§11-1-resolved.md`.

## `Symbol("trade")` — multi-contract pool pattern

**Emitters:** 29 distinct contracts. Every one of those 29 also emits
`update_reserves` events in the same window (29/29 overlap). Two
additional contracts emit `update_reserves` without ever emitting `trade`
in this window (i.e. liquidity-only activity, no swaps).

**Topic vector** (length 4):

```
topics = [
    Symbol("trade"),
    Address(token_in),             // C-address
    Address(token_out),            // C-address
    Address(trader),               // G-address (account) or C-address
]
```

**Data** — `Vec` of 3 ScVal entries:

```
data = Vec [
    I128(amount_in),
    I128(amount_out),
    I128(fee),                     // amount denominated in token_in (~bps of amount_in)
]
```

Concrete sample: `evidence/trade_event_sample.json` (ledger 62079996, tx
`7f785bf7d2...`, contract `CA6PUJLBYK...`). Values:
`amount_in = 25_761_941_491`, `amount_out = 3_901_204_480`,
`fee = 12_880_971`. The fee is ~0.05% of `amount_in`, consistent with a
constant-product AMM fee charged on the in-leg.

**Type difference vs. `swap`:** `trade` uses **`I128`** (signed) while
`swap` uses **`U128`** (unsigned). Both can be safely treated as
non-negative for accounting purposes, but the schema's `NUMERIC(28,14)`
column accepts either since the DB type is the same.

## `Symbol("SwappedFromVUsd")` — out of scope

Single hit, single emitter (`CAOTMWRKNMV5GW...`). PascalCase symbol,
`data` is a `Map` keyed by `amount` / `fee` / `recipient` / `token` /
`vusd_amount`. This is a virtual-USD synthetic swap (consistent with
Allbridge or a similar bridge protocol), **not** a constant-product
AMM trade. Pre-filter must reject it.

## Open questions for venue attribution

These are **not** answerable from raw event shape alone — they need a
known-address registry:

1. Which venue (Soroswap / Aquarius / Phoenix) does the single
   `swap`-emitter `CBQDHNBFBZYE...` belong to?
2. Which venue do the 29 `trade`-emitters belong to? (Single venue, or
   a mix?)
3. Why does Phoenix not appear in this window? Sample too short, or
   does Phoenix emit under a different topic again?

Each is a candidate for a follow-up backlog DOCS/RESEARCH task — see
`S-amm-trades-schema-§11-1-resolved.md` §"Future work".

## Reproduction

```bash
cd /home/oski/Projects/stellar/stellar-prices-api/tools/dump-swap-events
cargo build --release
cd ../..
./tools/dump-swap-events/target/release/dump-swap-events \
    --dir .temp/FC4DB5FF--62016000-62079999 \
    --histogram                                    # full topic_0 histogram

./tools/dump-swap-events/target/release/dump-swap-events \
    --dir .temp/FC4DB5FF--62016000-62079999 \
    --symbol trade                                 # one JSON line per trade event

./tools/dump-swap-events/target/release/dump-swap-events \
    --dir .temp/FC4DB5FF--62016000-62079999 \
    --symbol swap --pretty                         # pretty-printed swaps
```
