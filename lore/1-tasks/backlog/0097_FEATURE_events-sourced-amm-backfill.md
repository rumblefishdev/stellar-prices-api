---
id: "0097"
title: "Events-sourced AMM backfill — reprice from BE soroban_events (CH-to-CH), no ledger re-download"
type: FEATURE
status: backlog
related_adr: ["0009"]
related_tasks: ["0096", "0088", "0053", "0079"]
tags: [layer-indexing, priority-high, effort-large, milestone-M1, backfill, clickhouse, amm, soroswap, reprice, tooling]
milestone: 1
links:
  - "../../../packages/soroswap-extractor/src/lib.rs"
  - "../../../packages/prices-ingest-core/src/soroban.rs"
history:
  - date: 2026-07-15
    status: backlog
    who: okarcz
    note: >
      Spawned from 0096. The Soroswap extractor fix (0096) makes LIVE prices
      correct from deploy on, but the ~824k historical swaps (back to activation)
      still need repricing. Because BE's soroban_events holds the full event
      history (activation → 63.49M), the fill is a CH-to-CH reprice, not a
      multi-day ledger re-download.
---

# Events-sourced AMM backfill — reprice from BE `soroban_events`

## Summary

Backfill historical AMM candles (starting with the ~824k Soroswap swaps the 0096
extractor fix unblocks) by reading **BE's `default.soroban_events`** directly and
running the events through our existing extraction pipeline — a **ClickHouse-to-
ClickHouse reprice**, no ledger archive re-download. Estimated minutes-to-low-hours
vs the ~6-day / multi-day-EC2 ledger re-download.

## Context

0096 found the Soroswap 0-candles root cause (extractor read the swap action from
`topic[0]`; Soroswap uses `topics=[String("SoroswapPair"), Symbol("swap")]`, action
in `topic[1]`) and fixed the extractor. LIVE Soroswap prices from the next
ledger-processor deploy onward. But the **historical** swaps (≈824k, back to ledger
50,688,706 ≈ activation) are already gone from `price_ohlcv_1m` and must be
re-derived.

BE's `soroban_events` (schema: `lore/3-wiki/project/soroban-events-schema.md`) holds
the **full** event history on the same cluster (`events_floor = 50,457,424`,
`events_ceiling = 63.49M`, no retention gap). So we can reprice from events instead
of re-downloading + re-decoding ledgers.

## Implementation (sketch)

- New binary/mode (e.g. `events-backfill`) that:
  1. Reads `default.soroban_events` for AMM contracts, ledger-ordered, **deduped**
     (RMT — use `FINAL` or dedupe on `(contract_id, ledger_sequence, transaction_id,
     event_index)`; raw rows are doubled pre-merge).
  2. Resolves `contract_id` (Int64) → strkey via `soroban_contracts`, parses
     `topics_xdr`/`data_xdr` (JSON, not XDR) into `SorobanEventRow`.
  3. Feeds groups through the **existing** `classify_amm_groups` → `dispatch` →
     extractor → `amm_trade_to_tick` → `CandleAccumulator` → `OhlcvWriter`
     (reuse guarantees byte-identical output to the live path — no SQL reimpl of
     asset resolution / SAC collapse / canonical ordering / decimals).
  4. Writes `price_ohlcv_1m` (idempotent RMT by version), per source.
- Preload the seeded `pool_registry` + assets (same as live/backfill startup).
- Bound by ledger range; resumable; write-idempotent. Then pre-roll into coarse
  tables (coordinate with 0088 / the 0090 cleanup+pre-roll sequence).
- Access: BE tables are cross-DB readable on `ch-prod-01` via `docker exec`
  (default user), not the prices mTLS user — decide the run identity/host.

## Acceptance Criteria

- [ ] Events-sourced repricer reuses the live extraction pipeline (no divergent
      SQL); dedupes the RMT doubling; idempotent writes.
- [ ] Produces non-zero `soroswap` candles in `price_ohlcv_1m` for the historical
      range, per-source verified against the `soroban_events` swap counts.
- [ ] Pre-rolled into the coarse tables (with 0088 / cleanup coordination); BE's
      1h/1d view shows Soroswap history.
- [ ] Generalizes: documented as the reusable "reprice from BE events" path for any
      future extractor gap.

## Notes

- Root cause + fix context: [[task-0096-soroswap-root-cause]].
- BE repo + events schema: [[be-repo-path]], `lore/3-wiki/project/soroban-events-schema.md`.
- The live catch-up (0064/0094), once the fixed processor is deployed, produces
  Soroswap for the tail it still has to process — this task covers the older span.
- Prod CH access: [[hetzner-ch-prod-ssh-access]]; operator runs prod queries
  ([[feedback-user-runs-prod-ch-queries]]).
