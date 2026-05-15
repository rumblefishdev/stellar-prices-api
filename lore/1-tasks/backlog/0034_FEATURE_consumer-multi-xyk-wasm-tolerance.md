---
id: "0034"
title: "Consumer must tolerate >=2 Phoenix XYK WASM builds (PHO/USDC currently dropped if hash-keyed)"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0032", "0018"]
tags: [priority-medium, effort-small, phoenix, consumer, stream-1, price-feed-correctness]
links:
  - "../archive/0032_RESEARCH_phoenix-stable-pool-first-observation/notes/S-no-stable-pool-deployed.md"
  - "../archive/0018_RESEARCH_decode-per-amm-swap-event-shapes/notes/G-amm-swap-event-shapes.md"
history:
  - date: 2026-05-15
    status: backlog
    who: oski
    note: "Spawned from 0032 negative-result survey."
---

# Consumer must tolerate >=2 Phoenix XYK WASM builds

## Summary

Task 0032's factory survey discovered that Phoenix mainnet runs **two
distinct XYK WASM builds** (`167ab414...506c` and `13b158655e...f2ca`),
not one. If the prices-api consumer's Phoenix venue lookup keys
extractor selection off a single WASM hash, the second-WASM pool
(PHO/USDC, `CD5XNKK3...IAA`) gets silently dropped from price feeds.

## Context

Two XYK builds in production:
- 10 pools share `167ab414a226427de34c19947ef9c5cf38c6c0ed91ecf9392f7cef3278ff506c`
- 1 pool (`CD5XNKK3...IAA`, PHO/USDC) uses
  `13b158655e40396957537bf1c528c6542b315930c1c9e0df640f57293c8af2ca`

Both expose the same Soroban interface, same contract-meta description,
same `query_version() = "2.0.0"`, same `Config.pool_type = 0`. The
237-byte WASM delta has not been investigated for event-emission
divergence (see task 0036).

## Implementation

Recommended classifier in the consumer's pool registry (per 0032 S-note):

1. At pool registration time, call `query_config(pool_id).pool_type`
   on the Phoenix factory pool. Store alongside the pool address.
2. Route swap events by `(pool_type, event_count)`:
   - `pool_type == 0` AND 8 events → XYK extractor
   - `pool_type != 0` AND 6 events → stable extractor (per 0018 §3)
3. Do **not** key extractor selection off WASM hash — survives future
   Phoenix XYK rebuilds without code changes.

Alternative if a WASM-set is preferred: maintain a set of accepted XYK
hashes, but seed it with both observed hashes from
[0032 evidence](../archive/0032_RESEARCH_phoenix-stable-pool-first-observation/notes/evidence/phoenix_pool_inventory_2026-05-15.txt)
and add a runtime warning if an unrecognized hash appears.

## Acceptance Criteria

- [ ] Consumer's Phoenix venue lookup does not silently drop pools
      whose WASM hash differs from the most common XYK build.
- [ ] Classifier documented as `pool_type + event_count`, with a unit
      test covering both XYK pool variants by config-fixture.
- [ ] PHO/USDC swaps (pool `CD5XNKK3...IAA`) verified end-to-end
      through the consumer in a staging run.
