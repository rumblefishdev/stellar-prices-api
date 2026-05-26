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
  - "evidence/soroswap_pair_swap_sample.json"
  - "evidence/soroswap_router_swap_sample.json"
  - "evidence/soroswap_aggregator_swap_sample.json"
  - "evidence/sw_v1_false_positive_sample.json"
  - "evidence/wider_sample_histogram.txt"
history:
  - date: 2026-05-07
    status: seed
    who: okarcz
    note: "Captured from running tools/dump-swap-events against .temp/FC4DB5FF--62016000-62079999"
  - date: 2026-05-07
    status: developing
    who: claude
    note: >
      Added "Update: wider sample" section with findings from a second
      run against .temp/FC47D9FF--62400000-62463999 (60,545 files, ~4
      days, 234M events). Identified Soroswap's actual topic pattern
      (String("Soroswap*") + Symbol(<op>)), the literal swap symbol now
      from 44 emitters (not 1), and reclassified sw_v1 as a false
      positive. Original 3.5-day section preserved verbatim.
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

---

## Update: wider sample (ledgers 62400000–62463999)

> **Why this section exists.** The original 3.5-day window was too narrow
> to surface several swap topics. A second run on a ~4-day window roughly
> 5 weeks later (ledgers 62400000–62463999) was added to validate and
> extend the findings. **The original 3.5-day section above is preserved
> verbatim** — this section *augments*, not replaces.

### Source

- Sample: 60,545 zstd-compressed files in `.temp/FC47D9FF--62400000-62463999/`
  (~9.9 GiB on disk).
- Run mode: `dump-swap-events --histogram` (4 m 49 s on cold cache),
  followed by targeted `--symbol <name>` runs for each candidate
  swap topic.
- Diagnostic-source events still excluded (consensus tx-level + per-op
  only).

| Metric | Value |
|---|---:|
| Files scanned | 60,545 / 60,545 |
| Files failed | 0 |
| Total contract events seen | 234,312,389 |
| Distinct `topic_0` strings (consensus only) | ~120 |

### Headline change vs. the 3.5-day window

The first window suggested only two AMM topic kinds (`swap`, `trade`).
The wider sample shows that was an artefact of low volume — at least
**five distinct swap-bearing topic patterns** are observable, and the
single-emitter pattern for `Symbol("swap")` was misleading.

| `topic_0` | First sample (3.5 d) | Wider sample (~4 d, 5 wks later) | New finding |
|---|---:|---:|---|
| `Symbol("swap")` | 53 / 1 emitter | **23,567 / 44 emitters** | **No longer single-router**: now a long-tail mix of routers + pools |
| `Symbol("trade")` | 91 / 29 emitters | 19,050 (volume grew, distinct count not enumerated this run) | Aquarius pattern stable |
| `Symbol("update_reserves")` | 98 / 31 emitters | 20,222 | Aquarius reserve updates, paired with `trade` |
| `String("SoroswapPair")` | 0 | **2,512** | **NEW** — Soroswap pool events; 79 distinct emitters; uses *String*, not *Symbol* |
| `String("SoroswapRouter")` | 0 | **885** | **NEW** — Soroswap router |
| `String("SoroswapAggregator")` | 0 | **18** | **NEW** — Soroswap multi-hop aggregator |
| `Symbol("SwappedFromVUsd")` | 1 | 34 | Allbridge virtual-USD; out of scope |
| `Symbol("SwappedToVUsd")` | 0 | 37 | **NEW** — Allbridge counterpart |
| `Symbol("sw_v1")` | 0 | 336 | **False positive** — see below |

### Soroswap topic shape (the one we couldn't see in the 3.5-day window)

Soroswap does **not** emit `Symbol("swap")` from its pool contracts.
Instead it uses a **two-topic** structure where `topics[0]` is a *String*
naming the contract role and `topics[1]` is a *Symbol* naming the
operation. This makes the §7 filter strictly more complex than the first
sample suggested — `topics[0]` alone is insufficient for Soroswap.

```
topics = [
    String("SoroswapPair") | String("SoroswapRouter") | String("SoroswapAggregator"),
    Symbol("swap") | Symbol("sync") | Symbol("deposit") | Symbol("withdraw") | …,
]
```

Sub-event distribution for `SoroswapPair` in this window
(`topics[0]=String("SoroswapPair")`):

| `topics[1]` | Count | Meaning |
|---|---:|---|
| `Symbol("sync")` | 1,256 | Reserve update (post-anything) — analogous to Aquarius `update_reserves` |
| `Symbol("swap")` | **1,241** | The actual Soroswap pool swap event |
| `Symbol("withdraw")` | 8 | LP withdrawal |
| `Symbol("deposit")` | 7 | LP deposit |

#### Soroswap pool swap data shape

`evidence/soroswap_pair_swap_sample.json` — pool
`CAM7DY53G63XA4AJRS24Z6VFYAFSSF76C3RZ45BE5YU3FQS5255OOABP`,
ledger 62460506:

```
data = Map {
    amount_0_in:  i128,
    amount_0_out: i128,
    amount_1_in:  i128,
    amount_1_out: i128,
    to:           Address,
}
```

This is the **classic Uniswap V2 `Pair.sol` Swap event** layout. Either
`amount_0_in` or `amount_1_in` is `0`; the non-zero one is the in-leg,
the other side's `_out` is the out-leg. Token addresses are not in the
event — they must be looked up from the pool contract's `token_0` /
`token_1`. (This is a meaningful change vs. Aquarius, where token
addresses are inline in the event.)

