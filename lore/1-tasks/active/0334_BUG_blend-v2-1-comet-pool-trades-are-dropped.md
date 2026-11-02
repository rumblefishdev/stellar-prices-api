---
id: "0334"
title: "Blend v2.1's Comet BLND/USDC pool is missing from pool_registry — every swap since 2026-10-07 is dropped and both pool alarms fire"
type: BUG
status: active
assignee: akot
related_adr: []
related_tasks: ["0300", "0291", "0100", "0323", "0329"]
tags: [layer-backend, priority-high, effort-small, ingest, soroban, comet, observability]
links:
  - "../../../packages/prices-ingest-core/src/static_pools.rs"
  - "../../../packages/coverage-sweep-probe/allowlist.toml"
  - "../../../docs/runbooks/seed-pool-registry.md"
  - "https://github.com/blend-capital/docs-v2/blob/main/mainnet-deployments.md"
  - "https://github.com/CometDEX/comet-contracts-v1/releases/tag/v1.1.0_contracts_pkg1.0.0_cli25.1.0"
history:
  - date: "2026-10-16"
    status: active
    who: akot
    note: "Task created from the unregistered-pool and coverage-sweep alarms; pool verified as Blend v2.1's"
---

# Blend v2.1's Comet BLND/USDC pool is missing from pool_registry

## Summary

Blend v2.1 moved its backstop to a new Comet BLND/USDC pool,
`CASYAKK3SHDN6S2E6IUOBVRXT3KKPVQCVQO4GNBDTVE6XZQQMW4UMIHE`. Comet pools have no
factory event, so only `STATIC_POOLS` registers them, and it holds only the old
pool `CAS3FL6T…` ([[0300]]). The live processor drops every swap of the new pool.
It is in `prices-production-ledger-processor-unregistered-pool` (flapping since
2026-10-07, ~12 transitions a day) and makes 393 of the 404 events behind
`prices-production-coverage-sweep-unclassified` (ALARM since 2026-10-12).

## Verification (2026-10-16)

- **Official.** `blend-capital/docs-v2` `mainnet-deployments.md` (commit
  ef36127c, 2026-10-07) lists it as v2.1's "Comet v1.1 Contract (BLND:USDC
  liquidity pool)". The v2.1 backstop `CCS4AZ5O…` is listed there too and in
  blend-ui's `.env.production` as `NEXT_PUBLIC_BACKSTOP_V2_1`. It holds ~50% of
  the pool's LP supply.
- **Code.** Wasm `d735c339…` is byte-identical (sha256) to CometDEX release
  `v1.1.0_contracts_pkg1.0.0_cli25.1.0` (2026-09-11). That release adds
  `token_in != token_out` (`ErrSameToken`) to both swap entry points, which closes
  the 2026-08-25 self-swap exploit.
- **Payload.** Same `POOL/swap` topic and data map as the old pool (`caller`,
  `token_amount_in/out`, `token_in/out`). All 520 lifetime swaps are BLND SAC
  `CD25MNVT…` ↔ USDC SAC `CCW67TSZ…`. 730 events in prod ClickHouse match
  StellarExpert's count.
- **Price.** Hourly VWAP vs the same hour elsewhere: Soroswap median −0.09%
  (p5/p95 −1.8%/+0.8%), SDEX median −0.59% (−3.3%/+1.5%). The only outliers
  (≤ −34%) are the launch hours of 2026-10-07, when SDEX and Soroswap disagreed
  with each other too.
- **Range.** Pool created 2026-09-30, first swap 2026-10-07 00:16 UTC (ledger
  64,809,271), the same minute as the first WARN. Nothing was traded before.
  ~1.55 M BLND / ~9.5 k USDC to 2026-10-16. 80% of swaps come from one arb-like
  contract (`CBNVK5PE…`).

The sweep's two other emitters are not priceable:

- `CDU47AIU…` (wasm `f3dd3edf…`, 10 events): securities-financing ledger (repo,
  stock loan, FX forward; PSG Digital Labs). `trade_opened/closed` carry notional
  and `rate_bps`, no token price. Allow-list it.
- `CD35HSTR…` (1 event): an instance of the allow-listed order-book wasm
  `4d6abb0d…`, matched only by contract id today. Allow-list this instance.

## Implementation Plan

1. Add `CASYAKK3…` (venue `comet`) to `STATIC_POOLS`, with a test on a real swap
   of this pool.
2. Allow-list `CDU47AIU…` and `CD35HSTR…` in `coverage-sweep-probe`.
3. Deploy the ledger-processor; its cold start loads the pool.
4. `events-backfill --discover-pools` WRITE (runbook "Factory-less pools").
5. Reprice from ledger 64,809,271 to the deploy.

## Acceptance Criteria

- [ ] `STATIC_POOLS` holds `CASYAKK3…`; a test decodes one of its real swaps
- [ ] Both sweep emitters allow-listed
- [ ] Ledger-processor deployed; no `unregistered_pool_events` WARN after it
- [ ] `prices.pool_registry FINAL` has a `comet` row for `CASYAKK3…`
- [ ] Minutes from 2026-10-07 00:16 UTC repriced; BLND `comet` 1h candles
      exist for those hours
- [ ] Next coverage sweep clean; `coverage-sweep-unclassified` back to OK

## Notes

- The old pool `CAS3FL6T…`, called frozen after the exploit, still emitted 1,084
  swaps from 2026-09-30 to ~2026-10-11 and is indexed. Its prices were not
  checked here.
