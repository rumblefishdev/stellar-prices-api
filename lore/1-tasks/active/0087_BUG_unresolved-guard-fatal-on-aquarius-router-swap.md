---
id: "0087"
title: "unresolved-pools guard fatal-fires on Aquarius router `swap` events — blocks the combined backfill"
type: BUG
status: active
related_adr: ["0001"]
related_tasks: ["0053", "0080", "0060"]
tags: [layer-indexing, amm, aquarius, soroban, backfill, extractor, clickhouse]
links:
  - "../../../packages/prices-ingest-core/src/soroban.rs"
  - "../../../packages/aquarius-extractor/src/lib.rs"
  - "../backlog/0080_RESEARCH_aquarius-concentrated-pool-swap-event-shape.md"
history:
  - date: 2026-07-07
    status: active
    who: okarcz
    note: >
      Spawned from the 0053 combined-backfill validation tranche
      ([50457424, 50687999]), which fatal-exited on "3 unresolved AMM pools."
      Investigation identified the events as Aquarius ROUTER `swap` summaries
      (topic[0]=swap, topic[1]=Vec([tokenA,tokenB]), topic[2]=swapper) —
      deliberately ignored to avoid double-counting the pool-level `trade`, but the
      guard flags them → false-positive FATAL that blocks a clean full run.
  - date: 2026-07-07
    status: active
    who: okarcz
    note: >
      Steps 1 & 4 done (commit 4aef1f1). Added `is_aquarius_router_swap()` to
      `classify_amm_groups`: a `swap` with an address `Vec` at topic[1] (the
      router shape) is skipped before the unresolved-pools guard records it; a
      genuine unknown-pool `swap` (no Vec topic) still trips it. Regression test
      `router_swap_is_not_flagged_but_genuine_pool_swap_still_is` asserts both +
      no candle (double-count boundary). 32/32 crate tests pass. Both code-side
      ACs met. Starting Step 2 (underlying-pool coverage).
  - date: 2026-07-07
    status: active
    who: okarcz
    note: >
      Step 2 CLOSED via targeted archive fetch (2 endpoint ledgers, no full
      re-run). Decisive: at the tranche's early Soroban epoch the underlying
      pools emit NO events — the router `swap` (data = [pool, tin, tout, amt_in,
      amt_out]) is the SOLE trade record. So `amm_ticks: 0` is genuine bounded
      volume loss, NOT a discovery/seed bug. The sample's "router redundant with
      pool `trade`" holds only for the later epoch (62.0M, 14/14). Early-epoch
      coverage gap → 0080 (accept router as priceable / new extractor). Guard
      fix still correctly unblocks the backfill. AC3 checked. Only Step 3 (full
      re-run, no-fatal confirm) remains.
---

# Aquarius-router `swap` false-positive fatals the combined backfill

## Summary

The combined backfill's unregistered-pool guard (0053 decision #3) fatal-exits
when it meets the **Aquarius router's `swap` summary event** — a deliberately
ignored aggregator wrapper, not a liquidity pool. The router contract is never
factory-registered, and the event's `topic[0] == Symbol("swap")`, so the guard
records it to `prices.unresolved_pools` and fails the run. It is a **false
positive** blocking a clean full `[activation, tip]` backfill.

## Evidence (0053 tranche, 2026-07-07)

3 contracts (`CANMWW5D…`, `CBVSLUYH…`, `CDVTDAUA…`, ledgers 50639018–50686276,
swap_count 16/1/1) emitted the identical shape:

```
topic[0] = Symbol("swap")
topic[1] = Vec([ Address(tokenA), Address(tokenB) ])
topic[2] = Address(G… swapper)
```

This matches the captured Aquarius-router sample in
`lore/4-notes/samples/soroban-events/swap.jsonl:3` (the emitter is named "the
Aquarius router" at `aquarius-extractor/src/lib.rs:13-16`; its `data` vec leads
with the underlying **pool** address then `[tokenA, tokenB, amount_in,
amount_out]` — a router summarizing a pool-level trade). No extractor matches
this shape, **by design** — matching it would double-count the pool `trade`.

## Root cause

