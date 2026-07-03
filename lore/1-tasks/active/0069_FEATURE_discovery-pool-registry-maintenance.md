---
id: "0069"
title: "Asset Discovery — Soroswap/Aquarius pool-registry maintenance"
type: FEATURE
status: active
related_adr: ["0007"]
related_tasks: ["0039", "0054", "0037"]
tags: ["phase-future", "effort-small", "priority-low", "discovery", "amm"]
links: []
history:
  - date: 2026-06-26
    status: backlog
    who: claude
    note: "Spawned from 0039 Step 5 future work — the additive pool-registry piece was not delivered in PR #56 (oracle/supply/cleanup/MV + discovery-via-0054 shipped instead)."
  - date: 2026-07-03
    status: active
    who: okarcz
    note: >
      Promoted backlog → active to start implementation. Confirmed still a real
      gap (not built elsewhere): asset-discovery writes prices.assets only;
      prices.pool_registry has a single writer (sdex-backfill CLI); no periodic
      pool-discovery worker or EventBridge rule exists; the live processor loads
      the registry once at cold start and never refreshes/persists. All deps
      (0037/0054/0039) are completed, so unblocked. Not a 0070 go-live gate —
      this is the post-deploy robustness layer (survive cold starts, catch pools
      the live factory-event stream misses).
---

# Asset Discovery — Soroswap/Aquarius pool-registry maintenance

## Summary

Extend the Asset Discovery worker (shipped by task 0054, reused by 0039) with
Soroswap / Aquarius pool-pair registry maintenance. Pool registries tell the
Ledger Processor (0038) which AMM contracts to extract swaps from; without
periodic maintenance, newly-created pools on those protocols are missed on the
live path.

## Context

0039 Step 5 (Q#2 → Option A) reused 0054's discovery binary as-is and only
*planned* to add this pool-registry maintenance on top. The 0039 implementation
(PR #56) delivered the oracle, supply, cleanup workers and the `current_prices`
MV, plus discovery via 0054 — but the pool-registry extension was not built. It
is carried out of 0039 as this standalone follow-up. Related: the AMM
historical-pool-discovery gap (factory-registry seeding) and the 0037 Phoenix
pool-registry surface.

## Implementation

- Add periodic Soroswap / Aquarius pool-pair discovery to the discovery worker
  (or a sibling step on the same `rate(1h)` rule).
- Persist the discovered pools to the registry the Ledger Processor reads to
  decide which contracts to extract swaps from.
- Coordinate the registry hand-off with the 0037 Phoenix pool-registry surface
  so all three AMM protocols share one registry contract/shape.

## Acceptance Criteria

- [x] New Soroswap/Aquarius pools created in-window are added to the pool
      registry within one discovery cycle — `discover_window` grows the
      `Registries` from in-window factory events (`process_ledger` →
      `learn_factory`) and persists it every hourly run.
- [x] The Ledger Processor picks up the maintained registry (extracts swaps
      from the newly-registered pools) — it already `load_pool_registry()`s at
      cold start (task 0078). *Caveat (documented):* the live processor reads the
      registry at cold start only, so a newly-persisted pool becomes resolvable
      on its next reload; live-created pools are already resolved in-stream from
      their own factory event, so this maintenance is the durability layer that
      survives cold starts, not the primary live path.
- [x] Registry shape is consistent with the 0037 Phoenix surface — reuses the
      exact `PoolRegistryRow` / `to_pool_rows` shape (all three venues) that
      0053 decision #4 / 0037 defined; no new shape introduced.

## Implementation Notes

Folded into the existing hourly **asset-discovery** worker — **no new Lambda or
EventBridge rule**. The worker already ran `process_ledger` over its rolling
window purely to harvest AMM token *identities*, discarding the pool
`Registries`. The change turns that discard into durable maintenance.

Files:
- `packages/prices-ingest-core/src/writer.rs` — new
  `OhlcvWriter::write_pool_registry(&Registries)`, the durable counterpart of
  the existing `load_pool_registry`, so read+write share one row shape / table
  name and can never drift.
- `packages/sdex-backfill/src/sink.rs` — `write_pool_registry` now delegates to
  the shared writer method inside its retry wrapper (behaviour identical; the
  inlined insert is gone → one writer, no drift).
- `packages/asset-discovery/src/lib.rs` — `register_ledger_assets` takes a
  caller-owned `&mut Registries` (was a throwaway); `discover_window` loads the
  persisted registry, grows it across the window, and **persists before advancing
  the cursor** (crash-safety), then reports `pools_total` in `DiscoveryStats`.
- `packages/asset-discovery/src/main.rs` — logs / returns `pools_total`.

Tests:
- `register_ledger_assets_preserves_preseeded_pools` (pure unit) — a loaded pool
  survives a scan with no factory events (rolling re-scan is additive, never
  clobbering).
- `discover_it.rs` integration (gated on local CH) — truncates + asserts the
  persisted registry round-trips the reported `pools_total`. Ran green against
  the local CH `26.3.10.60` (prod-pinned).

## Design Decisions

### From Plan

1. **Persist discovered pools to `prices.pool_registry`, consistent with the
   0037 surface.** Reused the `PoolRegistryRow` shape verbatim — all three
   venues already round-trip through it (task 0053).

### Emerged

2. **Discovery mechanism is factory-event scan, not a Soroswap/Aquarius API
   integration.** The task text (via 0054's note) floated "API integration", but
   task 0053 decision #4 had already inverted 0069's premise to
   *registry-as-output*: pools are learned from in-window factory events by the
   shared `process_ledger`/`learn_factory` path. Adding an out-of-band API poller
   would be a second, divergent discovery surface. Chose to reuse the established
   in-stream mechanism the backfill and live processor already share.

3. **Folded into the existing asset-discovery worker; no new infra.** The task
   allowed "a sibling step on the same `rate(1h)` rule". Since asset-discovery
   already scanned the same ledgers and already ran `process_ledger`, adding pool
   persistence there is a few lines and zero new AWS resources — versus a new
   Lambda + EventBridge rule that would re-scan the same ledgers. Kept the hot
   live path read-only on the registry (its deliberate design); the periodic
   worker owns durable maintenance.

4. **Extracted the shared `OhlcvWriter::write_pool_registry`.** The insert was
   inlined only in sdex-backfill's sink. Rather than copy it into
   asset-discovery, lifted it into the core writer (mirroring the already-shared
   `load_pool_registry`) so both writers can never drift — matching the codebase's
   stated single-query-shared philosophy.

## Future Work

- The live ledger-processor reads `pool_registry` only at cold start (no hot
  reload). If sub-cold-start freshness for pre-cursor pools ever matters, a
  periodic in-place registry refresh on the live processor is the follow-up —
  out of scope here and likely unnecessary given in-stream live discovery.
