---
id: "0053"
title: "Combined single-pass historical backfill (SDEX + Soroban AMM) — full-chain, forward-discovery, dual-stream progress"
type: FEATURE
status: active
related_adr: ["0001", "0003", "0004", "0007", "0009"]
related_tasks: ["0017", "0028", "0034", "0037", "0048", "0052", "0051", "0058", "0026", "0060", "0069", "0073", "0063"]
tags: [layer-indexing, priority-high, effort-large, milestone-M1, stream-1, single-pass, rust, cli, workstation, clickhouse, soroban, amm, soroswap, aquarius, phoenix, sdex]
milestone: 1
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../2-adrs/0009_backfill-direct-write-to-hetzner-clickhouse.md"
  - "../archive/0048_RESEARCH_soroban-events-pricing-decoder-spec/notes/G-soroban-events-pricing-decoder.md"
  - "../archive/0060_FEATURE_prices-clickhouse-crate-combined-backfill-sizing/README.md"
  - "./0069_FEATURE_discovery-pool-registry-maintenance.md"
history:
  - date: 2026-05-21
    status: backlog
    who: operator
    note: >
      Spawned during Tranche 1 task-set creation. ADR 0001 commits
      Stream 1 to a local-CH-sourced workstation CLI; 0017 covers
      the CH instance setup; 0037 covers the dispatch kernel; 0034
      covers Phoenix WASM tolerance; 0048 carries the decoder spec.
      No task owns the actual `soroban-amm-backfill` binary —
      decode loop, bucket to 1-min OHLCV, write to the local prices.*
      CH mirror, run the one-shot completion push to Hetzner CH. This
      task fills the gap.
  - date: 2026-06-19
    status: backlog
    who: claude
    note: >
      Corrected the local staging store from a local Postgres to a
      local ClickHouse mirror of the Hetzner `prices.*` schema (same
      local-CH→Hetzner shape BE uses) — Summary, Context, Steps
      1/5/6/7/8, acceptance criteria, deps. Added volume_quote (0058)
      population + USD columns left DEFAULT 0 for the enrichment pass
      (0026). Repointed provisioning blocker 0050→0063. Replaced
      personal username with "operator".
  - date: 2026-07-01
    status: backlog
    who: claude
    note: >
      **Rescoped: `soroban_events`-sourced separate binary → combined
      single-pass backfill.** ADR 0001's model (BE `backfill-runner`
      populates a local `soroban_events` table that this CLI queries) is
      superseded by the combined single-pass decision locked with the
      operator in task 0060 (2026-06-11) and already built + measured
      there (Soroswap+Aquarius extractors, one-parse SDEX+AMM+oracle
      extraction, prices-clickhouse crate, 100k sizing run). This task now
      owns: (1) extending the single-pass engine in `sdex-backfill` to the
      full chain; (2) **two disjoint range invocations** — a *soroban
      backfill* over `[activation, tip]` extracting **SDEX + AMM** in one
      download pass, and a *sdex backfill* over `[1, activation)` extracting
      **SDEX only** — so no ledger is downloaded twice (download is the
      bottleneck, not parsing; 0060); (3) **forward oldest→newest** decode
      over the Soroban range so every pool's factory-create precedes its
      swaps (organic discovery, no external registry seed); (4) **dual
      progress-row** updates so `/backfill/status` stays truthful; (5) the
      cloud-push (shared with 0028). Dropped the 0017 blocker (its
      `soroban_events` role is dead; the local-CH infra already exists via
      docker-compose + the prices-clickhouse crate). ADR 0001 amended
      in place to record the pivot.
  - date: 2026-07-01
    status: active
    who: claude
    note: >
      Promoted backlog → active after a blocker verification against the
      codebase. **All six blockers are completed/archived** (0034/0037/0048/
      0051/0052/0063) and the code foundation is present and robust:
      SwapExtractor trait (`extractors-core`) + dispatch kernel
      (`ledger-processor/src/dispatch.rs`, unknown pools handled, no panics),
      Soroswap/Aquarius/Phoenix-XYK extractors implemented (not stubs), Phoenix
      multi-WASM tolerance (dispatch routes by `pool_type`), the combined
      single-pass engine already in `sdex-backfill/src/ingest.rs` (SDEX + AMM +
      oracle from one parse), the mTLS client (`prices-clickhouse/src/mtls.rs`),
      and the registry accumulating across partitions within a run. Two apparent
      gaps are non-blocking: the Phoenix stable extractor `unimplemented!()` is
      **unreachable** (dispatch returns a clean error; no mainnet stable pools),
      and the Soroswap-unresolved-pool→empty path (`dispatch.rs:89-92`) is the
      discovery gap the forward-from-activation design already fixes. Remaining
      work is this task's own scope — full-range driver + mode gate, forward
      partition order, persist the discovered registry, dual `backfill_progress`
      updates, and the cloud-push (soft-coordinates with backlog 0028; the local
      extract→mirror bulk is startable now).
  - date: 2026-07-01
    status: active
    who: okarcz
    note: >
      **Push model decided: direct-write (Model B) — ADR 0009.** After checking
      BE's prod approach (`backfill-runner --target clickhouse` writes straight
      to Hetzner over Caddy mTLS) and the A-vs-B analysis, the operator chose to
      write the backfill **directly to Hetzner over the 0052 mTLS client** — no
      local mirror, no separate push CLI, no `assets` remap (ids align via
      load-from-target), and `/backfill/status` updates in real time. Steps 4–5
      rewritten; task 0028 (stage-then-push) superseded by ADR 0009. Already
      shipped on the branch (PR #72): extract-mode gate, the unregistered-pool
      guard → `prices.unresolved_pools`, and the pinned activation ledger
      (50,463,000). Remaining: mTLS sink + real-time dual progress rows +
      minute-aligned split + runbook + integration/idempotency tests.
---

# Combined single-pass historical backfill (SDEX + Soroban AMM)

## Summary

Build the **one-shot combined historical backfill** that populates the
local `prices.*` ClickHouse mirror across the whole chain and pushes it to
Hetzner. It extends the single-pass engine already built and measured in
[task 0060](../archive/0060_FEATURE_prices-clickhouse-crate-combined-backfill-sizing/README.md)
(one S3 download → decode → SDEX candles + Soroban AMM candles + oracle
rows, all `prices.*`) from a 100k sizing slice to the full historical range.

The run is split into **two disjoint ledger-range invocations of the same
engine**, because downloading each ledger's XDR is the dominant cost —
parsing is cheap (0060) — so every ledger is downloaded **exactly once**:

- **Soroban backfill — `[activation, tip]`, extracts SDEX + AMM (+ oracle).**
  The Soroban era. One download per ledger yields *both* classic SDEX
  trades (`source='sdex'`) and AMM swaps (`source∈{phoenix,soroswap,aquarius}`).
  The Soroban-era SDEX comes for free here — it is **not** re-downloaded
  later just to get its SDEX trades.
- **SDEX backfill — `[1, activation)`, extracts SDEX only.** The pre-Soroban
  tail, where no AMM pools can exist, so there is nothing to combine.

Coverage: SDEX = `[1, activation)` + `[activation, tip]` = `[1, tip]`;
AMM = `[activation, tip]`. No ledger downloaded twice.

## Context

### Supersession of the `soroban_events` sourcing model

The original 0053 (and ADR 0001) had a *separate* `soroban-amm-backfill`
binary query a local `soroban_events` table pre-populated by BE's
`backfill-runner --target=clickhouse` (owned by 0017). That model is
**superseded** by the combined single-pass decision locked with the
operator in **task 0060** (2026-06-11) and already implemented there:

- Both SDEX trades and Soroban events live in the **same**
  `LedgerCloseMeta`. `sdex-backfill` already downloads, decompresses and
  parses each ledger once; 0060 wired Soroban event extraction into that
  same pass (Soroswap + Aquarius extractors implemented, oracle extraction
  added) and writes per-source candles to `prices.*`.
- This **removes the dependency on BE's `backfill-runner` / a
  pre-populated `soroban_events` table** (and therefore on 0017's role #1),
  halves the download cost (no second pass over the Soroban era), and makes
  backfilled candles byte-identical to the live processor's (one ledger →
  both extractions).

See [ADR 0001](../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md)
(amended 2026-07-01) for the decision record.

### What 0060 already delivered (do not rebuild)

- `packages/prices-clickhouse` schema crate + `prices-clickhouse-init`.
- Soroswap + Aquarius extractors (Phoenix XYK from earlier work).
- Combined single-pass extraction (SDEX + AMM + REFLECTOR/REDSTONE oracle)
  in the `sdex-backfill` per-ledger loop, writing per-source candles +
  pre-rolled granularities to the local `prices.*` mirror.
- 100k-ledger sizing/timing: ~3.7 KB/ledger, ~61 min/100k, **download-bound**.
- Local CH via `docker-compose.yml` (pinned `26.3.10.60`, auto-applies
  `init.sql`).

### What remains (this task)

Full-history run correctness + operability: forward-discovery for the whole
Soroban range, the two-range split, dual-stream progress accounting, the
cloud-push, and the pool-discovery completeness guard.

## Design decisions (locked)

1. **One engine, two range invocations — not a new binary.** The combined
   single-pass extractor lives in `sdex-backfill` (per 0060). The *sdex
   backfill* is that same engine with AMM extraction a no-op below
   `activation`. Same code, two disjoint ranges.

2. **Download-once.** Ledger download/decompress is the bottleneck (0060);
   parsing is cheap. Never download a Soroban-era ledger twice — extract
   SDEX + AMM together in the `[activation, tip]` pass.

3. **Forward oldest→newest over `[activation, tip]` ⇒ organic pool
   discovery.** A Soroban AMM pool cannot exist before Soroban activation,
   so if the AMM window is exactly `[activation, tip]` and decode proceeds
   chronologically, every pool's factory-create event is seen **before** any
   of its swaps. Discovery is complete **by construction — no external
   factory registry seed is required as a prerequisite** (this downgrades
   [0069](./0069_FEATURE_discovery-pool-registry-maintenance.md) from a
   blocker to an optimization / live-maintenance concern). Guard: assert
   **"no swap decoded for an unregistered pool"** — a fired guard means an
   extractor gap (e.g. the Phoenix multi-WASM / Soroswap `topic[1]` classes
   0060 hit), not silently dropped volume.

4. **Persist the discovered pool registry as an output artifact.** So a
   partial re-backfill (a mid-history window) and the live processor can
   load it without re-deriving from activation. Reuse the live processor's
   persisted-registry mechanism. This inverts 0069: registry-as-output, not
   registry-as-required-input.

5. **Parallel download + strictly-ordered decode.** Download is the
   bottleneck and can be fully parallel (concurrent partition prefetch);
   decode/registration must respect ledger order so create precedes swap.
   Carry registry state forward across partitions.

6. **Dual progress-row update (keeps `/backfill/status` truthful).**
   Per overview §3.5 there are two `backfill_progress` rows: `sdex_archive`
   (`start=1`, `target=tip`, **backward**, `current_ledger`=oldest pushed)
   and `soroban_amm` (`start=activation`, **forward**, one-shot). The
   combined run writes SDEX for `[activation, tip]`, so it must advance
   **both** rows or SDEX under-reports:
   - **Soroban backfill** → `soroban_amm.status='completed'` **and**
     `sdex_archive.current_ledger = activation` (≈ the recent chunk done,
     ~23% of the chain), `status='running'`, `last_push_at=now()`.
   - **SDEX backfill** → walks `sdex_archive` from `activation → 1`
     (≈23% → 100%).
   - **Between the two runs**, set `sdex_archive.status='paused'` so the
     §5.6 `last_push_at` freshness alarm does not false-fire.
   - Record `earliest_data_available` per stream (needs the task 0073
     column) for the `?timeframe=all` `backfill_note`.

7. **Split on the activation ledger, minute-aligned.** The two runs cover
   disjoint ledger ranges; align the split to a minute boundary so the one
   `source='sdex'` minute straddling `activation` is not partially
   double-written (`ReplacingMergeTree` replaces, not sums).

8. **Cloud-push shared with task 0028, not duplicated per stream.**
   The push mechanics (stream local `prices.*` rows to Hetzner over the 0052
   mTLS client, chunked INSERTs, flip `backfill_progress`) are common to
   SDEX and AMM; house them once.

### Recommended run order

**Globally forward: SDEX `[1, activation)` first is *not* required for
correctness, but the operator-chosen order sets what `/backfill/status`
shows during the run** (see the discussion captured in §Design). Two valid
orders:

- **Soroban-era first** (`[activation, tip]` combined) → `soroban_amm`
  completes and `sdex_archive` jumps to ~23% immediately; the pre-Soroban
  SDEX tail follows as a later milestone. Good for getting recent + AMM data
  first; **requires the dual-row update (decision 6) or SDEX under-reports.**
- **Chronological** (`[1, activation)` SDEX, then `[activation, tip]`
  combined) → `sdex_archive` advances monotonically `1→tip`; simplest honest
  progress, but you grind the long pre-Soroban tail before any recent/AMM
  data.

Either is acceptable; the milestone framing (Soroban era now, SDEX tail
next) uses Soroban-era-first.

## Implementation Plan

### Step 1: Full-range driver over the 0060 engine
Extend the `sdex-backfill` run loop to accept a mode flag: `combined`
(SDEX+AMM+oracle, for `[activation, tip]`) vs `sdex-only` (for
`[1, activation)`). Keep the single parse pass; gate AMM/oracle dispatch on
the mode. `--start`/`--end` already exist.

### Step 2: Forward-ordered decode + registry carry-forward
Ensure the `[activation, tip]` pass decodes in ledger order with the
venue/pool registries persisted across partitions (reuse the live
processor's mechanism). Add the "no swap for an unregistered pool" guard.

### Step 3: Discovered-registry artifact
Emit the accumulated pool registry as a durable artifact at run end (and/or
checkpoint) so partial re-runs and the live processor can load it.

> **Push model — direct-write (Model B, [ADR 0009](../../2-adrs/0009_backfill-direct-write-to-hetzner-clickhouse.md), 2026-07-01).** Steps 4–5 below
> supersede the earlier local-stage-then-push (task 0028). The backfill writes
> **directly to Hetzner `prices.*` over the 0052 mTLS client** — no local mirror,
> no separate push CLI, no `assets` remap (ids align via load-from-target). This
> mirrors BE's prod `backfill-runner --target clickhouse` and lets
> `/backfill/status` update in real time.

### Step 4: Real-time dual progress-row updates
Advance the two `backfill_progress` rows **as partitions complete** (Model B
writes to Hetzner live, so status is truthful in real time — not only after a
terminal push). Per decision 6: soroban run → `soroban_amm` advances forward +
`sdex_archive.current = activation`; sdex run → walks `sdex_archive`; `paused`
between runs; `earliest_data_available`; set `target_ledger = live tip` at start;
`last_push_at = now()` on each update.

### Step 5: Direct-write mTLS sink (ADR 0009; supersedes the 0028 push)
Give `sdex-backfill`'s `Sink` an mTLS-target constructor over the 0052 client
(reuse the live processor's `client_from_lambda_env` / `ClickHouseSink`,
`aws-mtls` feature). A flag/env selects local-plaintext (testing against a
stand-in) vs Hetzner-mTLS (real run). Load the asset registry from the target so
surrogate ids align with live (**no remap**). Chunked INSERTs + retry/backoff on
the sink; idempotent re-run (`ReplacingMergeTree(version)`); resume via
`backfill_sdex_ledgers` on Hetzner. No local CH mirror; local ledger scratch is
cleaned per-partition as today.

### Step 6: Tests
- Unit: bucketing + pre-roll math for ≥2 venue-pair scenarios; the
  unregistered-pool guard; the dual-row update logic.
- Integration: end-to-end against Docker local CH from a small recorded
  ledger fixture spanning the activation boundary; assert per-source rows +
  both progress rows + the minute-boundary seam.
- Idempotency: re-run push → `SELECT count() … FINAL` stable.

### Step 7: Operator runbook
Fold into `docs/runbooks/running-ingestion-components.md`: the two-range
sequence, run order + what `/backfill/status` shows, the activation split,
and teardown.

## Acceptance Criteria

- [ ] Combined single-pass run over `[activation, tip]` writes SDEX + AMM +
      oracle rows to the local `prices.*` mirror from a **single download per
      ledger** (no second pass for Soroban-era SDEX).
- [ ] SDEX-only run over `[1, activation)` completes the pre-Soroban SDEX
      tail; union coverage is `[1, tip]` with no ledger downloaded twice.
- [ ] Forward-ordered decode yields complete AMM pool discovery over the
      Soroban range with **no external registry seed**; the
      unregistered-pool guard never fires on a clean run (and is an error if
      it does).
- [ ] Discovered pool registry is emitted as a durable artifact.
- [ ] `push` streams local rows to Hetzner and updates `backfill_progress`;
      the **soroban run advances both** `soroban_amm`→`completed` and
      `sdex_archive`→`current=activation`; the **sdex run** walks
      `sdex_archive`→`completed`; `status='paused'` is set between runs.
- [ ] `GET /backfill/status` reports truthful, monotonic progress for both
      streams throughout (no SDEX under-report while recent SDEX exists).
- [ ] OHLCV for Soroswap pairs verifiable for Nov 2023 dates (Tranche 1 AC).
- [ ] Idempotent re-run of `push` (no duplicate rows after merge).
- [ ] Runbook updated.

## Blocked on

- **0037** — shared `SwapExtractor` trait + dispatch kernel + Phoenix pool
  registry (largely satisfied via 0060; confirm coverage).
- **0034** — Phoenix multi-WASM tolerance (or the PHO/USDC pair is dropped).
- **0048** decoder spec (archived) — consumed.
- **0052** — mTLS client for the cloud-push step (done; local decode runs
  without it).
- **0051 / 0063** — Hetzner `prices.*` schema + provisioned `prices` DB /
  mTLS before the push lands (both done).
- **~~0017~~** — **removed.** Its `soroban_events` role is superseded by the
  single-pass model; the local-CH infra already exists (docker-compose +
  the prices-clickhouse crate from 0060).

## Out of scope

- Live Soroban AMM / SDEX ingestion — the live processor (0038); this task
  is purely historical.
- The factory-registry *maintenance* service for live discovery — 0069
  (now an optimization, not a prerequisite here).
- Sub-table-granular resume of a partially-pushed run — v1 pushes all
  granularities or none, then idempotent re-run cleans up.

## Notes

- The local CH instance is torn down after the push (ADR 0001
  §Consequences); it is not a long-lived store.
- Decode + bucket math are shared with the live Ledger Processor (0038) —
  house both in the shared ingest crate so a fix in one path fixes the
  other. This is exactly why the single-pass backfill is byte-identical to
  live: same code, replayed from history.
- `backfill_progress.target_ledger` is set to the live tip when each run
  starts, so `progress_pct` is meaningful during the run.
