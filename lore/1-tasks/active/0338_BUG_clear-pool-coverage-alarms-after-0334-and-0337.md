---
id: "0338"
title: "coverage-sweep-unclassified stays in ALARM: 0334 and 0337 never finished their registry and reprice steps, and three new emitters are unclassified"
type: BUG
status: active
assignee: akot
related_adr: []
related_tasks: ["0334", "0337", "0336", "0242", "0100", "0200"]
tags: [layer-backend, ingest, soroban, comet, observability, coverage, priority-high, effort-small]
links:
  - "../../../packages/coverage-sweep-probe/allowlist.toml"
  - "../../../packages/prices-ingest-core/src/static_pools.rs"
  - "../../../docs/runbooks/0100-coverage-sweep-triage.md"
  - "../../../docs/runbooks/seed-pool-registry.md"
  - "../../../docs/runbooks/events-sourced-amm-reprice.md"
history:
  - date: "2026-11-02"
    status: active
    who: akot
    note: "Task created from the coverage-sweep-unclassified alarm of the 2026-11-02 05:17 UTC run; one task for all of it"
---

# Clear the pool-coverage alarms after 0334 and 0337

## Summary

`prices-production-coverage-sweep-unclassified` has been in ALARM since
2026-10-26 05:18 UTC. The 2026-11-02 run completed with 6 contracts and 594
events. Three contracts are the Comet pools of [[0337]]: merged and deployed,
but without a `prices.pool_registry` row, which is the only thing the sweep
reads (`sweep.rs:156`). The other three are new emitters.

The trades are no longer being dropped: the last `unregistered_pool_events`
WARN is 2026-10-26 01:40 UTC, and comet candles exist from the deploys on. What
is missing is the history. Neither [[0334]] nor [[0337]] has been repriced.

## Findings (2026-11-02)

### The sweep

Window 65,040,990–65,262,168. `with_pool` is over all txs of the contract in
the window.

| Contract | What | Wasm | Events | `with_pool` | Outcome |
| --- | --- | --- | --- | --- | --- |
| `CDQMIKIHKA5UY3FQJ2UDTLNX3GOPD3GPNZWSPTOBVOHEUMQJQXOPC7Y3` | Comet v1.1, BLNT/USDC | `d735c339…` | 299 | | Registry row |
| `CA5HX3D47PQZWC2E7VWXCKH4534P2NOJEOIXAWU47MI4QV2FQUYY3FLC` | Comet v1.1, BLNT/XLM | `d735c339…` | 281 | | Registry row |
| `CCNDNFA7LEVUJRYWTSWVAA2ESZAKFO6MYOPI6E5AZT3V2UBP2MNV7QFZ` | Comet v1.1, BLND/USDC | `d735c339…` | 6 | | Registry row |
| `CAF5RWQG5GSTEPD5XAKCUH2H77NWA4J6DNFT6TUP2NU6Q6VW3R6HCZYO` | Permissioned order book, grBENJI/USDC | `4d6abb0d…` | 4 | 0/50 | `[[contract]]` |
| `CC5QR5OOQMCPVSL3SJMH7QESCFINWUMX3427CT4NIDBS3FRVDHAM4NAJ` | Tokenised fund aLQD (Alloy Liquid Dollar) | `a85d6d53…` | 1 | 3/8 | `[[contract]]` |
| `CAZE5GVMH5332L6FEGVPPDLEPVYUFGQB7ZOCH2OKKBJCBYZQ5FGKZN7L` | Stableswap "Spreadless USD 3pool" | `ae835560…` | 3 | 0/7 | `[[contract]]` |

- **Registry.** `prices.pool_registry FINAL` holds 2 `comet` rows, `CAS3FL6T…`
  and `CASYAKK3…`. The three pools above are absent.
- **`CC5QR5OO…`.** Unverified source; interface read from the wasm: `issue`,
  `redeem`, `nav`, `rebalance`, `trade`, `set_router`. Created 2026-10-26. Its one
  `trade` wraps `SoroswapRouter/swap` (allow-listed `CAG5LRYQ…`) and
  `SoroswapPair/swap` on registered `CAM7DY53…`. Indexing it would double-count.
- **`CAZE5GVM…`.** Unverified source. An AMM: `get_amp` = 100, `get_reserves`,
  `swap_exact_in/out`, `deposit`, `withdraw`, own LP token `SPR-USD3`. USDT0, PYUSD
  and USDC, about $29 of reserves, created 2026-10-29 by `GBAXG35E…`. Its three
  swaps are 1 unit each. No extractor exists and it has `upgrade()`.
- **`CAF5RWQG…`.** The fourth instance of order-book wasm `4d6abb0d…`. Its
  `get_config` returns `base_token` grBENJI (`CBEFW4IK…`) and `quote_token` USDC,
  and the book has `whitelist`, `access_control` and admin-set phases and
  reference price. Decoded trades are ~75,000 at 0.9999999–1.0000000.