#### Soroswap router / aggregator shapes

- **`SoroswapRouter`** (`topics[1]=Symbol("swap")`): emits a single
  consolidated event for the user-facing swap (or multi-hop), data is a
  Map with an `amounts` Vec<i128> for the per-hop amounts. See
  `evidence/soroswap_router_swap_sample.json`.
- **`SoroswapAggregator`** (`topics[1]=Symbol("swap")`): top-level
  aggregator event for cross-protocol routing; data Map keyed by
  `amount_in`, etc. See `evidence/soroswap_aggregator_swap_sample.json`.

These two are convenient for indexing user-facing swap intent, but the
**authoritative on-chain trade data is in the `SoroswapPair` swap event**
emitted by the pool itself for each hop. The Prices API should consume
the pool-level event to avoid double-counting multi-hop swaps.

### `Symbol("swap")` revisited — no longer a single emitter

In this window 44 distinct contracts emit the literal `Symbol("swap")`.
Top emitters (events / contract):

| Events | Contract |
|---:|---|
| 11,947 | `CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK` (the original sole emitter from the 3.5-day window) |
| 4,128 | `CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX` |
| 2,706 | `CCR2CH4GQVCZHG7CHFVMNANCK45CU5DVKXZIIITDZQAU3CEJZ7RQH2MQ` |
| 2,480 | `CDMIM23WOUL5CZBKX3GOA3V5R5AMVIMTCP52KCDQORWELAPLJ27WZCHL` |
| 440 | `CBCZGGNOEUZG4CAAE7TGTQQHETZMKUT4OIPFHHPKEUX46U4KXBBZ3GLH` |
| (+ 39 more) | … |

The "factory/router pattern, single emitter" hypothesis from the first
window is **now wrong**. Reading: at least the top four are routers /
aggregators (high per-contract volume), and the long tail is more
varied. Venue attribution still needed (task 0002).

### `Symbol("sw_v1")` — false positive

336 events. Looks like a swap topic at first glance ("sw" → swap?) but
inspection shows otherwise:

```
topics = [Symbol("sw_v1"), Symbol("add"), Vec[Symbol("Ed25519"), Bytes(32)]]
```

`Symbol("add")` + an Ed25519 public key in `topics[2]` indicates **smart
wallet signer management**, not an AMM trade. `sw` is "smart wallet" or
"signer wallet", not "swap". Documented as a false positive so future
wider scans don't waste time re-investigating. See
`evidence/sw_v1_false_positive_sample.json`.

### Phoenix — still not directly identifiable

No PascalCase `Phoenix*` topics observed in this wider sample (matching
the first window). Possibilities:

1. Phoenix is hidden inside the 44 `Symbol("swap")` emitters. This is
   plausible — Phoenix's open-source code (per public reading) emits
   `Symbol("swap")` from its pool contracts.
2. Phoenix activity in this 4-day window was below the noise floor.
3. Phoenix uses a topic name we still haven't seen.

Resolution requires venue attribution (task 0002) — specifically,
matching the 44 swap-emitting contracts against Phoenix's known
factory/pool addresses.

### Updated implications for the schema

What changes for `prices_amm_trades` and the §7 filter:

1. **The `topics[0]`-only filter is insufficient for Soroswap.** The BE
   indexer's per-venue mapping must match the *contract_id set*, not just
   the topic symbol. For Soroswap pools specifically, the indexer must
   also drop `topics[1] != Symbol("swap")` to avoid emitting trade rows
   for `sync` / `deposit` / `withdraw`.
2. **Soroswap event data has no inline token addresses.** The indexer
   must look up `token_0` / `token_1` from the pool contract once and
   cache per-pool, then attach to each emitted trade row. This is
   different from the Aquarius `trade` decoder that reads token addresses
   from the event topics.
3. **Three distinct decoders are now empirically observed**, not two:
   - Aquarius pool: `Symbol("trade")` → 3-element `Vec<i128>` payload.
   - Aquarius router-style: `Symbol("swap")` → 5-element `Vec` with mixed
     `Address`/`U128` payload.
   - Soroswap pool: `String("SoroswapPair") + Symbol("swap")` →
     uniswap-v2-style `Map` payload.
   Plus likely a Phoenix decoder once attribution lands.

These do not change the DDL in §4 — the typed columns
`(token_in, token_out, amount_in, amount_out, venue, pair_contract_id)`
absorb all three shapes — but they substantially raise the importance of
the per-venue decoder dispatch documented in synthesis note
`S-amm-trades-schema-§11-1-resolved.md`. The recommendation in that note
to update `amm-trades-schema.md` §7 step 3 (task 0003) is reinforced.

### Reproduction (wider-sample run)

```bash
cd /home/oski/Projects/stellar/stellar-prices-api

./tools/dump-swap-events/target/release/dump-swap-events \
    --dir .temp/FC47D9FF--62400000-62463999 \
    --histogram                                    # full survey

./tools/dump-swap-events/target/release/dump-swap-events \
    --dir .temp/FC47D9FF--62400000-62463999 \
    --symbol SoroswapPair --pretty                 # Soroswap pool events

./tools/dump-swap-events/target/release/dump-swap-events \
    --dir .temp/FC47D9FF--62400000-62463999 \
    --symbol SoroswapRouter --pretty               # Soroswap router
```
