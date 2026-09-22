---
id: "0300"
title: "Index the Comet weighted pool (Blend backstop BLND/USDC, CAS3FL6T…) — 53k swaps since 2024-05 that never reached a candle"
type: FEATURE
status: backlog
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

- [ ] `POOL/swap` decoded into fills with the right orientation and decimals
      (unit tests on real event samples).
- [ ] `CAS3FL6T…` in `prices.pool_registry`; live ingestion writes BLND/USDC
      candles from it.
- [ ] History from ledger 51,500,460 re-priced; swap count reconciled against
      the raw event count.
- [ ] The allow-list's `until = "0300"` entry removed; the coverage sweep
      stays at zero unclassified for this pool.

## Notes

- Deploys only after [[0286]] phase 1 (Compute freeze), like [[0290]].
- BLND's other markets: check SDEX/Aquarius coverage of BLND before sizing the
  impact on its USD price.
