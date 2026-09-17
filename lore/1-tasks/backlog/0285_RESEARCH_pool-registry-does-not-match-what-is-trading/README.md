---
id: "0285"
title: "prices.pool_registry does not match what is actually trading — 72 of 178 swap-emitting contracts are unregistered, and they carry 96.7% of the swap events"
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0282", "0101", "0096", "0097", "0099", "0080"]
tags: [layer-indexing, priority-medium, effort-medium, amm, soroswap, phoenix, ingestion, data-correctness, clickhouse]
links:
  - "../../active/0282_BUG_aquarius-live-ingestion-drops-half-its-trades/README.md"
  - "../../blocked/0101_FEATURE_live-era-amm-reprice-gap/README.md"
history:
  - date: 2026-09-15
    status: backlog
    who: okarcz
    note: >
      Found while trying to size [[0282]]'s damage for soroswap and phoenix. The
      raw-vs-stored instrument that works exactly for aquarius returned nonsense
      for them — 27,897 trades STORED against 6,942 raw — which is impossible if
      the registry described what trades. It does not: over the live era 178
      distinct contracts emit `swap` and only 106 are in `prices.pool_registry`.
      Filed as RESEARCH, not BUG: the unregistered contracts have NOT been
      classified, and the two plausible readings (pools we never index vs
      routers/aggregators reusing the event name) have opposite fixes — one is
      missing data, the other would be double-counting.
---

# The pool registry does not describe what is actually trading

## Summary

`prices.pool_registry` is the set of AMM pools we believe exist. Measured on
production 2026-09-15 over the live era (ledgers 63,494,982 → 64,439,313,
2026-07-16 → 09-15), it does **not** match the set of contracts actually
emitting swap events, and the mismatch is not marginal — **96.7% of all `swap`
events in the window come from contracts the registry has never heard of.**

This is **not** [[0282]]. 0282 is about candle writes replacing each other after
extraction. This is about swaps that may never reach extraction at all. It was
found *because* 0282's sizing work needed a raw-vs-stored number for soroswap
and phoenix and could not get a coherent one.

⚠️ **Filed as RESEARCH deliberately.** The unregistered contracts are
unclassified, and the two readings have opposite consequences:

- **pools we do not index** → we are silently missing their trades entirely,
  and the estate is short by more than 0282 accounts for;
- **routers / aggregators** that emit `swap` while the underlying pool emits its
  own event → adding them would **double-count** every trade they touch.

Do not act on the counts below until they are classified.

## Evidence — production, `dev_read`, 2026-09-15

### The instrument fails in a way that proves the premise

The comparison that reproduces [[0282]]'s aquarius numbers to the row returns an
impossible result for the other two venues:

| venue | raw (registry-joined) | stored `trade_count` | reading |
| --- | --- | --- | --- |
| aquarius | 659,365 | 346,915 | coherent — 47.4% lost |
| soroswap | 6,942 | **27,897** | ⛔ stored 4x the raw |
| phoenix | 2,960 | **3,602** | ⛔ stored exceeds raw |

Stored cannot exceed raw. Either the registry-joined raw is an undercount, or
the live path writes candles for pools the registry does not contain — and both
of those are this task.

### Registered vs actually trading

| venue | registered | registered AND traded in the window |
| --- | --- | --- |
| aquarius | 488 | 221 |
| soroswap | 221 | 99 |
| phoenix | 19 | 7 |

(Registered-but-idle is normal and not a problem. The problem is the other
direction.)

### 🔑 The other direction: who emits `swap`

```
contracts emitting `swap`, live era:   178
  ... of which in pool_registry:       106   (99 soroswap + 7 phoenix)
  ... unregistered:                     72

swap events, live era:             296,413
  ... from registered contracts:     9,902   (3.3%)
  ... from unregistered contracts: 286,511   (96.7%)
```

### Grouped by contract code — this is what makes it classifiable

`wasm_hash` separates "a pool family" from "one busy contract" cleanly:

