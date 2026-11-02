---
id: "0337"
title: "Three new Comet v1.1 pools (BLNT/USDC, BLNT/XLM, a small BLND/USDC) are missing from pool_registry and the BLNT backfill contract pages the sweep"
type: BUG
status: active
assignee: okarcz
related_adr: []
related_tasks: ["0300", "0334", "0336", "0100", "0291"]
tags: [layer-backend, ingest, soroban, comet, observability, coverage, priority-high, effort-small]
links:
  - "../../../packages/prices-ingest-core/src/static_pools.rs"
  - "../../../packages/coverage-sweep-probe/allowlist.toml"
  - "../../../docs/runbooks/0100-coverage-sweep-triage.md"
  - "../../../docs/runbooks/seed-pool-registry.md"
  - "https://github.com/BLNT-Protocol/blnt-backfill-contract/tree/fe366da68da7f24ef20063c43aaf0b13ea752257"
history:
  - date: "2026-10-26"
    status: active
    who: okarcz
    note: "Task created from the coverage-sweep-unclassified alarm of the 2026-10-26 05:17 UTC run"
---

# New Comet pools and the BLNT backfill contract page the sweep

## Summary

`prices-production-coverage-sweep-unclassified` went to ALARM at
2026-10-26 05:18 UTC. The Monday run found 4 contracts with 25 swap-shaped
events. The previous run (2026-10-23 10:54, after [[0336]]) found 0.

Three of the four are Comet pools on wasm `d735c339…`, the CometDEX v1.1
code [[0334]] registered for Blend's `CASYAKK3…`. Comet has no factory, so
only `STATIC_POOLS` registers a pool. The live processor drops their swaps:
`ledger-processor-unregistered-pool` flapped 00:35–01:56 UTC on 10-26, in
step with the BLNT swaps. The fourth contract is not a market.

## Findings (StellarExpert public API, 2026-10-26)

| Contract | What | Created | Swaps in window |
| --- | --- | --- | --- |
| `CDQMIKIHKA5UY3FQJ2UDTLNX3GOPD3GPNZWSPTOBVOHEUMQJQXOPC7Y3` | Comet v1.1, BLNT/USDC | 2026-10-25 19:18 by `GDEA2WMO…` | 9 |
| `CA5HX3D47PQZWC2E7VWXCKH4534P2NOJEOIXAWU47MI4QV2FQUYY3FLC` | Comet v1.1, BLNT/XLM | 2026-10-25 19:19, same creator | 9 |
| `CCNDNFA7LEVUJRYWTSWVAA2ESZAKFO6MYOPI6E5AZT3V2UBP2MNV7QFZ` | Comet v1.1, BLND/USDC | 2026-09-23 by `GCRKRXU2…` | 4 |
| `CANIE64BDFKLHROLOIY2KTQVBWUZNUKZP75DLXC272QM7TIN5N3M4LVA` | BLNT backfill contract (wasm `dcdc0b18…`) | 2026-10-25 19:30 by `GDEA2WMO…` | 3 (`swap_blnd`) |

- **Tokens.** BLNT SAC `CCBNWP5L…` (classic `BLNT-GAP2ACGH…`, created
  2026-10-25), USDC SAC `CCW67TSZ…`, XLM SAC `CAS3J7GY…`, BLND SAC
  `CD25MNVT…`. Payload is the same `POOL/swap` map as [[0334]].
- **Prices are coherent.** ~0.0048 USDC/BLNT and ~0.036 XLM/BLNT (implies
  XLM ≈ $0.135); the BLND pool ~0.0054 USDC/BLND, near the Blend pool.
- **BLNT swaps come from one contract** `CB2DT56G…` routing USDC → BLNT →
  XLM, so each tx swaps in both BLNT pools. The BLND pool's 4 swaps
  (~$0.10 in total) come from `CDYYYMHP…`.
- **`CANIE64B…` is not a market.** Verified source
  (`BLNT-Protocol/blnt-backfill-contract`): it custodies BLNT v3's initial
  allocation; `swap_blnd` is a fixed 1:1 BLND → BLNT conversion (burns
  BLND, pays pre-funded BLNT). No admin, no upgrade. Pricing it would pin
  BLNT to BLND.

## Decision

- Register all three pools as `comet`, the [[0334]] pattern. The BLND pool
  barely trades, but the rule stays one rule: a Comet pool is registered.
- Allow-list `CANIE64B…` as a permanent `[[contract]]`.
- No wasm-hash auto-registration for now: three pools in one week come from
  one launch. Revisit if Comet pools keep appearing.

## Implementation Plan

1. Add the three pools to `STATIC_POOLS` with a decode test on a real swap
   of each.
2. Allow-list `CANIE64B…` in `coverage-sweep-probe`.
3. Deploy Compute (ledger-processor cold start loads the pools) and
   EventBridge (the allow-list is compiled into the probe).
4. `events-backfill --discover-pools` WRITE.
5. Reprice from ledger 65,099,227 (first swap of `CCNDNFA7…`, 2026-10-23)
   to the deploy.
6. Invoke the probe in a later clock hour than 05:17 UTC.

## Acceptance Criteria

- [ ] `STATIC_POOLS` holds the three pools; a test decodes one real swap of each
- [ ] `CANIE64B…` allow-listed with reason and task
- [ ] Compute and EventBridge deployed; the running Lambdas carry the change
- [ ] No `unregistered_pool_events` WARN for these pools after the deploy
- [ ] Registry rows written; range from ledger 65,099,227 repriced
- [ ] Manual probe run reads `unclassified = 0`; alarm back to OK
