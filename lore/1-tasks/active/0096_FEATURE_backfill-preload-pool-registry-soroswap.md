---
id: "0096"
title: "Soroswap backfill coverage gap — close dispatch silent-drop (root cause: registry seed-timing)"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0078", "0079", "0053", "0088", "0090"]
tags: [layer-indexing, priority-high, effort-medium, milestone-M1, backfill, clickhouse, amm, soroswap, pool-registry, data-loss]
milestone: 1
links:
  - "../../../packages/prices-clickhouse/schema/init.sql"
history:
  - date: 2026-07-15
    status: backlog
    who: okarcz
    note: >
      Surfaced during the 0090 pre-roll: prod coarse tables have ZERO soroswap
      candles despite the registry being seeded. The backfill-side twin of the
      live-only 0078 fix.
  - date: 2026-07-15
    status: active
    who: okarcz
    note: >
      Promoted to active to begin implementation (backfill pool_registry
      preload). Branch cut off develop.
  - date: 2026-07-15
    status: active
    who: okarcz
    note: >
      Root cause CONFIRMED = registry seed-timing, NOT a missing preload. The
      backfill startup already preloads pool_registry (venue + soroswap tokens)
      since task 0053 (commit 1582905). Prod check: all 221 soroswap rows have
      tokens, but were seeded 2026-07-14, AFTER the Soroban backfill run
      (~07-04..08), so reg.soroswap was empty at run time → 0 candles. Re-scoped:
      0096 = dispatch observability hardening + root-cause record; the
      operational Soroswap re-run moves to 0088.
---

# Soroswap backfill coverage gap — close dispatch silent-drop (root cause: registry seed-timing)

## Summary

The historical backfill produces **zero `soroswap` candles**, so the coarse
tables (and `price_ohlcv_1m`) have no Soroswap prices at all. Aquarius and
Phoenix are present; Soroswap is entirely missing.

**Root cause CONFIRMED (2026-07-15) = registry seed-timing, not a missing
preload.** The original framing — "the backfill has no equivalent preload" —
was **wrong**: `sdex-backfill` has preloaded `prices.pool_registry` (venue +
Soroswap token pairs) since task 0053 (`run.rs:128` → `sink.load_pool_registry`,
commit `1582905`). The real cause is that the Soroswap registry rows did not
exist when the Soroban-era backfill ran — see below.

This task is therefore **re-scoped** to (1) close the dispatch observability
hole that let the gap go silent, and (2) record the confirmed root cause. The
operational fix — re-running the Soroswap range now that the registry is seeded
— moves to task **0088** (the paused backfill run-tracker).

## Evidence (prod ch-prod-01, 2026-07-15)

- `price_ohlcv_1m` sources: `sdex` 530M, `aquarius` 2.14M, `phoenix` 156k,
  **`soroswap` 0**.
- `pool_registry FINAL` by venue: `aquarius` 488, `phoenix` 19, **`soroswap`
  221** — so the registry IS seeded (task 0079).
- `unresolved_pools FINAL`: 138 pools / 7,887 swaps, all `source='backfill'`;
  **`unresolved_that_are_soroswap = 0`**.

So Soroswap swaps are neither resolved (0 candles) NOR recorded as unresolved —
they are **invisible to the backfill**, not merely unattributed.

## Root cause (CONFIRMED 2026-07-15)

A Soroswap swap event **omits the pair tokens** (CLMM `Map{amount0, amount1}` or
uniswap-v2 `amount_N_in/out`, single `[Symbol("swap")]` topic — verified against
`lore/4-notes/samples/soroban-events/swap.jsonl`), so it can only become a candle
if the pool→tokens mapping is known at processing time, i.e. from
`prices.pool_registry` via `reg.soroswap`.

