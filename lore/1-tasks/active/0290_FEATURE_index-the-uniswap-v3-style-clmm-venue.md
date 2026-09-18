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

✅ **Resolved 2026-09-18 (afternoon)** — see "Every pool is announced by a
`pool_created`" below: four factory generations, found with a chunked scan on
`signature = 'pool_created'` (under 1 s per 640k-ledger chunk).

**🗄️ SUPERSEDED the same afternoon — see "✅ DECIDED: discover from
`pool_created`" below.** Kept for the record:

~~**DECIDED with the operator 2026-09-18 — seed by pool wasm hash.**~~ A one-off
discovers every pool whose wasm is `003710B3` / `95A8E001`, whichever factory
generation created it, and live learns new pools from the live factory
`CD3KRKGD…GLYF` going forward. Rationale: it covers all 99 traded pools without
first identifying the dead rehearsal factories (an archaeology exercise whose
only cheap route — `deployer_id` — is NULL for factory-deployed contracts), and
it reuses [[0291]]'s `--discover-pools` shape, which was proven on production
this morning. The rejected alternative was enumerating and registering every
factory generation.

⚠️ Consequence to keep in view: a pool created by a **future** rehearsal or
replacement factory would be missed by live, exactly as [[0291]]'s pools were.
[[0291]]'s `UnregisteredPoolEvents` alarm is the backstop — extend its shape
matcher to this venue's `swap` so a missed pool is heard, not silent.

### Every pool is announced by a `pool_created` — measured 2026-09-18 (afternoon)

A chunked read of `signature = 'pool_created'` over ledgers 60.0M → 65.1M
(8 × 640k chunks, each under 1 s; ClickHouse fills `signature` for this event
because its topic is a Symbol) returns **138 events**:

| emitter (the `sender` field) | wasm | pools | ledgers |
| --- | --- | --- | --- |
| `CAGRET7K…APTC` | `FC9B0DF0` | 8 | 60,147,305 → 60,239,179 |
| `CBBXISWE…K5RI` | `FC9B0DF0` | 6 | 60,286,788 → 60,433,518 |
| `CCRSMJDI…GZGF` | `FC9B0DF0` | 61 | 60,770,878 → 61,263,178 |
| `CD3KRKGD…GLYF` (live) | `9F94C577` | 58 | 61,487,379 → 64,116,662 |
| `CBMBKXI7…Q227` — **not SushiSwap** | `ED0D122C` | 5 | 63,173,214 → 63,173,222 |

All four SushiSwap factories share the SushiSwap deployer
(`deployer_id = 427269037949309133`); together they announce **133 pools**.

- **Coverage is complete.** All **119** contracts on pool wasms `003710B3` (58)
  and `95A8E001` (61) are among the 133 — zero on those wasms unannounced.
- **The other 14 are real SushiSwap pools on older wasms** — `144EF710` (8) and
  `C1AA54E8` (6), from the two earliest rehearsal factories. Their `swap`
  payload is the identical shape (`amount0, amount1, liquidity, recipient,
  sender, sqrt_price_x96, tick`), **143 swaps**, the last at ledger 62,417,110 —
  dormant. The 88,689 figure above counted only the two newer wasms, so it
  misses these 143.
- **A second protocol emits `pool_created`.** One contract, `CBMBKXI7…Q227`
  (wasm `ED0D122C`, a different deployer), created five fixed-denomination
  pools with data `{denomination, generation, pool}` and a second topic. No
  token pair, so `learn_factory`'s arm (which requires `pool_address`,
  `token0` and `token1`) registers none of them. Pinned by a test.

**✅ DECIDED with the operator 2026-09-18 (afternoon) — discover from
`pool_created`, not by pool wasm hash.** This replaces the morning's
wasm-hash decision. That decision rested on "without first identifying the
dead rehearsal factories"; the factory-event read needs no identification — it
has no emitter filter, as `learn_factory` has none — and it finds a
**superset**: the 119 the wasm-hash seed would find, plus the 14 older-wasm
pools it would miss. It also yields the token pair, which a wasm-hash list
alone does not. Commit `291c21c` extends `--discover-pools` to `pool_created`;
the wasm-hash seed was never built and will not be. A
`--discover-pools --dry-run` over 60,000,000 → tip is expected to report
**133 new `sushiswap` pools**.

This also shrinks the morning's "future factory" consequence: live learns a
`pool_created` by shape from **any** emitter, not only `CD3KRKGD…GLYF`, so a
replacement factory emitting the same event is learned without a code change.
[[0291]]'s `UnregisteredPoolEvents` alarm remains the backstop for one that
changes the event.

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

## Implementation Notes

Branch `feat/0290_index-the-uniswap-v3-style-clmm-venue`, commit `51a3030`.
**Workspace 1,118 passed, 0 failed** (1,115 before, +3 new tests).

