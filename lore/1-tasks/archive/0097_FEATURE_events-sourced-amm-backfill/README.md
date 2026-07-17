---
id: "0097"
title: "Events-sourced AMM backfill — reprice from BE soroban_events (CH-to-CH), no ledger re-download"
type: FEATURE
status: completed
related_adr: ["0009"]
related_tasks: ["0096", "0088", "0053", "0079"]
tags: [layer-indexing, priority-high, effort-large, milestone-M1, backfill, clickhouse, amm, soroswap, reprice, tooling]
milestone: 1
links:
  - "../../../packages/soroswap-extractor/src/lib.rs"
  - "../../../packages/prices-ingest-core/src/soroban.rs"
history:
  - date: 2026-07-17
    status: completed
    who: okarcz
    note: >
      DONE + archived. PR #117 merged (squash e55ef7e). All 4 ACs met and
      VERIFIED ON PROD: the events-backfill crate reprices AMM candles CH-to-CH
      from BE soroban_events via the shared live seam (no ledger re-download);
      2,560,166 candles written over [50457424, 63352611] in 251s; soroswap EXACT
      at all 7 granularities (536,318 trades, identical volume 1m->1M) where it
      was absent from coarse entirely — the 0096 gap is filled AND rolled up;
      SDEX untouched (14,140,873 1d rows, tip 2026-07-09). 202 tests, clippy
      clean under -D warnings on CI's list. Delivered beyond the original scope:
      TWO live bugs found + fixed (Phoenix variable-length swap groups, ~2.1% of
      swaps discarded by an n>=8 gate; safe_log redaction that collapsed EVERY CH
      error to "detail suppressed" in the live Lambda + SDEX backfill), the
      missing dispatch-error counter, a scoped AMM pre-roll (neither existing
      script fit; the runbook referenced the wrong one), and the correction of
      0096's bogus "~824k swaps" baseline (ground truth 536,319). Spawned 0099
      (deploy the Phoenix fix to live — live is STILL ~2% short until then — plus
      the soroswap 2026-07-06 -> 07-15 live-era gap that nothing covers) and 0100
      (triage 144 unregistered AMM-shaped emitters; establish aquarius/phoenix
      swap baselines, which do not exist).
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
  - date: 2026-07-17
    status: active
    who: okarcz
    note: >
      WRITE RUN + PRE-ROLL COMPLETE on prod. events-backfill wrote 2,560,166
      candles to price_ohlcv_1m over [50457424, 63352611] in 251s (aquarius
      3,842,273 / phoenix 242,201 / soroswap 536,318 ticks; 16 phoenix swaps
      failed dispatch = 0.0066%, unpriceable fragments). Pre-roll ran via
      preroll-amm-reprice.sql. VERIFIED: soroswap EXACT at all 7 granularities
      (536,318 trades, identical volume 1m->1M) having been absent from coarse
      entirely; phoenix coarse 242,224 (1h) vs pre-fix 237,026 => the 5,175
      recovered swaps landed in the store of record; no level below 1m for any
      source; SDEX untouched (14,140,873 1d rows, tip 2026-07-09). Coarse totals
      drift slightly ABOVE 1m as buckets widen (aquarius +181/1h, +4,864/1d) —
      expected boundary overhang: the bucket straddling end_ts holds the FULL
      bucket from the earlier pre-roll, not just our window slice; the excess
      matches each source's trade rate at every level. Pre-roll GOTCHA: FINAL is
      MANDATORY (15m holds our rows ON TOP OF pre-existing ones, so 0090's
      "drop FINAL" advice would DOUBLE-COUNT) and a year-bounded FINAL blows the
      5.59 GiB quota (opens 12 partitions, reads all sources incl. sdex, which
      the source filter cannot prune) — month-chunk to the toYYYYMM layout.
      Cleanup rule re-enabled after verification.
  - date: 2026-07-17
    status: active
    who: okarcz
    note: >
      Full-range DRY-RUN verified on prod [50457424, 63352611]: soroswap
      536,318 ticks vs 536,319 swaps measured (off by 1); aquarius 3,842,273;
      phoenix 237,026 -> 242,201 after the fix below. pools=728 resolved=728,
      unresolved 0, dropped 0, no missing closed_at. TWO REAL BUGS FOUND +
      FIXED (PR #117): (1) Phoenix XYK swap groups are VARIABLE length; the
      `n >= 8` gate silently discarded 5,175 real 7-event swaps (~2.1%) — a
      LIVE bug in shared dispatch code, sibling of 0096; live stays wrong till
      the processor is deployed (-> 0099). (2) dispatch errors had NO counter
      (unresolved fallback is Soroswap-scoped), so the summary read
      "swaps dropped: 0" while 5,175 swaps died — took hand-grepping 19,624
      log lines to see; LedgerSoroban now carries dispatch_errors. ALSO: the
      "~824k soroswap swaps" baseline inherited from 0096 is WRONG and not
      reproducible (ground truth 536,319; 824k likely swap+sync via a LIKE
      overmatch) — it sent us on a 35%-shortfall hunt that was chasing a
      phantom. Ops findings: CH user header has no default; safe_log ate every
      CH error code; no SETTINGS allowed under readonly=1 (code 164); the
      coverage probe was fatal + unbounded (code 241). Spawned 0099 (Phoenix
      live deploy + live-era reprice) and 0100 (triage 144 unregistered
      AMM-shaped emitters, 2.25M events). Write run + pre-roll still pending.
---

# Events-sourced AMM backfill — reprice from BE `soroban_events`

## Summary

> ⚠️ **The "~824k Soroswap swaps" figure carried from 0096 is WRONG** — see
> Decision Log 2026-07-17. Measured ground truth is **536,319 swaps** in
> `[50457424, 63352611]`. The 824k number is not reproducible from
> `soroban_events` and appears to have counted `swap`+`sync` (or used a
> `LIKE '%SoroswapPair%'` overmatch, which sweeps in sync/deposit/withdraw/skim
> — ~2× the swap count). Every "824k" below is retained only as the historical
> record of what this task was opened against.

Backfill historical AMM candles (starting with the Soroswap swaps the 0096
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
- [x] Produces non-zero `soroswap` candles in `price_ohlcv_1m` for the historical
      range, per-source verified against the `soroban_events` swap counts.
      **Dry-run VERIFIED on prod 2026-07-17** over `[50457424, 63352611]`:
      `soroswap 536,318` ticks vs **536,319** swaps measured in `soroban_events`
      (off by 1 = 0.0002%); `aquarius 3,842,273`; `phoenix 237,026` → **242,201**
      after the variable-length fix below. `unresolved pools 0`,
      `swaps dropped 0`, `pools=728 resolved=728`, every SoroswapPair event in
      range from a REGISTERED pool (`outside_registry = 0`). The **write** run is
      still pending — runbook §2.
- [x] Pre-rolled into the coarse tables (with 0088 / cleanup coordination); BE's
      1h/1d view shows Soroswap history. **DONE 2026-07-17** via
      `schema/preroll-amm-reprice.sql` (STAGE 0 phoenix delete → STAGE 1 15m ←
      1m → STAGE 2 month-chunked chain). Verified: **soroswap is EXACT at all 7
      granularities** (536,318 trades / 2,849,662,048.74 volume_base identical
      from 1m through 1M) — it was absent from `1d` entirely before. Phoenix
      coarse reads 242,224 (1h) / 242,318 (1d) vs the pre-fix 237,026, so the
      5,175 recovered swaps DID land in the store of record (what STAGE 0's
      delete existed to guarantee). No level is BELOW `1m` for any source → no
      lost buckets, no stale row surviving. SDEX untouched: 14,140,873 `1d` rows,
      tip 2026-07-09, unchanged.
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

## Decision Log

Full findings from the first prod run — measured baselines, two real bugs, and the
operational traps — are in
[`notes/S-prod-dry-run-findings.md`](notes/S-prod-dry-run-findings.md). Headlines:

- **The inherited "~824k Soroswap swaps" baseline is WRONG** — ground truth is
  **536,319**; we extract 536,318. It cost a long hunt for a phantom 35% shortfall.
  Count Soroswap swaps by topic[1], never `LIKE '%SoroswapPair%'` (which matches
  sync/deposit/withdraw too, ~2× the swaps).
- **Phoenix XYK groups are VARIABLE length** — the `n >= 8` gate silently dropped
  5,175 real 7-event swaps (~2.1%). A **LIVE** bug in shared dispatch code
  (→ 0099), sibling of 0096.
- **Dispatch errors had no counter** — the summary said `swaps dropped: 0` while
  5,175 swaps died. Fixed via `LedgerSoroban::dispatch_errors`.

## Future Work

Spawned as backlog tasks (2026-07-17):

- **0099** — deploy the Phoenix variable-length swap fix to live + reprice the
  live-era gap. The fix landed here in SHARED code (`ledger-processor` /
  `phoenix-extractor`), so live Phoenix stays ~2% short until deployed; 0097
  only repriced `[50457424, 63352611]`. Also owns the split decision if PR #117
  is separated into backfill vs live.
- **0100** — triage the 144 unregistered AMM-shaped emitters (2.25M swap/trade
  events) the coverage probe found. Confirmed NOT Soroswap; unknown whether
  they are unregistered aquarius/phoenix pools (real lost volume) or non-AMM
  noise. Includes establishing measured swap baselines for aquarius/phoenix —
  neither has one, unlike Soroswap's 536,319, so their tick counts are
  currently unverifiable.

## Notes

- Root cause + fix context: [[task-0096-soroswap-root-cause]].
- BE repo + events schema: [[be-repo-path]], `lore/3-wiki/project/soroban-events-schema.md`.
- The live catch-up (0064/0094), once the fixed processor is deployed, produces
  Soroswap for the tail it still has to process — this task covers the older span.
- Prod CH access: [[hetzner-ch-prod-ssh-access]]; operator runs prod queries
  ([[feedback-user-runs-prod-ch-queries]]).
