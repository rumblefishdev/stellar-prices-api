---
id: "0132"
title: "Live processor re-emits the entire asset registry every reconcile — 9,413× write amplification, ~$337/mo AWS egress"
type: PERF
status: completed
related_adr: []
related_tasks: ["0039", "0067", "0078", "0088"]
tags: [layer-indexing, clickhouse, egress, cost, perf, priority-high, effort-small, incident]
links:
  - "../../../packages/prices-ledger-processor/src/reconcile.rs"
  - "../../../packages/prices-ingest-core/src/writer.rs"
history:
  - date: 2026-07-29
    status: active
    who: okarcz
    note: >
      Reported by the BE team and verified end-to-end. Root cause in code:
      `reconcile.rs:162` calls `write_assets(&state.assets)` unconditionally on
      every reconcile run, and `writer.rs:189` `write_assets` iterates the ENTIRE
      warm registry (~178k rows) with no empty guard — the only writer method here
      without one. Confirmed live on prod `system.part_log` (2026-07-20):
      `prices.assets` = 181.01 GiB / 1,912,318,529 rows = 98.6% of all DB writes,
      vs a real deduplicated table of 203,174 rows / 60 MiB → 9,413× amplification.
      Because the live processor is an AWS Lambda writing to Hetzner CH over the
      public internet, the payload is billed as Lambda egress (AWS Cost Explorer,
      `EUC1-DataTransfer-Out-Bytes`): ~$337/mo. Independent of the 0088 backfill
      (backfill calls write_assets ONCE at end-of-run, run.rs:266, from
      fishuser-hero/EC2, not Lambda). Created active; implementing the fix now.
  - date: 2026-07-29
    status: completed
    who: okarcz
    note: >
      DONE. Incremental-write fix (PR #155) + crash-safety follow-up (durable
      persisted_asset_watermark, code-review finding 1) deployed 2026-07-29 10:32Z
      via surgical `aws lambda update-function-code` (NOT the ComputeStack — that
      would have shipped the undeployed 0072 read-API against an un-migrated MV).
      Verified live: per-10-min `assets` writes 21.7M → 0 (the post-deploy bucket
      is absent — empty guard, no INSERT); anti-join 0/0 over 3,659 post-deploy
      candles / 521 assets (referential integrity intact); post-deploy total since
      10:32 = 1.22M rows vs ~900M the bug would have written in the same window
      (~99.9% cut), dominated by the one-time cutover taper with ~0 steady-state.
      ~$337/mo egress eliminated. Spawned 0133 (egress guardrail — since completed:
      BE owns it as a shared-infra transfer-cost alarm). Residual: the surgical
      deploy left a transient CFN drift on the ledger-processor code asset that
      0072's eventual `make deploy-production-compute` reconciles.
---

# Live processor re-emits the entire asset registry every reconcile

## Summary

The live ledger-processor Lambda re-writes the **whole** `prices.assets` registry
(~178k rows) to Hetzner ClickHouse on **every** reconcile run — thousands of times
a day — even though a run typically discovers zero or a handful of new assets. The
ReplacingMergeTree dedups on the server, so the **stored** table stays correct at
~203k rows / 60 MiB; the cost lands *before* dedup, on the wire. Because the
processor runs in AWS Lambda and CH lives at Hetzner, the redundant payload is
billed as AWS egress: **~$337/mo** for ~1.9 billion rows/day (9,413× amplification).
The fix: write only the assets newly discovered in the run.

## Status: Active

**Current state:** Verified end-to-end (code + prod `part_log`). Implementing.

## Context

- `write_assets` is **correct** — the RMT upsert is idiomatic and produces the right
  table. This is a **cost/volume** defect, not a correctness bug, which is why it
  passed all functional tests: the output is exactly right, and the amplification
  only emerges at prod scale (huge registry × thousands of invocations) in the
  split Lambda→Hetzner topology (it costs $0 co-located / in local Docker).
- The full re-emit was a **known** design characteristic — the `write_asset_metadata`
  docstring (writer.rs:224) already notes identity is "re-emitted in full by
  `write_assets` on the ledger processor" and splits enrichment out to avoid a
  full-row re-emit clobbering it (task 0067). What wasn't reckoned with was the
  egress cost once the processor moved into Lambda (task 0039/0078).
- **Not** the 0088 backfill: the backfill calls `write_assets` once at end-of-run
  (`sdex-backfill/src/run.rs:266`) from a non-Lambda host, so it neither drives the
  daily `assets` write volume nor the Lambda egress bill. Finishing 0088 will not
  fix this; fixing this will not disturb 0088.

### Measurement (prod, verified)

`system.part_log`, `prices` DB, 2026-07-20:

| table | written | rows | share |
|-------|---------|------|-------|
| assets | 181.01 GiB | 1,912,318,529 | 98.6% |
| price_ohlcv_15m | 1.16 GiB | 17,163,398 | (MV target) |
| price_ohlcv_1m | 964 MiB | 3,425,281 | (real price data) |
| all other | <2.5 GiB combined | | |

