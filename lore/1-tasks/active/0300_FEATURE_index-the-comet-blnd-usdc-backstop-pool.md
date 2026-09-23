---
id: "0300"
title: "Index the Comet weighted pool (Blend backstop BLND/USDC, CAS3FL6T…) — 53k swaps since 2024-05 that never reached a candle"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0100", "0290", "0285", "0286"]
tags: [layer-indexing, priority-medium, effort-medium, amm, ingestion, data-correctness]
links:
  - "../active/0100_FEATURE_recurring-coverage-sweep-for-unindexed-venues.md"
  - "../../../packages/coverage-sweep-probe/allowlist.toml"
history:
  - date: "2026-09-22"
    status: backlog
    who: akot
    note: >
      Spawned from 0100's phase-1 classification. The coverage sweep's
      largest unclassified emitter is a Comet (Balancer-style weighted) pool
      trading BLND against USDC — the Blend protocol's backstop LP — with
      53,092 swaps from ledger 51,500,460 (2024-05-02) to 64,554,336 and no
      extractor of ours recognising its `[Symbol("POOL"), Symbol("swap")]`
      event. It sits on the sweep's allow-list as a temporary [[wasm]] entry
      `until = "0300"` until this task indexes it.
  - date: "2026-09-23"
    status: active
    who: akot
    note: >
      Activated. Pattern to follow is [[0290]] (PR #324). History is expected
      to fold into [[0286]] phase 3 like SushiSwap's did, which makes the
      order extractor deploy → `CAS3FL6T…` in `pool_registry` → phase 3.
  - date: "2026-09-23"
    status: active
    who: akot
    note: >
      Code done on `feat/0300_index-the-comet-blnd-usdc-backstop-pool`
      (7 commits, `646a00a..cdbc182`): venue `comet`, a `comet-extractor`
      crate, self-swaps dropped for every AMM venue, Comet in both loss
      sensors, a committed `STATIC_POOLS` list for factory-less pools, `comet`
      in the pre-roll SQL and a Comet gate in `reingest_0286.py`. Workspace
      1173 passed / 0 failed; verifier 10/10; review 0 blockers (WR-01, WR-02,
      IN-03, IN-05 fixed). Local end-to-end on real prod events (read-only):
      candles == real swaps in every window, discover writes 1 row then 0,
      live path prices a real ledger from an empty registry. AC 1 met; AC 2–3
      wait for the deploy and 0286 phase 3, AC 4 for PR #332. Stays active.
---

# Index the Comet weighted pool (Blend backstop BLND/USDC)

## Summary

Task [[0100]]'s coverage sweep found a real AMM venue we do not index: the
Comet pool `CAS3FL6TLZKDGGSISDBWGGPXT3NRR4DYTZD7YOD3HMYO6LTJUVGRVEAM`, LP token
"Comet Pool Token" (`CPAL`, 7 decimals), trading **BLND (asset_id 225) against
USDC (asset_id 3)**. It is Blend's backstop pool. Every swap in it since
2024-05-02 is missing from the candles — the same class of gap as SushiSwap V3
([[0290]]), smaller in count, but it is the main on-chain market for BLND.

## Findings (2026-09-22, production, `dev_read`)

- **Code:** wasm `8abc28913035c07411ed5d134e6bfeab4723d97ddd4d1a22a0605d35c94d1a36`.
  Interface (`default.wasm_interface_metadata`): `init(controller, tokens,
  weights, balances, swap_fee)`, `gulp`, `join_pool`, `exit_pool`,
  `swap_exact_amount_in`, `swap_exact_amount_out`, `get_spot_price`,
  `get_normalized_weight`, plus the SEP-41 token functions of its LP token.
  Balancer-v1-style weighted pool (Comet).
- **Contracts on that wasm:** two. `CAS3FL6T…` (deployed ledger 51,499,545)
  and `CB3A6LUP…` (51,498,988), the latter with **no swaps**.
- **Swap event:** topics `[Symbol("POOL"), Symbol("swap")]`, data map
  `{caller, token_amount_in, token_amount_out, token_in, token_out}` — e.g.
  `token_in = CD25MNVT…` (BLND SAC), `token_out = CCW67TSZ…` (USDC SAC),
  `10428427249 → 50915486`. Tokens and both amounts are in the event, so a
  price needs no pool state.
- **Other events:** `POOL/join_pool` (396), `POOL/exit_pool` (394) and
  `POOL/deposit` (2) in the last 14 days — liquidity, not trades.
- **Volume:** 53,092 swaps lifetime (ledgers 51,500,460 → 64,554,336);
  727 in the 14 days to 64,543,788; 125 of its 412 transactions in that window
  also hold a registered pool's swap (arbitrage / aggregator routes), so it is
  a venue in its own right, not a wrapper.

## Implementation

- An extractor for the Comet `POOL/swap` event (venue name to decide, e.g.
  `comet`), on the same shape as the existing extractors; fills are
  ratio-priced and take ADR 0287's price-forming bound.
- Registration: no factory event to learn from is known yet — check whether
  Comet pools are created by a factory (Blend's backstop pool was deployed
  directly); otherwise seed `pool_registry` for `CAS3FL6T…` explicitly and
  make the sweep catch the next one.
- Live path (`prices-ingest-core/src/soroban.rs`) and history
  (`events-backfill` over ledgers 51,500,460 → now), under 0286's definitions.
- Remove the temporary `[[wasm]] 8abc2891… until = "0300"` entry from
  `packages/coverage-sweep-probe/allowlist.toml` once the pool is in
  `pool_registry`.

## Acceptance Criteria

- [x] `POOL/swap` decoded into fills with the right orientation and decimals
      (unit tests on real event samples).
      → 13 real prod payloads (both directions, largest, dust, a 5-swap tx) in
      `comet-extractor` and `prices-ingest-core/src/soroban/comet_tests.rs`;
      USDC is the quote in both directions, price = USDC per BLND.
- [ ] `CAS3FL6T…` in `prices.pool_registry`; live ingestion writes BLND/USDC
      candles from it. **Code ready, waits for the deploy.** Proven locally:
      `prices-cli` on real ledger 64,573,919 with an EMPTY registry wrote the
      comet candle (0.00549329 USDC/BLND) and left the registry empty;
      `--discover-pools` wrote the row once, then 0.
- [ ] History from ledger 51,500,460 re-priced; swap count reconciled against
      the raw event count — against the **real** swaps (raw `POOL/swap` minus
      self-swaps, D1): 53,096 − 1,459 = 51,637 on 2026-09-23. **Waits for
      [[0286]] phase 3.** Proven locally per window (below).
- [ ] The allow-list's `until = "0300"` entry removed; the coverage sweep
      stays at zero unclassified for this pool. **Waits for PR #332** (the
      probe crate is not on `develop`); once the row is in `pool_registry` the
      sweep skips the pool anyway, so this is clean-up.