- **`extractors-core`**: `Venue::Sushiswap` → `"sushiswap"`, both directions.
- **`soroswap-extractor` is now venue-neutral rather than copied.** Its
  `decode_swap` ALREADY handled this exact CLMM shape (signed
  `amount0`/`amount1`), and `swap_action` already returns `"swap"` for a bare
  `[Symbol("swap")]` topic — the only venue-specific thing was the hardcoded
  `venue: Venue::Soroswap` on the emitted row. So the types were generalised to
  `TokenPair` / `PairPoolRegistry` / `PairSwapExtractor` with `Soroswap*` kept
  as aliases (no call site changed), and the extractor carries a `venue`.
  `PairSwapExtractor::new` still means Soroswap; `with_venue` is the new path.
  A second extractor crate would have been a divergent copy of tested code.
- **`learn_factory`**: a `pool_created` arm. It matches on SHAPE like every
  other arm — no factory address is hardcoded anywhere in the codebase — which
  is why one arm covers all four factory generations. Requires all three
  addresses (`pool_address`, `token0`, `token1`), so a differently-shaped
  `pool_created` from another protocol cannot register a tokenless pool.
- **Separate registries.** `Registries.sushiswap` is its own
  `PairPoolRegistry` instance, so a contract_id can never resolve to the other
  pair-backed venue's tokens. `pool_count()` includes it.
- **`dispatch`** gained a fifth parameter and a `Venue::Sushiswap` arm.
- **`registry_io`**: `to_pool_rows` / `load_pool_rows` arms, so pools persist
  and survive a cold start through [[0291]]'s write path unchanged.
- **The unresolvable-pair guard** (`classify_amm_groups`) was Soroswap-only;
  it is now a `match` over venue, so a SushiSwap pool whose pair is unknown is
  recorded in `unresolved` instead of vanishing.
- **`unregistered_pool_venue`** keys on the DATA (`amount0` + `amount1`) via a
  new `has_data_key`, not on the topic — see Issues.

**Negative controls** (each new test must fail without its code):

| control | result |
| --- | --- |
| `pool_created` arm removed from `learn_factory` | `sushiswap_factory_pool_created_learns_the_pair` FAILS ✅ |
| shape matcher keyed on topic only | the new router test AND the pre-existing `routers_and_unindexed_venues…` both FAIL ✅ |

**`--discover-pools` (commit `291c21c`, `events-backfill/src/discover.rs`).**
`'pool_created'` added to the read's `signature IN (…)` filter — the only change
the read needed. Tests: the real-payload test now covers all four venues'
factory shapes; a new pool is written with venue `sushiswap`; the other
protocol's real `pool_created` learns nothing. The SQL-pinning test
`the_read_matches_every_shape_learn_factory_accepts` was updated to the new
filter text (intentional). `events-backfill` + `prices-ingest-core`: 126 passed,
0 failed.

**Router test (commit `7021ae2`).**
`a_routed_sushiswap_trade_prices_once_from_the_pool_not_the_router`
(`prices-ingest-core/src/soroban.rs`) drives a real routed trade — ledger
64,481,111, pool `CCR2CH4G…H2MQ` swap at event 4 and router `CDMIM23W…ZCHL`'s
summary of the same amounts at event 5 — through `process_soroban_event_rows`.
It requires exactly one `sushiswap` tick, no router in the registry, and no
missing-pool count; and a router wrongly registered as a SushiSwap pool must
still add no tick (its data has no `amount0`/`amount1`). Negative controls:
giving the router pool-shaped data fails the missing-pool assertion, and with
that assertion removed fails the tick count at `left: 2`. ✅ Both failed.

A router `swap` from an unregistered contract still lands in `out.unresolved`
(the generic "unknown contract emitted a `swap`" record). That is a record,
not a failure: the backfill never reads unregistered contracts, and nothing
trips on it. Left as is and not pinned by the test.

**Modified test:** `routers_and_unindexed_venues_are_not_counted_as_unregistered_pools`
(`soroban.rs`) previously asserted this venue is NOT counted, with a bare `swap`
row named `clmm`. That premise is now wrong — we index it. The row was replaced
with a real SushiSwap **router** payload (`amount_in`/`amount_out`), which must
still be uncounted, and the bare no-amounts case was kept. No assertion was
weakened: the test still asserts `None` for every row in the list.

## Issues Encountered

- 🔴 **The routers share the pools' topic exactly.** Both emit
  `topics = [Symbol("swap")]`; only the data differs —
  `{ amount_in, amount_out }` for a router against
  `{ amount0, amount1, liquidity, sqrt_price_x96, tick }` for a pool. A shape
  matcher keyed on the topic alone counts **every routed swap** as a missing
  pool, and 0285 measured 5,792 + 1,726 router swaps whose transactions each
  also hold the pool swap they wrap. Caught before writing the matcher, by
  reading a real router event; negative control 2 pins it.