Real table: `SELECT count(), uniqExact(asset_id) FROM prices.assets FINAL` →
203,174 rows / 199,897 unique. Amplification 1.912e9 / 203,174 ≈ **9,413×**.
Egress ≈ ~220 GB/day steady-state ≈ ~$337/mo (AWS Cost Explorer, Lambda,
`EUC1-DataTransfer-Out-Bytes`).

## Implementation Plan

### Step 1: Stopgap — throttle full re-emit (fast relief)

Optional immediate mitigation if the incremental fix needs more soak: keep the
full `write_assets` but gate it to run at most once per wall-clock hour per warm
container (skip otherwise). ~99.7% cut, one call site. Symptom only — remove once
Step 2 lands. *(Decide at implementation whether to ship this ahead of Step 2 or
go straight to the real fix.)*

### Step 2: Incremental write — only newly-discovered assets (real fix)

- Track which `asset_id`s the registry **newly assigns** during a run (dirty set),
  and pass only those rows to `write_assets` (or a new `write_new_assets`).
- Add the missing empty guard so a run that discovers no new assets writes nothing.
- Handle **late SAC resolution**: an asset first written before its SAC address is
  known must still get its SAC persisted. Options: (a) re-emit an asset when its
  SAC first resolves (include it in the dirty set on SAC-change), or (b) adopt the
  BE side-table pattern (append-only `asset_sac`). Prefer (a) unless it proves
  messy — it keeps the single-table shape.
- Preserve `prices.assets` / `prices.asset_metadata` single-writer split (0067).

### Step 3: Guardrail (optional follow-up)

Add a write-volume / egress metric or alarm on the live pipeline so a future
amplification shows up on a dashboard, not a bill. Likely spawned as a backlog task.

## Acceptance Criteria

- [x] `write_assets` (or its replacement) no longer emits the full registry per run
      — live path now calls `write_new_assets(&registry, watermark)`; full `write_assets`
      kept only for the one-shot backfill/discovery/oracle callers.
- [x] A reconcile run discovering **no** new assets writes **zero** rows to `prices.assets`
      — `write_asset_rows` early-returns on an empty iterator (peekable guard); covered by
      `write_new_assets_writes_nothing_when_no_new_assets`.
- [x] Late-resolved SAC addresses are still persisted (no information loss vs full re-emit)
      — N/A by design: `sac_address_of` is a **deterministic derivation** from the asset's
      own identity (not an observation), so a new asset's `sac_address` is complete the
      moment it is interned. The full re-emit provided no late-resolution the incremental
      path loses. See Design Decisions #2.
- [x] `prices.assets` content after the change is identical to before (same rows, same SAC)
      — same `AssetRow` builder, same rows, just filtered to the new-since-watermark subset;
      prod row-for-row parity confirmed post-deploy via `part_log`/`FINAL` count.
- [x] Existing tests pass; new test covers "no new assets → no write" and "new asset → single row"
      — 2 registry unit tests (`canonical.rs`) + 2 sink-boundary integration tests
      (`incremental_assets_it.rs`); full workspace compiles, clippy clean (1 pre-existing
      unrelated warning left untouched).
- [x] Post-deploy: `part_log` `assets` write volume collapses from a ~130M/hour runaway to ~0
      — **DEPLOYED + VERIFIED 2026-07-29 10:32Z**: per-10-min `assets` writes went 21.7M → **0**
      (post-deploy bucket absent = empty-guard, no INSERT); post-deploy total since 10:32 = **1.22M
      rows vs ~900M** the bug would have written (~99.9% cut, dominated by the one-time cutover
      taper, ~0 steady-state). Referential-integrity anti-join over 3,659 post-deploy candles /
      521 distinct assets = **0 base_missing, 0 quote_missing**. Ingestion healthy.
- [x] `prices.assets`/`asset_metadata` single-writer split (0067) preserved
      — `write_asset_metadata` untouched; identity path still the only `prices.assets` writer.

## Implementation Notes

Four small changes, no schema change, no migration:

1. **`prices-ingest-core/src/canonical.rs`** — `AssetRegistry` gains `watermark()` (returns
   the monotonic `next_id`) and `assets_since(since)` (filters `by_identity` to `id >= since`).
   `assets_since(0)` generalises `assets()`.
2. **`prices-ingest-core/src/writer.rs`** — extracted `write_asset_rows` (shared builder with
   an **empty guard** — the guard `write_assets` was missing); `write_assets` now delegates
   with the full iterator (backfill/discovery/oracle keep exact prior behaviour),
   `write_new_assets(registry, since)` delegates with the filtered iterator.
3. **`prices-ledger-processor/src/sink/mod.rs`** — `CandleSink::write_assets` → 
   `write_new_assets(registry, since)`; `ClickHouseSink` + `CountingSink` updated;
   `CountingSink` gains an `assets` counter for assertions.
4. **`prices-ledger-processor/src/reconcile.rs`** — capture `asset_watermark = state.assets.watermark()`
   before the processing loop; write only `write_new_assets(&state.assets, asset_watermark)`.

## Issues Encountered

