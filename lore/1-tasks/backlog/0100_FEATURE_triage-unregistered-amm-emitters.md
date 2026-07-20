---
id: "0100"
title: "Triage the 144 unregistered AMM-shaped emitters (2.25M swap/trade events)"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0097", "0079", "0078"]
tags: [layer-indexing, priority-medium, effort-medium, amm, clickhouse, pool-registry, coverage]
links:
  - "../../../packages/events-backfill/src/source.rs"
history:
  - date: 2026-07-17
    status: backlog
    who: okarcz
    note: >
      Spawned from 0097 future work. 0097's dry-run coverage probe found 144
      contracts OUTSIDE prices.pool_registry emitting 2.25M swap/trade-shaped
      events over [50457424, 63352611]. Confirmed NOT Soroswap (every
      SoroswapPair event in range comes from a registered pool). Unknown
      whether they are unregistered aquarius/phoenix pools (real volume we are
      losing) or non-AMM contracts (correctly ignored).
---

# Triage the 144 unregistered AMM-shaped emitters

## Summary

0097's dry-run coverage probe reported **144 contracts outside
`prices.pool_registry` emitting 2,252,506 swap/trade-shaped events** in
`[50457424, 63352611]`. They are **not** Soroswap — that was measured
directly (`outside_registry = 0` for every SoroswapPair event). What they
*are* is unknown, and the answer decides whether we are silently losing AMM
volume.

## Context

- The probe's heuristic is `signature IN ('swap','trade') OR topics_xdr LIKE
  '%SoroswapPair%'`, which deliberately over-matches: any contract using those
  signature names is caught, AMM or not (aggregators, routers, other DEXes).
- Reads are filtered to registry contracts, so an unregistered pool is
  **invisible to the reprice** — its swaps are never fetched, never counted,
  never reported as dropped. This is the [[amm-historical-pool-discovery-gap]]
  failure mode.
- Aquarius (3,842,273 ticks) and Phoenix (237,026 → 242,201) have **no
  published baseline** to check against, unlike Soroswap's 536,319. If any of
  these 144 are aquarius/phoenix pools, both counts are undercounts and nobody
  would know.
- The Aquarius *router* emits a `swap` summary wrapping the pool-level `trade`
  (task 0087) and is deliberately ignored — expect some of the 144 to be that,
  correctly.

## Implementation

- Resolve the 144 `contract_id`s to strkeys via `default.soroban_contracts` and
  classify each: event shape, signature, topic envelope, first/last ledger,
  event count.
- Decide per class: genuine AMM pool (→ seed into `pool_registry`, then reprice
  its range), known-ignorable (router summaries, aggregators → document), or
  non-AMM (→ document so the probe's noise is explained, not re-investigated).
- If genuine pools are found, reprice + pre-roll their ranges and re-check the
  aquarius/phoenix totals against a real measured baseline.
- Consider tightening the probe's heuristic (or annotating its output by venue)
  so future runs report signal, not a number needing this triage each time.

## Acceptance Criteria

- [ ] All 144 contracts classified; the classification recorded here.
- [ ] Any genuine AMM pools seeded into `pool_registry` + their ranges repriced.
- [ ] Measured baselines established for aquarius and phoenix swap counts (the
      equivalent of Soroswap's 536,319), so their tick counts are verifiable.
- [ ] Probe output either explained (expected non-AMM noise) or tightened.
