---
id: "0081"
title: "Full-pipeline AMM reconcile E2E with a real Galexie fixture (swap → price_ohlcv_1m)"
type: TEST
status: backlog
related_adr: ["0007"]
related_tasks: ["0078", "0053"]
tags: [layer-indexing, priority-low, effort-small, rust, test, amm, soroban, pool-registry]
links:
  - "../../../packages/prices-ledger-processor/tests/reconcile_e2e.rs"
  - "../../../packages/prices-ingest-core/src/soroban.rs"
history:
  - date: 2026-07-06
    status: backlog
    who: okarcz
    note: >
      Spawned from 0078 future work. 0078 proved the seeded-vs-unseeded
      resolvability claim at the `classify_amm_groups` seam (unit test), because
      no committed ledger fixture carries AMM activity — the bundled Galexie
      objects (62460540–42) decode to zero AMM ticks/unresolved. This task adds
      the missing *full-pipeline* proof once a real AMM-bearing fixture exists.
---

# Full-pipeline AMM reconcile E2E with a real Galexie fixture

## Summary

Drive the complete live pipeline (`decode_object` → `process_ledger` →
`CandleAccumulator` → sink) over a **real** Galexie ledger that contains a known
AMM swap, with `pool_registry` seeded for that pool, and assert a
`price_ohlcv_1m` candle is produced (not an `unresolved_pools` row). This is the
end-to-end version of the classify-seam unit test 0078 already landed.

## Context

- 0078 wired the live processor's cold-start `pool_registry` preload and proved
  the core claim at the `classify_amm_groups` seam (seeded → `amm_ticks`, empty →
  `unresolved`). See 0078 Emerged #2 for why the seam, not a full E2E.
- The gap is purely a **fixture** one: the committed Galexie ledgers have no AMM
  swaps, and there is no synthetic `LedgerCloseMeta` builder in-repo.

## Implementation

- Identify a mainnet ledger with a Soroswap/Phoenix/Aquarius swap for a pool
  whose contract address + tokens are known (the 0053 discovery artifact or the
  seeded `pool_registry` is a good source of candidate pools/ledgers).
- Fetch that ledger's Galexie `*.xdr.zst` object read-only
  (`--no-sign-request`) into `prices-ledger-processor/fixtures/ledgers/` (keep
  the self-skipping-when-absent convention; the object is gitignored/committed
  per repo policy for fixtures).
- Add a reconcile test: seed `Registries` with the pool (mirror
  `pool_registry_preload_it`'s fixture), run the `Reconciler` over the fetched
  ledger with a real (non-counting) or asserting sink, and check the candle for
  that pair is emitted with a plausible price/volume and nothing lands in
  `unresolved`.

## Acceptance Criteria

- [ ] A real AMM-bearing ledger fixture is present (or self-skips when absent).
- [ ] Full-pipeline reconcile over it, with the pool seeded, emits the expected
      `price_ohlcv_1m` candle and zero `unresolved_pools` rows for that pool.
