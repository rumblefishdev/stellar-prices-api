---
id: "0059"
title: "MV rollup-chain version propagation under enriched `_1m` re-inserts"
type: FEATURE
status: blocked
related_adr: ["0007"]
related_tasks: ["0026", "0051"]
tags: [layer-database, priority-high, effort-medium, clickhouse, materialized-views, rollups]
links:
  - "../../../docs/database-schema/database-schema-overview.md"
  - "../blocked/0026_FEATURE_volume-quote-usd-enrichment-impl/notes/G-local-prototype-spec.md"
  - "./0051_FEATURE_clickhouse-prices-schema-and-mv-chain-migration.md"
history:
  - date: 2026-06-09
    status: backlog
    who: claude
    note: >
      Spawned from 0026 future work. The 0026 enrichment Lambda
      corrects volume_quote_usd by re-INSERTing _1m rows with
      version+1. That re-insert re-fires the MV rollup chain, raising
      open questions about how _15m..._1M re-aggregate and what version
      the MVs project onto their ReplacingMergeTree targets. 0026
      scoped itself to _1m only and flagged this as a 0051 dependency.
  - date: 2026-06-09
    status: active
    who: okarcz
    note: >
      Promoted from backlog to active. Fixed the stale 0026 link
      (active -> blocked). Note: real progress is gated on 0051
      landing the MV rollup-chain DDL — the SELECT/GROUP BY/projected
      version this task verifies does not exist yet.
  - date: 2026-06-09
    status: active
    who: okarcz
    note: >
      Converted to directory form. Authored the decision + proof-plan
      G-note (notes/G-rollup-version-propagation-decision.md) — the
      0051-independent slice. Key finding: the draft insert-trigger MV
      DDL (schema-overview §3.2) under-counts multi-block buckets even
      before enrichment; recommends re-aggregate-from-_1m-FINAL
      (Refreshable MV) rollups. Proof execution against a local docker
      CH is the next step (deferred). 0051 contract + ADR-0007
      amendment flagged.
  - date: 2026-06-09
    status: active
    who: okarcz
    note: >
      Proof EXECUTED against clickhouse-server 24.8.14 (proof/ harness +
      run.sh + RESULTS.md). Confirmed the draft under-counts 150->10 and
      enrichment does not propagate; re-aggregate-from-_1m-FINAL yields
      the correct 150/500. Two new findings: draft DDL does not compile
      (alias collision), and max(version) is an insufficient rollup
      version projection. Added a superseded-warning callout to schema
      doc §3.2. G-note promoted to mature. Remaining work (full _15m.._1M
      chain test, real DDL) still gated on 0051.
  - date: 2026-06-10
    status: active
    who: okarcz
    note: >
      Durability correction (doc-grounded). Verified against the ClickHouse
      CREATE VIEW reference + Refreshable MV guide that the default refresh
      "atomically replaces the table's previous contents" — so replace-mode
      plus a bounded window would destroy rollup history (and empty the
      rollup if _1m is cleared). Durable rollups must use APPEND, which keeps
      them on ReplacingMergeTree dedup → strictly-increasing version
      (sum(version)/epoch) is ALWAYS required. Recorded both doc citations
      and a new §3.1 in the G-note; added APPEND-not-replace + _1m-retention
      items to the 0051 contract.
  - date: 2026-06-10
    status: blocked
    who: okarcz
    by: ["0051"]
    note: >
      Design slice complete and merged to develop (PR #37, merge commit
      fde4bb4): corrected rollup pattern in schema-overview §3.2, executed
      proof (proof/ + RESULTS.md), doc-grounded durability correction, the
      A-vs-A′ comparison, and the LOCKED-IN decision — Refreshable MV in
      APPEND mode re-aggregating from _1m FINAL (Hetzner CH 26.3.10.60
      confirmed; external-worker fallback dropped, no ADR-0007 amendment
      needed). Moving to blocked: the remaining ACs (full _15m…_1M chain
      integration test + production DDL) are gated on 0051 landing the MV
      rollup-chain DDL — the SELECT/version/refresh-window this task
      verifies does not exist until then.
---

# MV rollup-chain version propagation under enriched `_1m` re-inserts

## Summary

The 0026 enrichment Lambda re-INSERTs corrected `price_ohlcv_1m` rows
with `version = original + 1`. Each such INSERT re-fires the MV rollup
chain (`_1m → _15m → … → _1M`, which `sum()`s `volume_quote_usd`). This
task verifies — and fixes if needed — that the rolled-up granularities
end up with the *enriched* values rather than the stale `0`-contribution
rows, and that the MV-projected `version` makes the corrected rollup row
win its `ReplacingMergeTree` merge.

