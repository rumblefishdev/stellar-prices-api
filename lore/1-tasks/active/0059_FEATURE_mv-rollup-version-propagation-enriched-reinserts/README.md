---
id: "0059"
title: "MV rollup-chain version propagation under enriched `_1m` re-inserts"
type: FEATURE
status: active
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
  - date: 2026-06-26
    status: active
    who: oski
    note: >
      **Unblocked — 0051 completed (archived 2026-06-22).** The MV
      rollup-chain DDL landed in
      packages/prices-clickhouse/schema/rollups.sql (full _1m → _15m …
      _1M refreshable-MV chain) and was applied live on ch-prod-01
      (CH 26.3.10), so the SELECT / projected-version / refresh-window
      this task verifies now exists. Moving back to active to land the
      remaining ACs: the full _15m…_1M chain integration test +
      extending the proof harness against the real rollups.sql DDL.
  - date: 2026-06-26
    status: active
    who: oski
    note: >
      **Remaining ACs delivered + a real bug fixed.** Authored
      packages/prices-clickhouse/tests/rollup_chain_it.rs — a full-chain
      (_1m → _15m → … → _1M) integration test driving the REAL shipped
      rollups.sql, plus a preroll.sql full-range pass; both green vs docker
      CH 25.6. The test surfaced a correctness bug in the as-shipped
      rollups.sql AND preroll.sql: `toStartOfInterval(timestamp,…) AS
      timestamp` shadows the source column, so argMin(open)/argMax(close)/
      argMax(close_usd) tie-break to an arbitrary row (volumes/high/low were
      fine). Fixed both (FROM … AS t + qualified t.timestamp) and the
      schema-overview §3.2 reference DDL; current.sql was already correct.
      The buggy DDL is live on ch-prod-01 (applied under 0051) → spawned
      0071 to re-apply. All four ACs now [x]. Verified the shipped
      max(version) projection is correct for the true-refreshable replace-mode
      chain that 0051 actually shipped (differs from the G-note's APPEND/
      sum(version) lock-in).
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

- [x] MV chain projects a `version` that lets an enriched `_1m`
      re-insert win at every rolled-up granularity — **proven against the real
      0051 DDL** across the full `_1m → _15m → … → _1M` chain
      (`tests/rollup_chain_it.rs`): the shipped chain is a *true* refreshable MV
      in replace mode (atomic target swap), so `max(version)` is sufficient and
      the enriched row wins at every grain (version advances 1 → 2).
- [x] No double-count / under-count of `volume_quote_usd` in `_15m … _1M`
      after an enrichment pass — **proven on the full chain**: `volume_base`
      stays 30 (no double-count) and `volume_quote_usd` propagates 0 → 300 at
      every grain after the enrichment re-insert + refresh.
- [x] Integration test covering write → roll up → enrich → assert across
      all granularities (`FINAL`) — **`tests/rollup_chain_it.rs`** drives the
      real `rollups.sql` chain end-to-end (+ a `preroll.sql` full-range pass),
      green against docker CH 25.6.
- [x] 0026 G-note dependency note resolved / cross-linked

## Implementation Notes

Full-chain integration test landed at
`packages/prices-clickhouse/tests/rollup_chain_it.rs` (two `#[ignore]` tests,
scratch-DB isolated like `views_it.rs`, driven deterministically via
`SYSTEM REFRESH VIEW` + poll like `current_mv_it.rs`). It applies the **real**
shipped `INIT_SQL` + `ROLLUPS_SQL`, rolls a 3-minute bucket up all six grains,
then enrichment-re-inserts (`version+1`, `volume_quote_usd` filled) and
re-drives the chain, asserting OHLCV + version at every grain `FINAL`.

**Bug found and fixed (see Design Decisions → Emerged).** The as-shipped
`rollups.sql` / `preroll.sql` mis-computed `open`/`close`/`close_usd`. Fixed in
both schema files + the schema-overview §3.2 reference DDL. The buggy DDL is
live on `ch-prod-01` (applied under 0051) → re-apply spawned as **0071**.