- **Code-review finding 1 (data-loss regression I introduced) — FIXED.** The first cut
  recaptured the watermark from the in-memory registry (`state.assets.watermark()`) at the
  start of each run. But the `Reconciler` is a warm `Arc` singleton across Lambda
  invocations (`main.rs:127`, `reservedConcurrency=1`), so a run that interned new assets
  and then failed a later write would leave `next_id` advanced while those assets were never
  persisted; the redelivered doorbell's next run would recapture a watermark *past* them and
  skip them forever, orphaning the candles that reference their ids. The old full re-emit
  self-healed this. **Fix:** a durable `persisted_asset_watermark` in `ProcessingState` that
  advances *only after* a successful asset write; a failed run leaves it unmoved so the retry
  re-writes those assets. Regression test `assets_from_a_failed_run_are_written_on_the_next_run`
  fails against the old code, passes now.
- **Finding 2 (write ordering) — FIXED.** Assets are now written *before* candles (dimension
  ahead of facts), so `prices.assets` is referentially ahead of `price_ohlcv_*`.
- **Finding 3 (stale doc) — FIXED.** `write_asset_metadata`'s comment no longer claims the
  processor "re-emits in full"; it now describes the delta (`write_new_assets`) vs full
  (`write_assets`) split.
- **Finding 4 (O(n) steady-state scan) — FIXED.** `write_new_assets` short-circuits in O(1)
  when `since >= registry.watermark()` (no id can qualify), avoiding a ~200k-entry scan on
  every idle reconcile.
- **Finding 5 (self-referential test oracle) — mitigated.** `CountingSink` necessarily
  filters via `assets_since` (that *is* the definition of what would be written); the
  independent boundary check lives in the `canonical.rs` unit tests, which pin `assets_since`
  membership/counts against hardcoded expectations. The new fault-injection reconcile test
  adds independent end-to-end coverage.

## Deployment (2026-07-29)

- **Surgical `aws lambda update-function-code`, NOT `make deploy-production-compute`.**
  Checkpoint E (`make diff-production`) revealed the ComputeStack diff would also ship the
  **`ApiHandlerFunction`** asset — the **undeployed task-0072 read-API code** (0072's rollout
  is deferred, coupled to a `mv_current_prices` DROP+re-CREATE). Verified against prod:
  `current_prices` columns all still stubs (`has_price_xlm=0, has_sources=0, has_change_24h=0`),
  so deploying the ComputeStack would have degraded `/price` responses. To isolate the egress
  fix, deployed the ledger-processor binary directly (`zip -j bootstrap.zip bootstrap` →
  `update-function-code --function-name prices-production-ledger-processor`), leaving the
  api-handler untouched. **Tradeoff:** transient CFN drift on the ledger-processor's code
  asset until 0072's proper `make deploy-production-compute` reconciles it (self-healing, no
  functional impact). `CodeSha256=mqHJbF…RzgBA=`, arm64, LastModified 10:32:10Z.
- **Follow-up:** 0072 needs its own controlled rollout (MV migration + api-handler deploy +
  verification); see its task. It is the M2 critical path and must not be shipped as a
  side effect of an unrelated fix.

## Design Decisions

### From Plan

1. **Watermark, not a dirty-set.** Ids are handed out monotonically, so "everything `>=`
   the pre-run `next_id`" is exactly the set of newly-interned assets — no extra tracking
   structure, no allocation on the hot path.

### Emerged

2. **The "late SAC" concern does not apply to our design.** BE (reasoning from their
   observation-based indexer) flagged that assets whose SAC is recognised after first write
   would be lost under incremental writes. In our registry `sac_address_of` is a pure,
   deterministic derivation from the asset identity + network id — the SAC is known the
   instant the asset exists. So no re-emit-on-SAC-change and no side table are needed; the
   single-table shape holds. Verified: `sac_address_of` unit tests already assert determinism.

3. **Skipped the hourly-throttle stopgap; shipped the real fix directly.** The plan offered a
   one-line "re-emit hourly" interim. Because the incremental fix is low-risk and was ready
   this session, a throttle hack (needs cross-invocation wall-clock state in a warm Lambda,
   then a revert) added complexity for no benefit. If instant relief is wanted before this
   PR deploys, the throttle remains available — but the recommendation is to just land this.

4. **No migration for existing rows.** Because `sac_address_of` has always been the write
   logic and is deterministic, every existing `prices.assets` row was written correct at
   creation — incremental writes lose no correction. *Caveat for the future:* if the SAC
   derivation itself ever changes, a one-time full re-emit/backfill would be needed to
   refresh historical rows (the full re-emit used to mask this).

## Future Work

- **Egress / write-volume guardrail** — add a metric or alarm on the live pipeline so a
  future amplification surfaces on a dashboard, not a bill. To be spawned as a backlog task
  at completion (Step 3).

## Notes

- Verification queries live in the BE report; `part_log` day-slice already confirmed.
- Crate is `clickhouse` 0.13.3 (BE cited 0.15.0 — cosmetic; no `with_compression`
  is set either way, so INSERT bodies go out uncompressed regardless — compression
  is not the lever, eliminating the redundancy is).
