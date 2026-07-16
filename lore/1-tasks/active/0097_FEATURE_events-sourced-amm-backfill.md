---
id: "0097"
title: "Events-sourced AMM backfill — reprice from BE soroban_events (CH-to-CH), no ledger re-download"
type: FEATURE
status: active
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
  - date: 2026-07-16
    status: active
    who: okarcz
    note: >
      Promoted to active. Starting implementation of the events-sourced
      repricer (CH-to-CH AMM backfill) on a dedicated branch.
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

- [x] Events-sourced repricer reuses the live extraction pipeline (no divergent
      SQL); dedupes the RMT doubling; idempotent writes. **Code-complete** — new
      `events-backfill` crate drives the shared `process_soroban_event_rows` seam;
      RMT doubling deduped (ledgers via `GROUP BY sequence`, events adjacently);
      writes are RMT-idempotent by `version`. 39 tests green.
- [ ] Produces non-zero `soroswap` candles in `price_ohlcv_1m` for the historical
      range, per-source verified against the `soroban_events` swap counts.
      **Operator-run** (needs `ch-prod-01` / BE tables) — runbook §1–3.
- [ ] Pre-rolled into the coarse tables (with 0088 / cleanup coordination); BE's
      1h/1d view shows Soroswap history. **Operator-run** — runbook §4 (cleanup
      disabled → reprice → incremental pre-roll → re-enable).
- [x] Generalizes: documented as the reusable "reprice from BE events" path for any
      future extractor gap. `docs/runbooks/events-sourced-amm-reprice.md`.

## Implementation Notes

**Landed (code-complete, this branch):**

- **Reuse seam** — `prices-ingest-core/src/soroban.rs`: new public
  `process_soroban_event_rows(ledger_seq, closed_at, &[RawSorobanEvent], reg,
  assets, out)` + `RawSorobanEvent` (both re-exported). Mirrors the AMM path of
  `process_ledger` exactly but sourced from CH rows instead of XDR — groups by
  tx, runs `learn_factory` → `classify_amm_groups` → `dispatch` →
  `amm_trade_to_tick`. Oracle (`REFLECTOR`/`REDSTONE`) skipped (out of scope).
  `learn_factory` refactored to take `(topics, data)` `Value`s so both the live
  and seam callers share it. 2 new unit tests (typed-JSON swap → soroswap tick;
  factory-learn + oracle-skip + tx-grouping).
- **New crate** `packages/events-backfill` (modeled on `sdex-backfill`): `cli`,
  `error`, `source` (CH reads), `run` (orchestration), thin `main`. Reads
  `default.soroban_events` filtered to seeded-registry contract ids, joins
  `default.ledgers` for `closed_at`, feeds the seam, accumulates per-source
  candles, writes `prices.price_ohlcv_1m` via the shared `OhlcvWriter`. `--dry-run`
  prints per-source tick counts for pre-write verification.

**Remaining (operator-run, needs prod/BE ClickHouse):** execute the reprice +
per-source verification + incremental pre-roll (runbook). Not run here per
[[feedback-local-only-no-prod-data]] / [[feedback-user-runs-prod-ch-queries]].

## Design Decisions

### From Plan

1. **New public seam over promoting privates** (confirmed): a single
   `process_soroban_event_rows` in `soroban.rs` keeps `classify_amm_groups` /
   `amm_trade_to_tick` / `learn_factory` private and un-leaked, guaranteeing the
   reprice cannot drift from live. New crate `packages/events-backfill`; single
   `default`-user CH client (reads `default.*`, writes `prices.*`).

### Emerged

2. **Contract-level read filter, not signature/topic** — Phoenix XYK needs its
   full **8 micro-events** (all NULL-signature, topic0 = `String("swap")`) and
   Soroswap swaps are NULL-signature too (topic0 = `String("SoroswapPair")`), so
   filtering by `signature`/topic content would break extraction. The read
   filters on the **numeric `contract_id`** of the seeded registry pools — which
   also gets primary-index granule pruning (the table is `ORDER BY (contract_id,
   …)`). Consequence: the unknown-contract "unresolved" safety-net path is
   effectively inert (we only fetch known pools), which is correct given the
   registry is fully seeded for the historical range.
3. **`closed_at` sourced from `default.ledgers`** — `soroban_events` has no
   close-time column, but candles bucket by ledger close time. Joined via a
   `GROUP BY sequence` subquery (dedupes the `ledgers` RMT) → `toUnixTimestamp`
   seconds.
4. **RMT doubling deduped without full-table `FINAL`** — `ledgers` collapsed by
   `GROUP BY sequence` in the join; `soroban_events` duplicates removed adjacently
   in the run loop by `(contract_id, transaction_id, event_index)` (rows arrive
   ordered by that key). Avoids an expensive `FINAL` scan on a huge table.
5. **Assets written per chunk** — newly-minted surrogate ids are persisted after
   each chunk's candles so a crash mid-run never leaves candles referencing
   asset_ids absent from `prices.assets`.
6. **No `backfill_progress` wiring (v1)** — idempotent writes make re-run the
   resume mechanism; the `soroban_amm` progress stream is already `completed`
   (0090). A progress row for reprices is a possible follow-up if operator
   visibility is wanted.

## Notes

- Root cause + fix context: [[task-0096-soroswap-root-cause]].
- BE repo + events schema: [[be-repo-path]], `lore/3-wiki/project/soroban-events-schema.md`.
- The live catch-up (0064/0094), once the fixed processor is deployed, produces
  Soroswap for the tail it still has to process — this task covers the older span.
- Prod CH access: [[hetzner-ch-prod-ssh-access]]; operator runs prod queries
  ([[feedback-user-runs-prod-ch-queries]]).