## Context

Task 0026 deliberately enriches `_1m` only and left rollup correctness to
0051 (which owns the MV chain DDL). The open risks:

- A summing MV fires on the *inserted block* (the single re-inserted
  `_1m` row), not a re-read of the whole bucket — so it may emit a
  partial-sum `_15m` row that needs to combine with, not replace, the
  existing one. On a `ReplacingMergeTree` target this can double-count
  or under-count depending on the MV's `version` projection.
- What `version` the MV assigns to its target rows determines whether
  the corrected rollup row wins over the original `0`-contribution row.

## Decision (2026-06-09) — see [G-note](notes/G-rollup-version-propagation-decision.md)

Design + proof plan authored (no live CH yet). Headline findings:

- The draft insert-trigger MV in schema-overview §3.2 is **incorrect even
  without enrichment**: a CH MV aggregates only the *inserted block*, so a
  15-minute bucket fed by ~15 per-minute INSERTs emits 15 partial `_15m`
  rows; the `ReplacingMergeTree` target keeps just one (`max(version)`) →
  **~1/15 under-count**. Enrichment re-inserts then mis-/double-count on top.
- Root cause: incremental combine-on-insert and correction-by-re-insert are
  mutually exclusive under a plain insert-trigger MV.
- **Recommended:** rollups **re-aggregate from `_1m FINAL`** via a
  **Refreshable MV** (CH ≥ 23.12; scheduled `INSERT…SELECT…FROM _1m FINAL`
  fallback). Correct on both failure modes, keeps rollups inside CH (honours
  ADR 0007 §3.4), keeps identical-shape `ReplacingMergeTree(version)` targets,
  and gives the more-accurate Σ-of-per-minute `volume_quote_usd`.
- This supersedes the draft DDL → **0051 must not ship it as written**, and an
  ADR-0007 amendment should record the refreshable-rollup choice.

**Proof executed** (2026-06-09, `clickhouse-server 24.8.14` — see
[`proof/`](proof/) + [`proof/RESULTS.md`](proof/RESULTS.md), `proof/run.sh`
reproduces). Confirmed all predictions and surfaced two more: the draft DDL
**does not compile** (alias collision → `ILLEGAL_AGGREGATION`), and
`max(version)` is an **insufficient rollup version projection** (ties pre/post
early-minute enrichment; `sum(version)` strictly increases). Observed:
`volume_base` 150→10 under the draft; `volume_quote_usd` stays 0 after enrich;
re-aggregate-from-`_1m FINAL` yields the correct 150 / 500. The schema doc §3.2
was **rewritten to the corrected refreshable / re-aggregate pattern** (and the
insert-trigger phrasings elsewhere in the doc updated to match).

**Durability correction (2026-06-10, doc-grounded — G-note §3.1).** The default
refreshable MV *"atomically replaces the table's previous contents"*, so
replace-mode + a bounded window would hold only the window and empty the rollup
if `_1m` is cleared. Durable rollups require **`APPEND`** (*"inserts rows …
without deleting existing rows"*) — which keeps them on `ReplacingMergeTree`
dedup, so the strictly-increasing version projection (`sum(version)`/epoch) is
**always** required, not just in the scheduled fallback. Added to the 0051
contract; also implies `_1m` retention ≥ widest rollup refresh window.

## Implementation

- Pin down the exact MV DDL (`SELECT` shape, `GROUP BY`, projected
  `version`) for each step of the chain in task 0051.
- Decide the correct engine/semantics for rollup targets so an enriched
  `_1m` re-insert propagates correctly (candidates: project
  `max(version)`/`maxState`, or restructure as `AggregatingMergeTree`).
- Add an integration test: write a `_1m` row at `volume_quote_usd = 0`,
  let the chain roll up, run 0026 enrichment, and assert every
  granularity reflects the enriched value after `FINAL`.

## Acceptance Criteria

- [~] MV chain projects a `version` that lets an enriched `_1m`
      re-insert win at every rolled-up granularity — **semantics decided +
      proven** (`sum(version)`/refresh-epoch, not `max(version)`); production
      DDL lands in 0051
- [~] No double-count / under-count of `volume_quote_usd` in `_15m … _1M`
      after an enrichment pass — **proven on `_1m → _15m`** (re-aggregate from
      `_1m FINAL`); full chain `_15m … _1M` verified once 0051 lands the DDL
- [~] Integration test covering write → roll up → enrich → assert across
      all granularities (`FINAL`) — **proof harness exists** (`proof/`, 1 hop);
      extend to all grains against the real 0051 DDL
- [x] 0026 G-note dependency note resolved / cross-linked