`classify_amm_groups` (`prices-ingest-core/src/soroban.rs:308-330`): for a
contract absent from `reg.venue`, the `None` branch records ANY event with
`topic[0] == Symbol("swap")` as an `UnresolvedPoolSwap`. Routers are never
registered (they are not factory-created pools — `learn_factory` only registers
`add_pool` / `create` / `new_pair` emitters), so their `swap` summaries **always**
land in `unresolved_pools` and trip the fatal guard.

## The second question — is real volume actually missing?

The tranche logged **`amm_ticks: 0`** (zero resolved AMM candles). The real
Aquarius volume comes from the underlying **pool's `trade` event**, not the
router. If those pools were not discovered (created before the window, or their
`add_pool` not caught), their `trade` volume is **silently dropped** — the guard
only records `swap`, not `trade`, so a missing pool `trade` never even shows in
`unresolved_pools`. So this task must also **confirm whether the underlying
Aquarius pool `trade` volume is captured** for these pairs. If not, that is a
genuine coverage gap (discovery/seeding — ties to 0080), distinct from the guard
false positive.

## Implementation Plan

### Step 1 — Silence the router false positive
Narrow the guard in `classify_amm_groups` so a `swap` whose `topic[1]` is a `Vec`
of addresses (the known Aquarius-router shape) is **not** recorded as unresolved
(or keep a small known-router allowlist). Single-function change in
`prices-ingest-core`; no new extractor, no dispatch change.

