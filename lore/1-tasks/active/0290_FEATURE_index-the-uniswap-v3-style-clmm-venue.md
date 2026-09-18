---
id: "0290"
title: "Index the Uniswap-v3-style concentrated-liquidity venue (factory CD3KRKGD…) — ~8.5k swaps a month we never see"
type: FEATURE
status: active
assignee: okarcz
related_adr: []
related_tasks: ["0285", "0286", "0282"]
tags: [layer-indexing, priority-medium, effort-medium, amm, ingestion, data-correctness]
links:
  - "../active/0285_RESEARCH_pool-registry-does-not-match-what-is-trading/notes/S-classification-2026-09-17.md"
history:
  - date: 2026-09-17
    status: backlog
    who: okarcz
    note: >
      Spawned from 0285's classification. A pool family with its own factory
      emits Uniswap-v3-shaped swaps (16,933 over 2026-07-16 → 09-15) and no
      extractor of ours recognises it.
  - date: 2026-09-18
    status: active
    who: okarcz
    note: >
      Activated. Picked up because 0291 is blocked on 0286's schema and 0282 /
      0285 both wait on the 2026-09-19 measurement; this is the last known
      missing-trades gap from 0285 and is independent of the deploy freeze
      (research + extractor + tests; nothing to deploy until 0286 phase 1
      lands). Start by naming the venue and decoding its swap event.
---

# Index the Uniswap-v3-style concentrated-liquidity venue

## Summary

Since ~2026-04 (ledger ~61.49M) a factory at
`CD3KRKGDRVWPXVB3VXLUMQKMX6XZ6Q2H334IVZD4XXNAMKSRVQL5GLYF` (wasm `9F94C577`)
deploys concentrated-liquidity pools (wasm `003710B3`, newer `95A8E001`). Over
the live era they emitted **16,933 swaps** that reach no candle. See
[[0285]]'s note for the evidence.

## Findings — 2026-09-18 (production, `dev_read`)

### 🔑 The venue is **SushiSwap V3** (Sushi on Soroban)

Settled on-chain, not by inference. The factory's deployer
`GC7RIP4D7UETKBZU7FNUYW5XIR2IPLUQDV2DUTZRKVF6C3P2A5QEDSZI`
(`soroban_contracts.deployer_id = 427269037949309133`) also deployed contracts
whose `soroban_contract_metadata` reads **`name = "SushiSwap V3 Positions
NFT-V1"`, `symbol = "SUSHI-V3-POS"`** — the position manager of a standard
Uniswap-v3 deployment set.

The live set went out together at ledgers **61,472,396 → 61,472,663**:

| contract | wasm | role |
| --- | --- | --- |
| `CDKKPFBNNQETVN4Y2CIU6CQMWCPIZDXDZE76IDSTF7HLQT6Q573PCDTH` | `272253DE` | (helper, deployed first) |
| `CD3KRKGDRVWPXVB3VXLUMQKMX6XZ6Q2H334IVZD4XXNAMKSRVQL5GLYF` | `9F94C577` | **factory** |
| `CAUF4DFYSX52L2KJ4J7OFW3WDQMEUDVXNB7PG5VIC4VVOA3BCLWXDO2E` | `9677074A` | router ⛔ do not index |
| `CDMIM23WOUL5CZBKX3GOA3V5R5AMVIMTCP52KCDQORWELAPLJ27WZCHL` | `4EDD745F` | router (ledger 61,595,074) ⛔ do not index |
| `CARTUL5AWDZYBSN7HUUJZSKCAKCIAKM7M54Z76G6KRYCK4XPR3OHUQZ4` | `BE969DE5` | **SushiSwap V3 Positions NFT-V1** ← the name |

Four earlier rehearsal deployments exist (ledgers 60.14M, 60.29M, 60.75M,
61.28M), each with its own Positions NFT — only the 61.47M set is live, which
matches 0285's "first pools at ~61.49M".

⚠️ Public launch coverage mentions PYUSD/USDC and XLM/USDC as the first pools;
the live deployment is much wider (below). Do not scope the extractor from the
announcement.

### `pool_created` carries the token pair — no storage read needed

```json
topics = [sym "pool_created"]
data   = {fee: u32, pool_address: address, sender: address,
          tick_spacing: i32, token0: address, token1: address}
```

So the pair is learned exactly like Soroswap's `new_pair`, through
`learn_factory`. This closes the task's open "factory event carries it? contract
storage?" question: **the factory event carries it.**

### The pairs are mainstream Stellar assets, not a niche corner

Pools per token across all `pool_created` events, resolved through
`prices.assets`:

| token | pools | | token | pools |
| --- | --- | --- | --- | --- |
| USDC | 18 | | BLND | 3 |
| XLM | 15 | | SHX | 3 |
| DAWG | 15 | | AFR | 3 |
| LIBRE | 13 | | MBC | 3 |
| AQUA | 7 | | Apay | 3 |
| ACT | 5 | | XTAR | 2 |
| yXLM | 4 | | | |

### The gap is **88,689 swaps**, not 16,933 — and it starts 2026-01

0285's figure was its two-month sampling window. All-time, across pool wasms
`003710B3` + `95A8E001`:

| month | swaps | pools | | month | swaps | pools |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-01 | 28 | 6 | | 2026-06 | 14,823 | 47 |
| 2026-02 | 1,688 | 26 | | 2026-07 | 8,841 | 45 |
| 2026-03 | 7,866 | 42 | | 2026-08 | 9,572 | 48 |
| 2026-04 | 12,782 | 40 | | 2026-09 | 6,007 | 37 |
| 2026-05 | **27,116** | 49 | | **total** | **88,689** | **99 traded** |

