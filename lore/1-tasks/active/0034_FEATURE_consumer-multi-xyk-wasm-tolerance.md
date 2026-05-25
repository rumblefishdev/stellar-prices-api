---
id: "0034"
title: "Consumer must tolerate >=2 Phoenix XYK WASM builds (PHO/USDC currently dropped if hash-keyed)"
type: FEATURE
status: active
related_adr: ["0006"]
related_tasks: ["0032", "0018", "0037"]
tags: [layer-indexing, priority-medium, effort-small, milestone-M1, phoenix, consumer, stream-1, price-feed-correctness]
milestone: 1
links:
  - "../archive/0032_RESEARCH_phoenix-stable-pool-first-observation/notes/S-no-stable-pool-deployed.md"
  - "../archive/0018_RESEARCH_decode-per-amm-swap-event-shapes/notes/G-amm-swap-event-shapes.md"
  - "../backlog/0037_FEATURE_tranche1-ledger-processor-skeleton.md"
history:
  - date: 2026-05-15
    status: backlog
    who: oski
    note: "Spawned from 0032 negative-result survey."
  - date: 2026-05-18
    status: active
    who: oski
    note: "Activated to begin implementation."
  - date: 2026-05-18
    status: blocked
    who: oski
    by: ["0037"]
    note: >
      Re-blocked on discovery that no consumer code exists yet. The
      Tranche 1 Ledger Processor (per ADR 0006) has not been
      scaffolded — packages/ is empty, only tools/dump-swap-events
      exists. There is no pool registry to attach pool_type to and
      no extractor dispatch to route 8/6-event groupings through.
      Spawned 0037 as the minimal prerequisite: Tranche 1 Ledger
      Processor skeleton with a Phoenix pool registry and extractor
      dispatcher hook. 0034 resumes once 0037 lands.
  - date: 2026-05-25
    status: active
    who: oski
    note: "Activated to begin implementation of multi-XYK WASM tolerance."
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

- [x] Consumer's Phoenix venue lookup does not silently drop pools
      whose WASM hash differs from the most common XYK build.
- [x] Classifier documented as `pool_type + event_count`, with a unit
      test covering both XYK pool variants by config-fixture.
- [ ] PHO/USDC swaps (pool `CD5XNKK3...IAA`) verified end-to-end
      through the consumer in a staging run. (deferred — requires live environment)

## Implementation Notes

### Crates created

This task also delivered the 0037 skeleton as a prerequisite since no
consumer code existed. Three new workspace members under `packages/`:

| Crate | Path | Role |
|-------|------|------|
| `extractors-core` | `packages/extractors-core` | `SwapExtractor` trait, `SorobanEventRow`, `TaggedValue`, `TradeRow`, `Venue` enum — transcribed from 0018 Appendix A |
| `phoenix-extractor` | `packages/phoenix-extractor` | `PhoenixPoolRegistry` (contract_id → pool_type lookup) + `PhoenixXykExtractor` (8-event grouping decoder) |
| `ledger-processor` | `packages/ledger-processor` | lib + stub binary; `dispatch()` routes by venue, then `(pool_type, event_count)` for Phoenix |

### Classifier design

`PhoenixPoolRegistry` keys lookup by **contract_id** and stores
`pool_type: u32` from the factory's `query_config()`. WASM hash is
stored as `Option<[u8; 32]>` metadata but is **never consulted for
extractor selection**. Routing logic in `dispatch_phoenix()`:

- `pool_type == 0` AND `rows.len() >= 8` → `PhoenixXykExtractor`
- `pool_type != 0` AND `rows.len() >= 6` → stable path (stub, no
  mainnet stable pools exist yet per 0032)

This survives future Phoenix XYK rebuilds without code changes.

### Tests (13 total)

**phoenix-extractor (8 tests):**
- Registry fixture construction + lookup for both WASM variants
- Proof that different WASM hashes both resolve as XYK via pool_type
- XYK extractor: 8-event group decode, PHO/USDC alt-WASM pool,
  insufficient rows rejection, unordered field tolerance

**ledger-processor (5 tests):**
- Dispatch routes XLM/USDC (common WASM) correctly
- Dispatch routes PHO/USDC (alt WASM) identically
- Explicit proof that dispatch uses pool_type, not WASM hash
- Unknown venue skipped, empty rows return empty

### CI

Added `.github/workflows/rust.yml` — runs `cargo check`, `cargo test`,
`cargo clippy` on PRs touching `packages/` or `Cargo.*`.

## Design Decisions

### From Plan

1. **`pool_type + event_count` classifier**: per 0032 S-note §"So what?"
   recommendation. WASM hash stored but never used for routing.

2. **Per-venue extractor trait**: `SwapExtractor` with
   `extract(&[SorobanEventRow]) -> ExtractResult` per 0018 Appendix A.

### Emerged

3. **Absorbed 0037 skeleton into this task**: no consumer code existed,
   so the 0037 crate layout was a prerequisite. Built the minimum
   skeleton (3 crates) needed for 0034's classifier to compile and test.

4. **Field-name-based extraction over positional**: the XYK extractor
   matches fields by `topic[1]` string name rather than relying on
   emission order. This tolerates reordered events within a group
   (tested explicitly).

5. **`TaggedValue` enum for CH-level data**: models BE's tagged-JSON
   encoding (`type` + `value`) from `R-be-storage-format.md` rather
   than raw XDR `ScVal`. This is what the consumer actually reads from
   ClickHouse.

6. **Scoped clippy in CI**: runs clippy only on the three new crates,
   not workspace-wide, because `sdex-backfill` has pre-existing clippy
   issues unrelated to this task.