### Step 2 — Verify underlying Aquarius pool coverage
For the 3 router pairs, confirm the underlying pool `trade` events are captured
(pool in `reg.venue` → resolved to candles). If missing, find the pool
create-ledger: in-window (should auto-register via `add_pool` — investigate why
it didn't) vs pre-window (seed `pool_registry` — coordinate with 0080).

### Step 3 — Re-run the tranche
Re-run combined `[50457424, 50687999]`; assert the guard does **not** fatal on the
router swaps, and (if Step 2 found a gap) that Aquarius trades now resolve.

### Step 4 — Tests
Unit: the guard skips a router `swap` (Vec `topic[1]`) but still flags a genuine
unknown-pool `swap` (non-Vec shape). Regression guard on the double-count boundary
(router event must not produce a candle).

## Acceptance Criteria

- [x] The unresolved-pools guard no longer fatals on Aquarius-router `swap` events.
      (commit 4aef1f1 — `is_aquarius_router_swap()` filter in `classify_amm_groups`)
- [x] Genuine unregistered-pool `swap`s still trip the guard (safety net intact).
      (regression test asserts a non-Vec `swap` is still recorded as unresolved)
- [x] Confirmed whether the 3 pairs' underlying Aquarius pool `trade` volume is
      captured; if not, root-caused (in-window discovery bug vs pre-window seed
      gap → 0080). **Confirmed via targeted archive fetch of the two endpoint
      ledgers:** the underlying pools emit NO events at this early epoch — the
      router `swap` is the sole trade record. So `amm_ticks: 0` is genuine volume
      loss, NOT a discovery/seed bug. Root cause = early-epoch AMM surfaces trades
      only through the router summary (no pool `trade`), which no extractor handles.
      → coverage gap owned by **0080** (accept router as priceable for the early
      window / new extractor). The sample's "router redundant with `trade`" holds
      only for the later epoch (62.0M, 14/14). Guard fix still correctly unblocks.
- [ ] Tranche `[50457424, 50687999]` re-runs clean (no fatal) → **full combined
      run unblocked**.

## Blocks

- **0053** — the full combined `[activation, tip]` run cannot complete cleanly
  until the guard stops fatal-firing on router swaps (it fatal-exited on this
  2 h validation tranche).

## Step 2 Findings (2026-07-07) — router-redundancy CONFIRMED; tranche gap deferred to Step 3

Analysed the captured samples in `lore/4-notes/samples/soroban-events/`
(`swap.jsonl` + `trade.jsonl`, window 62078346–62079999):

1. **`data[0]` is the underlying pool** (confirms the task's claim). Of the 50
   `swap` rows, 32 are router-shape (Vec `topic[1]`), all from one router
   `CBQDHNB…`; they reference **14 distinct pools** in `data[0]`.
2. **14/14 of those pools also emit the canonical `trade`** event, shape
   `[Symbol("trade"), Address, Address, Address]` — exactly what
   `AquariusPoolExtractor` resolves. So in steady-state data the router `swap`
   is **provably redundant** with a captured, resolvable pool `trade`: the guard
   fix drops nothing real, and matching the router *would* double-count. ✅
3. **A separate genuine gap exists but is NOT this task:** the 18 non-router
   single-topic `swap`s carry a concentrated-liquidity map
   (`amount0/amount1/liquidity/sqrt_price_x96/tick`) — Uni-v3 style, unhandled
   by any extractor. Those would land in `unresolved_pools` legitimately. This is
   the **0080** concentrated-pool question, distinct from the router false-positive.

### Targeted-fetch result (2026-07-07) — DECISIVE, and it revises the above

Fetched the two endpoint ledgers of the tranche's router swaps straight from the
public archive (`aws s3 cp --no-sign-request`, one `.xdr.zst` per ledger) and
dumped **every** non-diagnostic event with a throwaway `xdr-parser` example
(BE crate, reused `extract_events`; removed after):

- **L50639018** — router `CBVSLUYH…` `swap`, `data[0]` pool
  `CC7LUVAF…`, topic[1] `Vec([CBDIIPX3…, CB3I5U7S…])`, amounts `100000000 →
  98911870`.
- **L50686276** — router `CANMWW5D…` `swap`, `data[0]` pool
  `CCKWA3RE…`, topic[1] `Vec([CAS3J7GY…, CAUIKL3I…])`, amounts `10000000 →
  514069487`.

**The underlying pools emit NOTHING.** Across both full ledgers (2071 events;
signatures `transfer 1528, fee 534, set_authorized 4, approve 2, swap 2,
burn 1`) there is **no `trade`, no `sync`, no `add_pool`**, and the pool
addresses `CC7LUVAF…`/`CCKWA3RE…` emit **zero** events. Each router-swap tx is
just `fee` + `approve` + token `transfer`s + the router `swap`. So at this early
Soroban epoch (Protocol-20 launch, ~2024-02/03) **the router `swap` is the SOLE
machine-readable trade record** — its data already carries
`[pool, token_in, token_out, amount_in, amount_out]`, a complete trade.

**Therefore `amm_ticks: 0` is genuine, bounded volume loss — not merely a guard
false-positive.** The Step-2-sample "router is redundant with a captured pool
`trade`" conclusion holds **only for the later epoch** (62.0M, where pools *do*
emit `trade` — 14/14). Early-epoch AMM trades surface *only* via the router
`swap`, which no extractor handles, so their volume is silently dropped. The
"do-NOT-match-the-router → double-count" rule is **epoch-dependent**: matching it
early would NOT double-count (there is no pool `trade` to double).

Root cause of `amm_ticks: 0` = **not** an in-window `add_pool` discovery bug and
**not** a pre-window seed gap — it is that this epoch's AMM exposes trades only
through the router summary. This is a coverage gap that belongs with **0080**
(Aquarius/router swap-event shapes across epochs) — a *new-extractor / accept the
router as a priceable source for the early window* decision, distinct from and
larger than 0087's guard fix.

**Bounds the gap:** volume is lost only for the window `[activation .. pools begin
emitting `trade`]`; the later epoch (verified at 62.0M) resolves normally via
`trade`. Quantifying that cutover ledger is 0080/0053 follow-up work.

The guard fix remains correct and sufficient to **unblock** the backfill (stops
the fatal); the early-epoch volume completeness is a separate, now-root-caused
gap. Step 3 (full re-run) will confirm no-fatal end-to-end and let the operator
measure how much early-epoch AMM volume the router-only path represents.

## Notes

- **Do NOT** add a swap-matcher for the router event — it would double-count the
  pool-level `trade`. The router event is correctly ignored for pricing; the only
  defect is the guard mis-flagging it as a gap.
- SDEX extraction on the tranche was fully correct (~21.5M ticks → ~12.9M candles
  over `[50457424, 50687999]`); this task is AMM-only.
