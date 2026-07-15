---
id: "0096"
title: "Soroswap coverage gap — extractor reads the swap action from the wrong topic"
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
  - date: 2026-07-15
    status: active
    who: okarcz
    note: >
      ROOT CAUSE SUPERSEDED (final) — it is an EXTRACTOR bug, not seed-timing.
      BE's soroban_events (full history, activation→63.49M) shows our 221 pools
      emit 824k swaps shaped `topics=[String("SoroswapPair"), Symbol("swap")]` —
      the action is in topic[1], with the constant "SoroswapPair" in topic[0].
      Our SoroswapPairExtractor read the action from topic[0] (== "swap"), so it
      never matched → Ok(vec![]) → 0 candles AND 0 unresolved (the classify guard
      also keys topic[0]). Seed-timing was a red herring: a re-run would still
      emit nothing. Fix (this branch): recognize the SoroswapPair envelope
      (action from topic[1]); data map is already the uniswap-v2 shape we decode.
      Historical fill now a CH-to-CH derivation from soroban_events (no ledger
      re-download); tracked in 0088.
  - date: 2026-07-15
    status: completed
    who: okarcz
    note: >
      DONE + archived. Extractor fix (soroswap-extractor swap_action → topic[1])
      + observability guard merged in PR #112 (2c53ee4); +4 tests, workspace/
      clippy/fmt clean. Ledger-processor redeployed to prod ~15:57Z and VERIFIED:
      `soroswap` now a live source in price_ohlcv_1m, flowing in lockstep with
      sdex/aquarius/phoenix. Historical fill of ~824k past swaps spawned as task
      0097 (CH-to-CH reprice from BE soroban_events). Root cause: extractor read
      the swap action from topic[0] but Soroswap uses [String("SoroswapPair"),
      Symbol("swap")] — action in topic[1]; earlier missing-preload + seed-timing
      hypotheses both disproven.
---

# Soroswap coverage gap — extractor reads the swap action from the wrong topic

## Summary

Everything (`price_ohlcv_1m` + coarse tables) has **zero `soroswap` candles**.
Aquarius and Phoenix are present; Soroswap is entirely missing.

**FINAL root cause (2026-07-15) = an extractor bug: we read the swap action from
the wrong topic.** Two earlier hypotheses were both wrong: it is **not** a
missing preload (the backfill has preloaded `pool_registry` since 0053), and
**not** seed-timing (that was a red herring — a re-run would still emit nothing).

Diagnosed against BE's full-history `soroban_events` (activation → 63.49M): our
221 seeded pools emit **824k swaps** shaped

```
topics = [ String("SoroswapPair"), Symbol("swap") ]   ← action in topic[1]
data   = Map{ amount_0_in, amount_0_out, amount_1_in, amount_1_out, to }
```

`SoroswapPairExtractor` recognised the action in **topic[0]** (`== "swap"`), but
Soroswap puts the constant `"SoroswapPair"` there and the action in **topic[1]**.
So the filter never matched → `extract` returned `Ok(vec![])` → zero trades. The
classify-layer guards (unresolved recording, `is_aquarius_router_swap`) also key
topic[0], which is why the gap was **doubly silent** (0 candles AND 0
`unresolved_pools`). The data map is already the uniswap-v2 shape `decode_swap`
handles — only recognition was broken.

**Fix (this branch):** `soroswap-extractor` recognises the `SoroswapPair`
envelope (action read from topic[1] when topic[0] == `"SoroswapPair"`, else
topic[0] for the older bare-`Symbol` fixtures). Live Soroswap prices correctly
from deploy onward. The **historical fill** of the 824k past swaps becomes a
**ClickHouse-to-ClickHouse derivation** from BE's `soroban_events` (no ledger
re-download) — tracked in **0088**.

The PR-#112 observability guard (record a venue-known-but-`reg.soroswap`-absent
Soroswap pool) is retained as defence-in-depth but is now mostly moot — the real
events are recognised and priced.

## Evidence (prod ch-prod-01, 2026-07-15)

- `price_ohlcv_1m` sources: `sdex` 530M, `aquarius` 2.14M, `phoenix` 156k,
  **`soroswap` 0**.
- `pool_registry FINAL` by venue: `aquarius` 488, `phoenix` 19, **`soroswap`
  221** — so the registry IS seeded (task 0079).
- `unresolved_pools FINAL`: 138 pools / 7,887 swaps, all `source='backfill'`;
  **`unresolved_that_are_soroswap = 0`**.

So Soroswap swaps are neither resolved (0 candles) NOR recorded as unresolved —
they are **invisible to the backfill**, not merely unattributed.

## Root cause (FINAL — an extractor bug)

Diagnosed against BE's full-history `default.soroban_events` (activation →
63.49M). Our 221 pools emit **824k swaps** shaped:

```
topics = [ String("SoroswapPair"), Symbol("swap") ]   ← action in topic[1]
data   = Map{ amount_0_in, amount_0_out, amount_1_in, amount_1_out, to }
```

`SoroswapPairExtractor` recognised the action in **topic[0]** (`== "swap"`), but
Soroswap puts the constant `"SoroswapPair"` there and the action in **topic[1]**.
So the filter never matched → `extract` returned `Ok(vec![])` → zero trades. The
`classify_amm_groups` guards (unresolved recording, `is_aquarius_router_swap`)
also key topic[0], so the gap was **doubly silent — 0 candles AND 0 unresolved**.
The data map was already the uniswap-v2 shape `decode_swap` handles — only
recognition was broken.