## Implementation Notes (2026-09-23)

Branch commits: `646a00a` venue + decoder + `comet` in all 97 pre-roll
`source IN (…)` sites · `d7041f5` self-swap guard (every venue) · `6588391`
loss sensors · `f6b2299` `STATIC_POOLS` · `32e2489` phase-3 gate · `ac996b3`
gate proves the binary, months after 202609 only reported · `cdbc182`
non-positive amounts are errors, venue-list comments.

- **Registration without a factory:** `prices-ingest-core/src/static_pools.rs`
  holds `STATIC_POOLS` (only `CAS3FL6T…` → `comet`), merged at four seams:
  live `Reconciler::new` before the persisted snapshot (routes from cold
  start, never writes the row); events-backfill reprice after the
  `EmptyPoolRegistry` guard; `--discover-pools` after its snapshot (writes the
  row once); sdex-backfill after its load (its end-of-run write re-writes the
  same row, idempotent under ReplacingMergeTree).
- **Phase 3 needs no code change to pick Comet up:** events-backfill reads
  every registered contract, and `preroll.sql` / `preroll-live-gap.sql` do not
  filter by venue. `reingest_0286.py` gains a Comet gate: registry row +
  a one-ledger dry run at 51,500,460 proving the deployed binary routes comet
  (`--ack-0300-binary` for `--amm wait`); a Comet month ≤ 202609 with 0 trades
  is a DEFECT, later months only a finding.
