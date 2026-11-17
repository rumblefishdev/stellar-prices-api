---
id: "0338"
title: "coverage-sweep-unclassified stays in ALARM: 0334 and 0337 never finished their registry and reprice steps, and three new emitters are unclassified"
type: BUG
status: completed
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
  - date: "2026-11-02"
    status: completed
    who: akot
    note: >
      PR #409 merged. EventBridge deployed, three registry rows written, 0334 and
      0337 repriced and pre-rolled. Alarm OK at 12:35:41 UTC. Residual recorded.
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

### What a reprice can and cannot recover

`events-backfill` has no venue filter, and an enriched row sits at `version + 1`,
above what the reprice writes for the same ledger. So a reprice fills an empty
minute, and is discarded in a minute that already has an enriched row. That
protects the other venues' USD values, and it limits what comes back here.

Of `CASYAKK3…`'s 578 swaps up to 2026-10-19, 302 sit in minutes with no comet row
and come back. Up to 286 sit in minutes that already hold an old-pool row: 282 on
10-07–11 and 4 on 10-19. All of 10-12–18 comes back.

### Timing

- `prices-production-cleanup` is DISABLED ([[0200]]), so the 1m rows survive.
- `mv_reconcile_*` look back 7 days, and the runbook wants them stopped when a
  pre-roll overlaps that window. A range ending 2026-10-27 00:00 UTC stops
  overlapping on 2026-11-03 00:00 UTC.
- The pre-roll's 1w and 1M rows for the buckets it cuts short lose on version, so
  the reconcile MVs have to rebuild them: week of 10-26 and October are still in
  their windows until about 2026-11-08.

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
- **The reprice is plain, with no `DELETE` of comet rows.** Up to 286 swaps of
  2026-10-07–11 and 10-19 stay undercounted, which is recorded here as known.
  Removing that residual needs comet `DELETE`s on `1m` and on the coarse tiers,
  which is not worth the risk for one pool in five days.
- **Two ranges, not one**, minute-aligned, so 10-19–23 is not rewritten for nothing.

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
   4. Invoke the probe in a later clock hour than the last run; expect
      `unclassified = 0` and the alarm in OK.
   5. From 2026-11-03 00:15 UTC and before 2026-11-08: reprice ledgers
      64,809,264–65,025,000 and 65,099,220–65,150,000, dry run first. The second
      range ends past the orphan BLNT minutes.
   6. `preroll-live-gap.sql` with `start_ts = '2026-10-07 00:00:00'`,
      `end_ts = '2026-10-27 00:00:00'`. Never `preroll-amm-reprice.sql`.
   7. Check the repriced 1m rows got `close_usd`; if not, `coarse-repair
      --table price_ohlcv_1m --start-month 202610 --end-month 202610 --skip-snapshot`.
4. Close [[0334]], [[0337]] and this task.

## Acceptance Criteria

- [x] `CAF5RWQG…`, `CC5QR5OO…` and `CAZE5GVM…` allow-listed with reason and task
- [x] The reason on the three older `4d6abb0d…` entries is corrected
- [x] EventBridge deployed; the running probe carries the change
- [x] `prices.pool_registry FINAL` has `comet` rows for the three 0337 pools
- [x] `source = 'comet'` candles cover 2026-10-12–18 and 10-25 23:35–10-26 01:40 UTC,
      on `1m` and on `1h`; the residual of 10-07–11 is recorded with its measured size
- [x] BLNT's 10 comet trades of 10-26 14:22–16:36 UTC exist under the classic id,
      and the orphan id is on [[0242]]'s residual list
- [x] Manual probe run reads `unclassified = 0`; alarm back to OK

## Result (2026-11-02)

- PR #409 merged as `380aed31`. EventBridge deployed; probe `LastModified`
  12:17:53 UTC, and the deployed `bootstrap` holds the three contracts. `cdk diff`
  for that stack changed only `CoverageSweepProbeFunction` `.S3Key`.
- `--discover-pools` WRITE at 12:33:22 UTC: `written=3`, then `to_write=0`.
  `comet` holds 5 rows. The binary was `3e0e521a…`, built with `cargo lambda
  build --target x86_64-unknown-linux-musl`, since the workstation has no musl-gcc.
- Manual invoke, window 65,046,238–65,267,416: `rows=22`, `allowlisted=22`,
  `unclassified=0`, `unresolved=0`. Alarm OK at 12:35:41 UTC.
- Reprice of 65,099,220–65,150,000 (18,717 rows), then 64,809,264–65,025,000
  (86,791 rows). Other venues kept their rows, trades and `close_usd` zero counts,
  which confirms that an enriched row outranks the reprice.
- Comet gained 512 trades on 1m: 481 in the first range and 31 in the second.
  10-12–18 matches BE's swap count per day.
- **Residual: 93 swaps of `CASYAKK3…` on 10-07–11**, not the 286 estimated above:
  a repriced row with a higher version did replace an old-pool row. The minutes
  affected remain undercounted, as decided.
- 334 comet rows at `close_usd = 0` were enriched in about 90 s, before the pre-roll.
- `preroll-live-gap.sql` over 10-07 to 10-27 ran on 2026-11-02, **without `SYSTEM
  STOP VIEW` and before the 11-03 start above**, which departs from runbook 0142.
  Its reason, a half-refilled `1m`, did not apply: no partition was dropped. No
  rollup or `mv-refresh` alarm fired.
- After it, comet agrees on 1m, 15m, 1h, 4h and 1d: 1,726 trades, $23,364.42. The
  reconcile MVs finished 1w and 1M: each week equals its days, October is 2,630
  trades, $40,006.36.
- The pre-roll put 72 `sdex` rows at `close_usd = 0` (46, 16, 7, 3 on 15m, 1h, 4h,
  1d). The 13:30 UTC coarse sweep restored every count to its value before.
- BLNT's 10 trades now exist under the classic id. The orphan id's rows stay for
  [[0242]]'s cleanup. #390 was merged at 13:28 UTC and is not deployed.