- The whole-era `topics_xdr LIKE '%pool_created%'` scan exceeds `dev_read`'s
  30 s limit. Filtering by emitter contract id first (factories are few) makes
  the same question cheap — the general lesson from [[0291]] applies.
- `soroban_contracts.deployer_id` is **NULL for factory-deployed contracts**, so
  it cannot be used to attribute a pool to its factory. The pool's exact
  `deployed_at_ledger` plus a single-ledger event filter works instead, and is
  how the earlier factory wasm `FC9B0DF0` was found.
- A **full** scan does not exceed the limit when it filters on `signature`
  rather than `topics_xdr LIKE`: under 1 s per 640k-ledger chunk.
- `pool_created` is **not unique to SushiSwap** — see the second protocol under
  Findings. Matching by shape stays safe only because the arm requires the
  token pair.
- Pre-existing, not from this task: `cargo clippy -p events-backfill -- -D
  warnings` fails on 9 lints inside `prices-ingest-core`, identically without
  this change; CI does not lint that crate. Lint with `--no-deps`.

## Acceptance Criteria

- [x] The venue is named and has a `source` label. → **SushiSwap V3**, settled
      on-chain via the deployer's `SushiSwap V3 Positions NFT-V1` contracts
      (see Findings). Label decided with the operator 2026-09-18:
      **`source = "sushiswap"`** — the full brand, as the contract metadata
      spells it, and unversioned like every other source (Aquarius's three pool
      types already share one label). `Venue::Sushiswap` in
      `extractors-core/src/lib.rs`.
- [ ] Its pools are learned from the factory and survive a cold start.
      → Code done: live learns from `pool_created` (`51a3030`),
      `--discover-pools` seeds history from it (`291c21c`, 133 pools measured),
      `registry_io` persists them. Open: the production dry run, then the write
      and a cold start — after 0286 phase 1 lifts the Compute deploy freeze.
- [ ] Live candles for it match a raw count of its pool `swap` events for a full
      day.
- [x] Its routers stay unindexed (a test pins it). →
      `a_routed_sushiswap_trade_prices_once_from_the_pool_not_the_router`
      (`7021ae2`), a real routed transaction, negative-controlled. See
      Implementation Notes.
- [ ] History from the first pool is backfilled, or explicitly deferred with a
      reason.

# 📕 RUNBOOK — `--discover-pools` dry run for SushiSwap V3

Read-only: `--dry-run` writes nothing, so it is safe during the Compute deploy
freeze. The real write only helps once 0290's ledger-processor is deployed
(after 0286 phase 1), so it waits for that.

**Run identity** is 0291's ([seed-pool-registry.md](../../../docs/runbooks/seed-pool-registry.md),
"Discover missing pools"): the Hetzner host, ClickHouse `default`,
`localhost:8123`, a static build.

1. **[local machine, repo root, branch `feat/0290_…`]** build and copy:

   ```bash
   cargo build --release -p events-backfill --target x86_64-unknown-linux-musl
   scp target/x86_64-unknown-linux-musl/release/events-backfill <prod-host>:~/events-backfill-0290
   ```

   A separate file name, so 0291's binary on the host is not overwritten.

2. **[prod host, under tmux]** the dry run. It starts at **60,000,000**, not
   0291's 63,000,000: the first SushiSwap factory event is at ledger 60,147,305.
   `<TIP>` = the latest ledger in `default.soroban_events`.

   ```bash
   read -rs CH_PW
   CLICKHOUSE_PASSWORD="$CH_PW" ~/events-backfill-0290 --discover-pools \
     --start 60000000 --end <TIP> \
     --clickhouse-url http://localhost:8123 --dry-run
   ```

   About 15 chunks of 320k ledgers at 2-4 s each: roughly a minute.

3. **[read the output]** Expected as of 2026-09-18:
   - one `pool not in prices.pool_registry` line per pool, every one
     `change="new"`, `venue="sushiswap"`;
   - then `to_write=133 per_venue={"sushiswap": 133}`.
   - More than 133 is fine if SushiSwap created pools after 2026-09-18.
   - ⛔ **Stop and investigate** on any `change="changed"` line (an existing
     row would be rewritten), or on a venue other than `sushiswap` (the other
     venues were seeded by 0291 on 2026-09-18 and should report 0).
   - The other protocol's five token-less `pool_created` events at ledger
     63.17M must **not** appear.

4. **[later, after 0286 phase 1 and 0290's deploy]** drop `--dry-run` to write,
   run the dry run again (it must report `to_write=0`), then verify:

   ```sql
   SELECT venue, count() FROM prices.pool_registry FINAL GROUP BY venue ORDER BY venue;
   -- expected: sushiswap 133 (or more), other venues unchanged
   ```