## Design Decisions

### Emerged

1. **`AS timestamp` bucket alias shadowed the source column (correctness bug).**
   `toStartOfInterval(timestamp, …) AS timestamp` makes the bare `timestamp`
   inside `argMin(open, …)` / `argMax(close, …)` / `argMax(close_usd, …)`
   resolve to the *bucket-start* (constant within a bucket), so O/C/close_usd
   tie-break to an arbitrary row instead of the true first/last by time. Volumes
   (`sum`), `high`/`low` are unaffected. The 0059 desk proof only checked
   volumes, so it never surfaced this. **Fix:** `FROM … AS t` + qualified
   `t.timestamp` in `rollups.sql` and `preroll.sql`. `current.sql` was already
   correct (it uses `AS c` + `c.timestamp`).

2. **`max(version)` accepted (not `sum(version)`).** The G-note locked in
   "APPEND + `sum(version)`", but 0051 actually shipped a *true* refreshable MV
   in **replace** mode (atomic target swap) + bounded window, with `preroll.sql`
   as the separate full-range historical path. Under atomic replace there is no
   `ReplacingMergeTree` version tie to lose, so the shipped `max(version)` is
   correct — the integration test confirms the enriched row wins at every grain.
   The test asserts the shipped semantics rather than re-litigating the G-note.

3. **Pin the local/CI ClickHouse to the EXACT production version.** Code review
   flagged that the new integration test was being validated against
   `docker-compose.yml`'s `clickhouse-server:25.6` (`25.6.13.41`), while
   `ch-prod-01` runs **`26.3.10.60`** (per the 0051 G-live-schema-state note and
   0063 provisioning verification) — a 25.x → 26.x major gap. Refreshable-MV
   semantics, SQL alias resolution and date/time functions can all differ across
   CH releases, so "green locally" was not evidence for the engine the DDL
   actually runs on. **Decision:** pin `docker-compose.yml` to the exact prod
   version `clickhouse/clickhouse-server:26.3.10.60` and re-run the full
   `prices-clickhouse` suite against it (rollup_chain, current_mv, views — all
   green). Policy going forward: local/CI must match the live `ch-prod-01`
   version; re-check `SELECT version()` on prod before bumping the pin. (This
   strengthens — but does not replace — the live re-verify owed by **0071** on
   the production cluster itself.)

4. **Flaky test anchor made deterministic + version-robust.** The first cut of
   `insert_bucket` embedded `toStartOfInterval(now(), …)`, re-evaluated
   server-side at *each* INSERT; the un-enriched and enriched batches are
   separate INSERTs with the whole chain-drive between them, so a wall-clock
   15-minute boundary crossing would anchor them to different buckets → no
   `ReplacingMergeTree` dedup → `_1m FINAL` keeps all 6 rows and the
   single-bucket / no-double-count assertions fail. **Fix:** `bucket_anchor()`
   fetches the boundary **once** as a `toUInt64(toUnixTimestamp(…))` integer
   epoch and rebuilds it as `toDateTime(<n>)`, reused for every INSERT. The epoch
   round-trip (not `formatDateTime`, whose `%i`/`%M` specifiers are a portability
   liability across CH versions) keeps it correct regardless of server version or
   timezone.

## Issues Encountered

- **MV → target column mapping is positional, not by-name** (verified with a
  swapped-alias `INSERT … SELECT` probe), so qualifying the source column while
  keeping the output column aliased `timestamp` is safe.
- `prefer_column_name_to_alias=1` is **not** a viable fix — it makes `GROUP BY
  timestamp` bind to the raw column, shattering the bucket into per-minute rows.

## Future Work

- **0071** (spawned) — re-apply the corrected `rollups.sql` / `preroll.sql` to
  the live `ch-prod-01` cluster (the buggy DDL is already deployed under 0051).