**The backfill startup already loads that registry** — `run.rs:128`
(`sink.load_pool_registry()`, not `Registries::new()`) → `writer.rs:106`
(`SELECT contract_id, venue, token0, token1, … FROM prices.pool_registry FINAL`)
→ `registry_io.rs:74` (`load_pool_rows` rehydrates `reg.venue` **and**
`reg.soroswap`). Added in commit `1582905` (task 0053 Step 3), before the live
0078 fix. So the preload is not the gap.

**The gap is timing.** Prod `ch-prod-01` check (2026-07-15):

- All **221** `soroswap` `pool_registry` rows have non-empty `token0/token1`
  (0 missing) — the registry is complete.
- But all 221 were seeded at **`2026-07-14 17:54:24`** — *after* the Soroban-era
  backfill run (~07-04..08). At run time `reg.soroswap` was **empty**, so no
  Soroswap swap could resolve → 0 candles.
- The 221 seeded contracts are **disjoint** from the 138 `unresolved_pools`
  contracts (join = 0), i.e. the run never even processed swaps for those pools
  in combined mode — consistent with the Soroban run being partial/paused (0088).

Aquarius/Phoenix survived the same run because their swap events carry tokens
inline and resolve via in-window forward-discovery, independent of the registry.

### Secondary code defect (in scope for this task)

`dispatch()` (`ledger-processor/src/dispatch.rs:89-92`) returns `Ok(vec![])` for
a `Venue::Soroswap` pool that is in `reg.venue` but **absent from `reg.soroswap`**
— no trade, no error. Back in `classify_amm_groups` (`soroban.rs:308`), because
the pool *was* found in `reg.venue`, it never reaches the `unresolved` branch. So
a venue-known-but-unresolvable Soroswap pool (or one whose swap fails to decode,
`soroban.rs:350` just `warn!`s) drops **silently — 0 candles AND 0 unresolved**.
That observability hole is why this gap was invisible until the 0090 pre-roll; it
must be closed so any future gap surfaces in `unresolved_pools`.

## Implementation

- **Close the silent-drop:** in `dispatch`/`classify_amm_groups`, a venue-known
  Soroswap pool that yields no trade (unresolvable pair, or decode error) must
  record an `unresolved_pools` row (or an equivalent diagnostic), never vanish.
- **Add a regression test** at the classify seam proving a Soroswap `swap` for a
  venue-known-but-`reg.soroswap`-absent pool produces an unresolved record
  (mirrors the existing 0078 seeded-vs-unseeded unit tests; no XDR fixture
  needed).
- **Operational re-run moved to 0088:** re-run the Soroswap-affected Soroban
  range in combined mode now that the registry is seeded (bounded, per the 0090
  runbook: disable cleanup → backfill → pre-roll → re-enable). Tracked there.

## Acceptance Criteria

- [x] Root cause confirmed: registry **is** preloaded; gap is seed-timing (rows
      seeded 2026-07-14, after the run) + a silent-drop observability hole. NOT
      registry-not-loaded.
- [ ] Silent-drop closed: a venue-known Soroswap pool that resolves to no trade
      records an `unresolved_pools`/diagnostic row (never invisible), with a
      regression test.
- [ ] Operational Soroswap re-run yields non-zero `soroswap` candles — tracked
      in **0088**, verified per-source there.

## Notes

- Does NOT block 0090 (pre-roll/cleanup are correct for the data that exists) —
  this is an upstream backfill-coverage gap. Related coverage memory:
  [[amm-historical-pool-discovery-gap]], [[amm-live-pool-registry-preload-gap]].
- Likely also affects the live path for any Soroswap pool not covered by 0078's
  preload; verify once live is unfrozen ([[proto27-xdr26-live-freeze]]).
- Operational Soroswap re-run handed to **0088** — the registry is now seeded
  (221 pools, 2026-07-14), so a combined-mode re-run over the Soroswap range will
  finally produce candles. The silent-drop fix here makes that re-run's coverage
  self-verifying (any still-unresolvable pool shows up instead of vanishing).