- **Local end-to-end (real prod events, read-only, local CH 26.3.10.60):**

  | Window | Raw swaps | Self | Real | Comet `trade_count` |
  |---|---|---|---|---|
  | 2025-11-21 | 114 | 0 | 114 | 114 |
  | 2026-08-25 | 2,963 | 642 | 2,321 | 2,321 |
  | 2026-09-22 → 09-23 | 7 | 0 | 7 | 7 |
  | dust samples (2024) | 29 | 0 | 29 | 29 (dust not price-forming) |

  Per-minute trade count, pf count and volumes: 0 mismatches. No BLND/BLND or
  USDC/USDC rows; 0 dispatch errors, 0 unresolved.
- **Deploy order:** ledger-processor → `events-backfill --discover-pools`
  (dry run, then write: `to_write=1 {"comet": 1}`) → [[0286]] phase 3.

## Design Decisions

### From Plan (Adam, 2026-09-23)

1. **D1 — self-swaps dropped for every venue**, in `amm_trade_to_tick` before
   `canonicalise`; not a dispatch error.
2. **D2 — the 2026-08-25 incident is indexed as-is**, re-affirmed after
   reading Script3's post-mortem (see Issues). Rejected: excluding the
   self-swap transactions; cutting the pool off at ledger 64,112,340.
3. **D3 — a committed static list** instead of a manual INSERT.
4. **D4 — history folds into [[0286]] phase 3** (the [[0290]] decision).
5. **D5 — venue `comet`.**

### Emerged

6. **`prices-ingest-core` depends on `comet-extractor`** so dispatch and both
   sensors share one `is_pool_swap` predicate.
7. **The static merge is NOT inside `OhlcvWriter::load_pool_registry`** — that
   would silence the `EmptyPoolRegistry` guard, stop discover from ever
   writing the row and break asset-discovery's `pools_total`.
8. **A zero amount is an error too** (IN-03): `json_int` turns an unparseable
   string into 0.

## Issues Encountered

- **The pool was exploited on 2026-08-25** (Script3 post-mortem, 2026-08-28):
  the missing `token_in != token_out` check let a self-swap corrupt the pool's
  accounting; 717,518 USDC drained; the pool is "unsafe and frozen" and Blend
  V2 winds down. Swaps still arrive (2 on 2026-09-23), ~0.005 USDC/BLND.
  Consequence of D2 in BLND's candles: 2026-08-25 carries ~877 M BLND / $8.1 M
  of exploit-cycle volume (≈100× the day's other BLND volume, day VWAP ≈4×
  market); on 2026-09-06/07 real sells print 4.47 / 2.63 / 0.86 / 0.23 USDC
  per BLND (ledgers 64,304,919–64,319,480) against a ~0.004 market.
- **Comet has no website**; its only presence is github.com/cometdex.
- A newer Comet wasm `61e1fc54…` (135 contracts, 20 swaps lifetime, topic
  `[swap_event, POOL, swap]`) is not indexed — recorded only.
- `cargo clippy --workspace` is red on findings already on `develop`
  (`canonical.rs`, older `soroban.rs` lines); none added here.

## Notes

- Deploys only after [[0286]] phase 1 (Compute freeze), like [[0290]].
- BLND's other markets: check SDEX/Aquarius coverage of BLND before sizing the
  impact on its USD price.
