# S — Who emits `swap`, and what `pool_registry` is missing

**Measured 2026-09-17**, production, `dev_read`, live era ledgers
63,494,982 → 64,439,313 (2026-07-16 → 09-15) unless a narrower window is named.
Classification from `default.wasm_interface_metadata` (function names + doc
strings) and from same-transaction co-occurrence.

## ⛔ Correction first: the task's headline counts used a broken filter

`default.soroban_events.signature` is **not reliable for string-topic events.**
In the window, 272,144 of 310,829 events whose `topic[0]` is a *string* have
`signature` NULL, and the rest carry `topic[1]` instead. Soroswap
(`["SoroswapPair","swap"]`) and most Phoenix swaps (`String("swap")`) are
string-topic, so `signature = 'swap'` saw only a sliver of them. The "96.7% of
swaps come from unregistered contracts" headline is therefore **overstated**:
registered Soroswap and Phoenix pools emitted far more swaps than it counted.

✅ **Aquarius is unaffected** — its `trade` topic is a symbol, all 659,362
events carry `signature = 'trade'`, so [[0282]]'s aquarius numbers stand.

**Filter by topic instead:** `JSONExtractString(topics_xdr, 1, 'value')` for
`topic[0]`, index 2 for `topic[1]` (ClickHouse JSON indexes are 1-based).

## The stored-exceeds-raw contradiction — explained (instrument, not data)

| venue | old raw (`signature`) | corrected raw | stored `trade_count` | lost |
| --- | --- | --- | --- | --- |
| soroswap | 6,942 | **51,589** (`topic[1]='swap'`) | 28,980 | **43.8%** |
| phoenix | 2,960 | **~4,194** (one `sender` event per swap group) | 3,675 | **~12%** |

Both now read as the [[0282]] mechanism, not as candles from unknown pools.
Stored is the 2026-07-16 → 09-16 `FINAL` sum, so the windows differ by a day at
most.

## Classification

| wasm | contracts | swap events | what it is | evidence | action |
| --- | --- | --- | --- | --- | --- |
| `06F4207B` | 2 | 258,798 | **Aquarius router** | `swap(user, tokens, token_in, token_out, pool_index, …)`, `get_pool(tokens, pool_index)`; 87.6% of its swap txs also hold a registered Aquarius `trade`, the rest an **unregistered** Aquarius pool's `trade` (below); all 258,798 events have the router topic shape (`Vec` at `topic[1]`) | ✅ **ignore** — already filtered by `is_aquarius_router_swap`; indexing it would double-count |
| `003710B3` | 46 | 16,213 | **Uniswap-v3-style concentrated-liquidity pools** | constructor `(factory, token0, token1, fee, tick_spacing, flash_executor, admin)`; event `swap {amount0, amount1, liquidity, recipient, sender, sqrt_price_x96, tick}` | 🔴 **missing trades** — a venue we do not index |
| `95A8E001` | 4 traded (~60 deployed) | 720 | second code version of the same pool family | same constructor/doc, `flash_begin`, `slot0`, `tick_spacing` | 🔴 same as above |
| `9F94C577` | 1 (`CD3KRKGD…GLYF`) | — | that family's **factory** | `create_pool`, `create_and_initialize_pool`, "fee tiers matching Uniswap V3"; emits `pool_created {fee, pool_address, sender, …}` | the registration hook for the new venue |
| `4EDD745F` | 1 | 5,792 | router for the v3-style family | `swap_exact_input_single`, "Following Uniswap V3's permissionless design" | ✅ ignore — 1,380 / 1,380 sampled txs also hold a pool swap |
| `9677074A` | 1 | 1,726 | router for the v3-style family (older) | same interface | ✅ ignore — 454 / 454 txs also hold a pool swap |
| `4C3DB3EB` | 1 | 3,109 | Soroswap-style router | `swap_exact_tokens_for_tokens`, `router_pair_for` | ✅ ignore — 3,015 / 3,015 txs also hold a Soroswap pool swap |
| `136742A3` | 2 | 36 | small tick-based pool (`tick_mode`, `swap_exact_out`) | interface | immaterial |
| 10 others | 1 each | ≤ 29 each, 67 total | unclassified | — | immaterial |

(Co-occurrence sampled on ledgers 64,200,000 → 64,439,313.)

🔑 **Venue name for the v3-style family is not yet known** — the factory is
`CD3KRKGDRVWPXVB3VXLUMQKMX6XZ6Q2H334IVZD4XXNAMKSRVQL5GLYF`, first pools at
ledger ~61.49M (2026-04). Identify it before scoping the extractor.

## 🔴 The registry is stale: every pool created after 2026-07-06 is missing

`prices.pool_registry` was last written **2026-08-11**, by the history backfill,
whose data ends **2026-07-06**. The live processor only **reads** it
(`prices-ledger-processor/src/main.rs:80-94`). Pools created later are known to
the live path only through factory events it sees **while a container is warm**
(`learn_factory`, in memory) — and forgotten on every cold start.

Pools with the registered venues' own code but absent from the table:

| venue | contracts | events, live era | created |
| --- | --- | --- | --- |
| aquarius (`12FCA5A7`, `F1077E0B`, `AE0DA5A8`) | **22** | **34,684 `trade`** (~5% of Aquarius) | 2026-07-07 → 09-12, all after 07-06 |
| soroswap (`18051456`) | **10** | 267 `swap` | ledgers 63.40M → 63.80M, all after 07-06 |

**Consequences:**

1. **Live drops their trades after any cold start**, silently. An unknown
   contract's `trade` is not even recorded as unresolved (the guard only
   counts `swap`), and the live path never writes `unresolved_pools` anyway.
   How much actually reached candles is **unmeasured** — it depends on
   container lifetimes.
2. ⚠️ **[[0286]] phase 3, precondition 9 — answered: YES**, the live path can
   write candles for pools absent from the table. A registry-only AMM reprice of
   a live-era month would drop them. **Refresh the registry before phase 3
   touches 2026-07 onwards.**
3. [[0282]]'s aquarius raw count omits these 22 pools, so it undercounts by up
   to ~5%. After the fix, a small **stored > raw** is expected and is this, not
   an over-count.

## Answer to "missing, double-counting, or neither?"

- **Missing:** yes, two sources.
  - the v3-style venue — **16,933 swaps** over two months, never indexed;
  - pools created after 2026-07-06 — **34,684 Aquarius trades + 267 Soroswap
    swaps** at risk on every cold start.
- **Double-counting:** **none today.** The routers are either filtered
  (Aquarius) or unknown to us (the rest). They must stay out of any registry.
- **Fit as a measurement denominator?** **Not as-is** — it is stale, and the
  `signature` column cannot be used to count string-topic venues.