Ledgers 60,770,886 → 64,488,316. **Exactly one event shape** across all 88,689 —
no variants to handle.

### ⚠️ Only 53 of the 99 traded pools come from the live factory

| origin | swaps all-time | since 63M | since 64M | pools active since 64M |
| --- | --- | --- | --- | --- |
| live factory `CD3KRKGD…GLYF` | 84,236 (95%) | 34,596 | 13,070 | **45** |
| earlier factory generations | 4,453 (5%) | 1,324 | **583** | **3** |

So registering only the live factory covers 95% of volume and 45 of 48 currently
active pools — but **three earlier-generation pools are still trading** (583
swaps since ledger 64M). They are not dormant, and history needs all of them.

⛔ **Open:** the earlier factories are not yet identified. `soroban_contracts.
deployer_id` is **NULL for factory-deployed pools**, so the cheap route does not
work, and a `topics_xdr LIKE '%pool_created%'` scan over the whole era exceeds
`dev_read`'s 30 s limit — chunk it, as 0291 had to. Candidate factory wasms are
among the deployer's earlier batches (`272253DE`, `148CA1A9`, `391D449E`,
`068104A7`, `CD86E132`, …, one per rehearsal generation).

**Alternative worth weighing:** seed the registry by **pool wasm hash**
(`003710B3` / `95A8E001`) in a one-off, and have live learn only from the live
factory. That covers every pool regardless of which generation made it, and
reuses [[0291]]'s `--discover-pools` shape.

### Price comes from the amounts; `sqrt_price_x96` is a free cross-check

Worked example, pool `CCR2CH4G…H2MQ` (token0 **XLM**, token1 **USDC**) at ledger
64,488,316 — `amount0 = -10,002,738,052`, `amount1 = +1,886,660,000`,
`tick = -16,710`, `sqrt_price_x96 = 0x6f0630935bf3444987646d6e`:

| derivation | price (USDC/XLM) | meaning |
| --- | --- | --- |
| `(sqrt_price_x96 / 2⁹⁶)²` | 0.18808545 | pool's marginal price **after** the swap |
| `1.0001^tick` | 0.18807462 | same number on the tick grid (**−0.006%**) |
| `\|amount1\| / \|amount0\|` | 0.18861436 | the **executed** price (+0.281%: fee + impact) |

- **Use the amounts** for the tick, as every other AMM extractor does. The
  0.006% agreement between `tick` and `sqrt_price_x96` is proof the decode is
  right — keep it as a test fixture, not as the price source.
- **Direction from the signs:** positive = into the pool (trader sold it),
  negative = out of the pool (trader bought it).
- Amounts are raw `i128` in each token's own decimals, which is exactly the form
  ADR 0287's price-forming rule expects ([[0286]]) — no pre-scaling needed
  before the bound is evaluated.

### Gotchas confirmed for the query path

- `soroban_events.contract_id` is an **Int64 surrogate**, not the `C…` string —
  join through `default.soroban_contracts.id` ([[soroban-events-gotchas]]).
- SAC tokens are **absent from `default.soroban_contract_metadata`**; resolve
  codes through `prices.assets` (`contract_address` OR `sac_address`).
- `stellar.expert`'s directory has **no entry** for the factory or the deployer,
  and its contract endpoint reports `validation: unverified` — the deployer's
  other contracts were the only on-chain route to the name.

## Context

- Pool constructor: `(factory, token0, token1, fee, tick_spacing,
  flash_executor, admin)`; factory emits `pool_created {fee, pool_address,
  sender, …}`.
- Pool event: `topics = [Symbol("swap")]`, `data = {amount0, amount1,
  liquidity, recipient, sender, sqrt_price_x96, tick}` — signed amounts, so
  direction comes from the sign.
- Three routers wrap these pools (`4EDD745F`, `9677074A`) and must **not** be
  indexed: every one of their swap txs also holds a pool swap.

## Implementation

- **Identify the venue by name** first — it decides the `source` label.
- Learn pools from `pool_created` (like `learn_factory` does for the other
  venues) and persist them ([[0291]]).
- Token pair comes from the pool's constructor/storage, not from the swap event
  — decide how to resolve it (factory event carries it? contract storage?).
- Map the swap to a tick using the signed amounts; decide whether price comes
  from amounts or from `sqrt_price_x96`, in line with ADR 0287's price-forming
  rule ([[0286]]).
- Backfill from the family's first pool, in the same run as [[0286]] phase 3
  if the timing allows.

## Acceptance Criteria

- [x] The venue is named and has a `source` label. → **SushiSwap V3**, settled
      on-chain via the deployer's `SushiSwap V3 Positions NFT-V1` contracts
      (see Findings). Label decided with the operator 2026-09-18:
      **`source = "sushiswap"`** — the full brand, as the contract metadata
      spells it, and unversioned like every other source (Aquarius's three pool
      types already share one label). `Venue::Sushiswap` in
      `extractors-core/src/lib.rs`.
- [ ] Its pools are learned from the factory and survive a cold start.
- [ ] Live candles for it match a raw count of its pool `swap` events for a full
      day.
- [ ] Its routers stay unindexed (a test pins it).
- [ ] History from the first pool is backfilled, or explicitly deferred with a
      reason.