- **The existing reason is wrong.** The three `4d6abb0d…` entries say
  "instruments are u64 ids with no token addresses". The u64s are order ids. The
  trade event carries no token address, but the book's config does. This also
  rules out the `not_a_venue` that [[0336]]'s notes suggest: the code is a market.

### The history that is missing

- **[[0334]] was not repriced.** `CASYAKK3…` emitted 174 swaps on 2026-10-12–18
  and `source = 'comet'` has 1 trade on those days. Its 1m candles start at
  2026-10-19 11:02 UTC. BE holds 560 swaps for 2026-10-07–18. Comet candles of
  10-07–11 exist but come from the old pool `CAS3FL6T…`.
- **[[0337]] was not repriced.** The first BLNT candle is 2026-10-26 14:22 UTC;
  the 18 BLNT swaps of 10-25 23:35–10-26 01:40 UTC have none. `CCNDNFA7…`'s swaps
  from 10-23 18:58 UTC (ledger 65,099,227) are not distinguishable in BLND/USDC
  candles and are assumed missing.
- **BLNT has two identities.** The first 10 comet trades (10-26 14:22–16:36 UTC)
  sit on `asset_id` 14236029787030528679, `soroban`, contract `CCBNWP5L…`, created
  at 14:22:57. From 18:42 UTC, when `sdex` first traded it, the trades sit on
  15676158627979534220, `classic` BLNT-`GAP2ACGH…`, `sac_address` `CCBNWP5L…`.
  This is the race in [[0242]]: the SAC traded before anything interned its
  classic. PR #390 is still open, so live can do this again for any new SAC.
- **A reprice heals the trades, not the orphan id.** `events-backfill` builds its
  registry from `prices.assets` (`run.rs:220`), and the classic row now carries
  the SAC, so it writes these minutes under the classic id. The rows under
  14236029787030528679 and its `prices.assets` row stay.
- **The order books.** `CA5YJ5H2…` is grBENJI/USDC like `CAF5RWQG…`; `CBOSXSEZ…`
  and `CD35HSTR…` are AMA1/XUSDT.

### What does not constrain this

- `prices-production-cleanup` is DISABLED ([[0200]]), so the 1m rows survive.
- `mv_reconcile_*` look back 7 days and BE holds the events, so there is no hard
  deadline, but every minute repriced here is outside that window.

## Decision

- All of it lives in this task, with no separate venue task.
- **`CAZE5GVM…` is a `[[contract]]`, not a temporary `[[wasm]]`.** A `[[wasm]]`
  needs `until` = a task that removes it, and none exists. The contract entry
  covers this pool only, so the next pool of the family still pages and reopens
  the question. It says plainly that this is a small unindexed AMM.
- `CAF5RWQG…` is a `[[contract]]`, and the reason on the three older order-book
  entries is corrected in the same commit. It is no `not_a_venue`.
- `CC5QR5OO…` is a permanent `[[contract]]`.
- Removing the stale `until` entries (`003710b3…`, `95a8e001…`, `8abc2891…`) is
  out of scope.
- **The BLNT orphan id goes to [[0242]].** The reprice ends past 16:36 UTC so the
  10 trades reach the classic id. This task deletes nothing on production: after
  the reprice the orphan rows are copies, and they join 0242's residual cleanup,
  which is where every other SAC-keyed row is removed.

## Implementation Plan

1. Allow-list the three contracts and correct the order-book reason; fix what
   the count-pinning tests report; `cargo test -p coverage-sweep-probe`.
2. Add BLNT's orphan id to [[0242]]'s residual list (done in its Heal log).
3. After merge, in this order:
   1. `make -C infra diff-production`, read in full; then
      `make -C infra deploy-production-eventbridge`.
   2. A **new** `events-backfill` from develop. An older binary does not hold the
      pools of 0337 and reports a false `to_write=0`.
   3. `--discover-pools --dry-run` (expect `to_write=3`, `change="new"`), WRITE,
      dry run again (`to_write=0`).
   4. Reprice ledgers 64,809,271–65,150,000 (10-07 to 10-26 ~16:50 UTC), dry run
      first. The end covers the orphan BLNT minutes.
   5. `preroll-live-gap.sql` over that range. Never `preroll-amm-reprice.sql`.
   6. Invoke the probe in a later clock hour than the last run; expect
      `unclassified = 0` and the alarm in OK.
4. Close [[0334]], [[0337]] and this task.

## Acceptance Criteria

- [ ] `CAF5RWQG…`, `CC5QR5OO…` and `CAZE5GVM…` allow-listed with reason and task
- [ ] The reason on the three older `4d6abb0d…` entries is corrected
- [ ] EventBridge deployed; the running probe carries the change
- [ ] `prices.pool_registry FINAL` has `comet` rows for the three 0337 pools
- [ ] `source = 'comet'` candles cover 2026-10-12–18 and 10-25 23:35–10-26 01:40 UTC
- [ ] BLNT's 10 comet trades of 10-26 14:22–16:36 UTC exist under the classic id,
      and the orphan id is on [[0242]]'s residual list
- [ ] Manual probe run reads `unclassified = 0`; alarm back to OK