**Two earlier hypotheses were disproven** (kept here so the trail is honest):
1. *Missing preload* — false: `sdex-backfill` has preloaded `pool_registry`
   (venue + soroswap tokens) since 0053 (`run.rs:128`).
2. *Seed-timing* (registry seeded 2026-07-14, after the run) — a red herring: a
   re-run would still emit nothing because the events are never recognised.

The swap.jsonl samples the extractor was first validated against
(`Symbol("swap")` + CLMM/simple maps, e.g. `CCR2CH4G…`) are a **different**
population — not in our `pool_registry`.

## Implementation

- **Extractor fix (`soroswap-extractor`):** `swap_action()` reads the action from
  topic[1] when topic[0] == `"SoroswapPair"`, else from topic[0] (older bare-`Symbol`
  fixtures). Used in `extract()` + `decode_swap()`. Trader also resolved from `to`.
- **Observability guard (`prices-ingest-core::classify_amm_groups`, PR #112):** a
  venue-known but `reg.soroswap`-absent Soroswap pool records an `unresolved_pools`
  row instead of vanishing — narrowed (post-review) to the genuine pair-resolution
  miss, not any zero-tick outcome (avoids false positives / unbounded memory).
- **Historical fill → task 0097:** the ~824k past swaps re-priced CH-to-CH from
  BE `soroban_events` (no ledger re-download). NOT the ledger re-run first planned
  for 0088.

## Acceptance Criteria

- [x] Root cause confirmed (final): extractor read the swap action from topic[0],
      but Soroswap events carry `String("SoroswapPair")` in topic[0] and the action
      in topic[1]. NOT missing-preload, NOT seed-timing (both disproven against BE's
      full-history `soroban_events`).
- [x] `soroswap-extractor` recognises the `SoroswapPair` envelope (action from
      topic[1]); the uniswap-v2 data map already decoded. Regression tests: real
      SoroswapPair swap decodes; `sync`/`deposit` (non-swap actions) don't produce
      trades.
- [x] Observability guard retained (PR #112): a venue-known-but-`reg.soroswap`-absent
      Soroswap pool is recorded, not silently dropped (defence-in-depth).
- [x] Deploy the fixed extractor → **live** Soroswap prices. Deployed 2026-07-15
      ~15:57Z (ledger-processor ComputeStack, arm64); verified: `soroswap` now a
      live source in `price_ohlcv_1m`, flowing in lockstep with sdex/aquarius/phoenix
      at the catch-up frontier.
- [ ] **Historical fill** of the ~824k past swaps → non-zero `soroswap` candles for
      the pre-catch-up range. CH-to-CH reprice from BE `soroban_events`; **spawned as
      task 0097** (was mis-scoped to 0088).

## Implementation Notes

- **Merged in PR #112** (squash `2c53ee4`). Files: `soroswap-extractor/src/lib.rs`
  (`swap_action` + `to` trader), `prices-ingest-core/src/soroban.rs` (observability
  guard + tests), doc.
- **Tests (+4 new, all green):** `soroswap-extractor` — real SoroswapPair swap
  decodes; non-swap actions (`sync`/`deposit`) don't. `prices-ingest-core` —
  end-to-end at the classify seam (SoroswapPair swap prices as `soroswap`);
  venue-known-but-unresolvable records; resolvable zero-amount swap is NOT recorded.
  Old bare-`Symbol("swap")` fixtures unchanged. `cargo check --workspace`, clippy,
  fmt clean.
- **Deploy** via `docs/runbooks/deploy-ledger-processor.md` (build arm64 bootstrap
  → `make diff-production` clean [only ComputeStack code asset] → `make
  deploy-production-compute`). Restart safe: durable CH cursor (0064) → resume, no
  rewind. Catch-up tail [~07-10→tip] now carries Soroswap for free.

## Design Decisions

### Emerged

1. **Diagnosed via BE `soroban_events`, not the ledger archive.** The prices DB
   had no Soroswap raw data (0 candles), so the shape came from BE's full-history
   event store — which also proved the historical fill can be CH-to-CH (0097), not
   a multi-day ledger re-download.
2. **Root cause revised twice** (missing-preload → seed-timing → extractor topic
   bug). Kept the disproven hypotheses in the record so future sessions don't
   re-walk them.
3. **`swap_action` accepts both shapes** (topic[1] for `SoroswapPair`, else
   topic[0]) rather than hard-switching — keeps the existing bare-`Symbol`
   fixtures/samples valid and is forgiving of a second emitter shape.
4. **Observability guard narrowed after code review** (PR #112): record only the
   `reg.soroswap`-absent case, not any `priced==0`, to avoid false positives on
   zero-amount swaps and unbounded in-memory `unresolved` growth on long runs.
5. **Historical fill re-homed to a new task 0097** (not 0088): it's a distinct
   tooling effort (events-sourced repricer) once the extractor is correct.

## Notes

- Related coverage memory: [[amm-historical-pool-discovery-gap]],
  [[amm-live-pool-registry-preload-gap]]. Root-cause record:
  [[task-0096-soroswap-root-cause]].
- Live-freeze catch-up ([[proto27-xdr26-live-freeze]], 0064/0094) still draining;
  now emits Soroswap for its tail.

## Future Work

- **Task 0097** — events-sourced AMM backfill (CH-to-CH reprice of the ~824k
  historical Soroswap swaps from BE `soroban_events`).