| wasm_hash (prefix) | contracts | swap events | registered as | reads like |
| --- | --- | --- | --- | --- |
| `06F4207B` | **2** | **258,809** | — | ⚠️ 2 contracts, 87% of all swaps — a **router/aggregator**, almost certainly not a pool |
| `003710B3` | **46** | 16,214 | — | ⚠️ 46 contracts sharing one wasm — reads like a **pool family we do not index at all** |
| `18051456` | 105 | 6,983 | soroswap | ✅ soroswap's pool wasm |
| `4EDD745F` | 1 | 5,792 | — | single busy contract |
| `4C3DB3EB` | 1 | 3,109 | — | single busy contract |
| `167AB414` | 6 | 2,400 | phoenix | ✅ phoenix pool wasm |
| `9677074A` | 1 | 1,727 | — | single busy contract |
| `95A8E001` | 4 | 720 | — | small family |
| `F74D87D7` | 1 | 560 | phoenix | ✅ second phoenix wasm version |

🔑 **The shape does most of the work.** The two dominant unregistered groups are
qualitatively different: `06F4207B` is 2 contracts doing 258k swaps (a routing
contract's signature — enormous volume, no pool count), while `003710B3` is 46
contracts sharing a code hash (a pool family's signature). They almost certainly
need opposite treatment.

## Implementation

- **Classify `06F4207B` first** — it is 87% of the volume and the answer decides
  whether this task is about missing data or about avoiding double-counting.
  Read the contract's interface (`default.wasm_interface_metadata`,
  `soroban_contract_metadata`) and inspect a sample event's topics against the
  known venue shapes ([[soroban-events-gotchas]] — swap has 3 shapes, and
  soroswap's action is in `topic[1]`, not `topic[0]`).
- **Then `003710B3` (46 contracts).** If it is a pool family, identify the venue
  and decide whether it belongs in `pool_registry` — that is a new extractor, so
  scope it as its own task rather than growing this one.
- **Establish how the registry is populated today and why it drifts.** Soroswap
  and phoenix are factory-discovered ([[amm-historical-pool-discovery-gap]],
  [[amm-live-pool-registry-preload-gap]]); aquarius is seeded. A registry that
  only grows at deploy time will always trail a live chain.
- **Check the reverse direction too** — whether the live path writes candles for
  pools absent from `pool_registry`, which is one of the two explanations for
  stored-exceeds-raw above. The live processor builds its own registry view;
  if it is *broader* than the table, then the table is the wrong instrument for
  every measurement anyone makes with it, including [[0101]]'s.
- ⚠️ **All 488 aquarius and all 19 phoenix registry rows have empty
  `token0`/`token1`** (only soroswap's 221 are populated). Recorded in [[0282]]
  as not causing that defect — the tokens ride in the event topics — but it is
  more evidence the table is not maintained as a source of truth.

## Acceptance Criteria

- [ ] `06F4207B` (2 contracts, 258,809 swaps) is classified: pool, router, or
      something else — with the evidence that settles it.
- [ ] `003710B3` (46 contracts, 16,214 swaps) is classified the same way.
- [ ] The remaining unregistered emitters are classified or explicitly dismissed
      as immaterial, with their event counts.
- [ ] A statement of whether we are **missing trades**, at risk of
      **double-counting**, or neither — and how many trades that is worth.
- [ ] The stored-exceeds-raw contradiction for soroswap and phoenix is explained.
- [ ] It is recorded whether `prices.pool_registry` is fit to be used as the
      denominator in raw-vs-stored measurements, since [[0282]] and [[0101]]
      both rely on it.
- [ ] Any follow-up work (a new venue extractor, a registry-refresh mechanism)
      is spawned as its own task rather than absorbed here.

## Notes

- ⛔ **Nothing here changes [[0282]]'s aquarius numbers.** Aquarius is measured
  through `signature = 'trade'` on 488 registered contracts and its raw counts
  reproduce 0282's own table exactly. This task is about soroswap, phoenix, and
  whatever the unregistered contracts turn out to be.
- ⚠️ **[[0101]] should not run until at least the first two criteria are
  answered.** It reprices soroswap and phoenix over this same live era using
  `pool_registry` as its pool set, so if that set is wrong its output is wrong
  in the same direction — and its acceptance criteria are stated in terms of
  counts drawn from it.
- The signatures differ by venue and it matters for every query here: aquarius
  emits `trade`, soroswap and phoenix emit `swap`. A single-signature filter
  silently returns one venue's data.
