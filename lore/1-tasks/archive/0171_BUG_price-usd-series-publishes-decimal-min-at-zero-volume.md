---
id: "0171"
title: "price_usd_series* publish Decimal128::MIN (-1.7e24) for any asset whose only priced candles carry zero volume — a non-Nullable CAST swallows the nullIf"
type: BUG
status: completed
assignee: akot
related_adr: []
related_tasks: ["0165", "0116", "0144", "0151", "0150", "0061", "0198"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "read-surface", "be-interop", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-10
    status: backlog
    who: okarcz
    note: >
      Found during code review of [[0165]] (PR #188). The review flagged the
      zero-volume case as a defect in 0165's guard; investigating it showed the
      guard choice was indeed wrong (fixed in that PR) but that a larger,
      PRE-EXISTING defect sits underneath and is not peg-specific. Splitting it
      out rather than widening 0165, because the fix requires a contract
      decision with BE, not a guard swap.
  - date: 2026-08-11
    status: backlog
    who: okarcz
    note: >
      UNBLOCKED - BE gave the contract decision this task was split out of 0165
      to obtain: OMIT THE ROW (option 1). Their reasoning: "misses are absent" is
      what their entire read path assumes (argMax over present rows, NULL when
      nothing matches), and a published sentinel forces every consumer to know a
      magic constant forever. That reasoning rejects option 2 as well as option
      3 - a silently substituted different statistic is the same class of
      problem. They are adding a close_usd > 0 guard on their side regardless,
      with ZERO occurrences in their read windows today, so this is insurance on
      both sides rather than a live incident. Implement HAVING sum(w) > 0 on both
      grains; the row-count change is now the riskiest part, not the omission.
  - date: "2026-09-14"
    status: active
    who: akot
    note: >
      Activated; taken by akot together with [[0198]], one branch and one PR
      for both. They are the same expression in views.sql (still at :377 and
      :666 on develop) and disagree on its failure mode: this task says it
      publishes Decimal128::MIN, 0198 measured an exception (code 349) on the
      prod pin. Settling which one is first.
  - date: "2026-09-14"
    status: completed
    who: akot
    note: >
      Fixed on PR #312 together with [[0198]]. Arm A of both series grains and
      both usd_reference grains now admit a candle only with volume_base > 0,
      so a zero-weight group never forms and the CAST never sees NULL. Settled
      the 0171-vs-0198 disagreement: BOTH are right on 26.3.10.60 — the
      interpreted CAST raises code 349, the JIT-compiled one (default, after
      3 executions) publishes Decimal128::MIN. Prod blast radius measured at
      ZERO zero-volume priced candles on either grain, so no row changes today.
      3 new #[ignore] tests (both grains, both JIT modes, red on develop /
      green on the fix) + 1 unit test pinning the predicate on all 4
      statements; 16 ignored + 36 unit tests pass on 26.3.10.60.
---

# `price_usd_series*` publish `Decimal128::MIN` at zero volume

## Summary

Both grains compute

```sql
CAST(sum(v) / nullIf(sum(w), 0) AS Decimal(38, 14)) AS close_usd
```

`nullIf` makes the expression `Nullable(Float64)` when total volume is zero —
but **`CAST` to `Decimal(38,14)` strips the Nullable**. The row does not go
absent and does not read NULL; it publishes

```
-1701411834604692317316873.03715884105728
```

i.e. `Decimal128::MIN`, ≈ **-1.7 × 10²⁴**, in the column BE multiplies into TVL.

## Verified on the prod pin (26.3.10.60)

```sql
SELECT toTypeName(sum(v) / nullIf(sum(w), 0))                        AS raw_type,
       toTypeName(CAST(sum(v) / nullIf(sum(w), 0) AS Decimal(38,14))) AS cast_type,
       CAST(sum(v) / nullIf(sum(w), 0) AS Decimal(38,14))             AS value,
       CAST(sum(v) / nullIf(sum(w), 0) AS Decimal(38,14)) IS NULL     AS is_null
FROM (SELECT toFloat64(0) AS v, toFloat64(0) AS w);
```

```
raw_type:  Nullable(Float64)
cast_type: Decimal(38, 14)          -- NOT Nullable — this is the bug
value:     -1701411834604692317316873.03715884105728
is_null:   0
```

**Pre-existing and unrelated to [[0165]].** The historical view body carried the
identical expression; 0165 only changed which rows reach it.

## Trigger

Any `(identity, bucket)` whose priced candles (`close_usd > 0`) all carry
`volume_base = 0`. Not hypothetical: [[0116]] documents dust-trade candles with
negligible volume, and zero-volume priced rows are exactly the shape a
single-trade or corrected candle can take.

**Not peg-specific.** [[0165]]'s peg fallback rescues the subset where the asset
is also a **quote** leg in that bucket (so `max(is_peg) = 1` and the `sum(w) = 0`
guard fires). Everything else — every non-peg asset, and peg assets appearing
only as a zero-volume **base** — still publishes the garbage. 0165's view comment
records this residual explicitly.

## Why it is worse than a wrong number

- **It is negative.** Any consumer summing USD value gets a result dominated by
  one row, with the sign flipped.
- **It is labelled as measured.** After 0165 the row carries
  `method = 'traded'`, because the weighted average nominally "succeeded".
- **It is invisible to the obvious check.** `countIf(close_usd IS NULL)` is
  structurally always `0` on a non-Nullable column — the exact vacuous assertion
  0165 shipped in its first draft. Any guard for this must test the **value**.
- Same family as [[0144]]/[[0151]]: a sentinel that is indistinguishable from a
  real reading, in a surface BE consumes directly.

## ✅ DECIDED 2026-08-11 — BE chose **option 1, omit the row**

> *"Omit the row. **'Misses are absent' is the contract our whole read path
> assumes** — argMax over present rows, NULL when nothing matches. A published
> sentinel forces every consumer to know a magic constant forever, and this
> thread is the proof nobody reads release notes in time."*
> — BE, answering on the [[0165]] re-measurement thread

**This task is no longer blocked**; the contract decision it was split out of
[[0165]] to obtain has been given. Implement `HAVING sum(w) > 0` (or the
equivalent) on both grains.

Two things they added that shape the work:

- **They are adding a `close_usd > 0` guard on their side regardless**, and
  measured **zero occurrences in their read windows today**. So this is
  insurance on both sides rather than a live incident — which sets the urgency,
  not the correctness bar. The row count change (⚠️ below) is therefore the
  riskiest part of the change, not the omission itself.
- Their reasoning explicitly rejects option 2 as well as option 3: a *silently
  substituted different statistic* is the same class of problem as a sentinel —
  a consumer has to know something extra, forever, that no release note reaches
  them in time to learn.

The original analysis is kept below, since the options and their trade-offs are
still what the implementation has to honour.

## The decision this needed (do not just patch the symptom)

Three candidate behaviours; **the choice was a contract change and needed BE**
— ✅ now answered above, **option 1**:

1. **Omit the row.** Most consistent with the documented contract — *"a miss is
   a missing row … never an error and never a dropped row"* — and with §12.3,
   which already classifies an absent row as `no_asset_price`. A `HAVING
   sum(w) > 0` does it. ⚠️ Changes row counts for existing consumers.
2. **Fall back to an unweighted aggregate** (e.g. `argMax(close_usd, timestamp)`)
   when total weight is zero. Keeps the row and the price is real, but it is a
   *different statistic* silently substituted — needs a `method` value of its
   own (`'unweighted'`), which 0165's provenance column now makes expressible.
3. **Publish 0.** Cheapest, and **rejected** — it re-creates the
   `close_usd = 0` ambiguity that [[0144]] and the whole 0145/0146/0147 cluster
   exist to unwind.

Option 1 is the recommendation; option 2 is defensible if BE would rather have a
number than a gap.

## Scope

- Both `price_usd_series` and `price_usd_series_1h`.
- **Audit `usd_reference` / `usd_reference_1h` too** — `views.sql:207,328` use
  the same `CAST(… / nullIf(…, 0) AS Decimal(38,14))` shape for `xlm_usd`. A
  garbage XLM reference would be worse than a garbage asset price, because
  §12.3 makes every pivot-priced asset depend on it. **Not yet checked.**
- Grep the rest of the schema for `nullIf(` inside a `CAST(... AS Decimal`.

## Acceptance Criteria

- [x] Decision recorded (omit / unweighted / other) with BE's input, and why.
      ✅ **2026-08-11 — OMIT THE ROW**, quoted verbatim above with their
      reasoning ("misses are absent" is what their whole read path assumes).
- [x] Neither grain of `price_usd_series*` can publish a non-positive
      `close_usd`; asserted by **value**, never by `IS NULL`.
      ✅ `a_zero_volume_only_base_is_absent_and_its_neighbours_still_publish`
      asserts `countIf(toFloat64(close_usd) <= 0) = 0` on both grains, in
      both JIT modes.
- [x] `usd_reference*` audited for the same shape and fixed or cleared on
      the record. ✅ Same shape, same failure, **fixed** (`volume_base > 0`
      beside `close > 0`), pinned by
      `usd_reference_omits_a_bucket_whose_reference_candles_have_no_volume`.
      The rest of the schema grepped. `current.sql` wraps its divisions in
      `ifNull`. **Correction from PR #312's review (finding 1):** the `vwap`
      of the six rollup MVs and the `preroll*.sql` sites,
      `volume_quote / nullIf(volume_base, 0)`, is `Nullable(Decimal(38,14))`
      written into the non-Nullable `vwap` column — the same Nullable →
      non-Nullable shape, on the WRITE path. Checked, and it cannot fire the
      way the views did: on an `INSERT` (plain `INSERT SELECT` and a
      refreshable `APPEND` MV both measured on 26.3.10.60)
      `insert_null_as_default = 1` — the default, and prod's value — turns
      the NULL into the column default `0`, which is exactly what ingest's
      `finalise_vwap` leaves in a zero-volume minute. Only
      `insert_null_as_default = 0` raises code 349. Prod on 2026-09-14 has
      **zero** `volume_base = 0` rows on `_1m`, `_15m` and `_1h`, and every
      rollup MV is `Scheduled` with no exception. Left as is here — an edit to
      `rollups.sql` does not land without DROP + re-CREATE — and carried into
      [[0146]] as an explicit `ifNull(…, 0)` for when the MVs are re-created.
- [x] Regression test on 26.3.10.60 covering the non-peg zero-volume case —
      the one 0165's peg guard deliberately does **not** reach.
      ✅ Red on `develop`'s `views.sql` (sentinel row present / code 349
      raised, per JIT mode), green on the fix.
- [x] A prod count of how many `(identity, bucket)` rows are affected today,
      so the blast radius is known before changing behaviour.
      ✅ **Zero** — see Implementation Notes. No row changes on deploy.
- [x] If any row is omitted, [[0150]] (materialising the series) is checked so
      the new predicate is not lost at materialisation time.
      ✅ No row is omitted today, but 0150 now carries the predicate as a
      settled precondition (its §4) and an acceptance criterion.

## Notes

- The prod count query (run before deciding — it may be zero, which would make
  this cheap):
  ```sql
  SELECT count() FROM prices.price_usd_series WHERE toFloat64(close_usd) <= 0;
  ```
- ⚠️ Do not "fix" this by removing the `nullIf`. Plain division by zero in
  ClickHouse yields `inf`/`nan`, which casts to Decimal just as badly.
- 0165 ships `countIf(toFloat64(close_usd) <= 0) = 0` assertions in `views_it.rs`
  for the fixtures it covers; extend that pattern rather than inventing another.

## Implementation Notes

PR #312, branch `fix/0171_price-usd-series-publishes-decimal-min-at-zero-volume`,
one PR for this task and [[0198]]. Three files:

- `packages/prices-clickhouse/schema/views.sql` — arm A of `price_usd_series`
  and `price_usd_series_1h` reads `WHERE p.close_usd > 0 AND p.volume_base > 0`;
  `usd_reference` and `usd_reference_1h` read
  `AND p.close > 0 AND p.volume_base > 0`. Header comments rewritten: the
  "known residual, deliberately NOT fixed" paragraph from 0165 is now the
  record of how it was closed, including the JIT finding below.
- `packages/prices-clickhouse/src/lib.rs` — unit test
  `views_sql_every_weighted_surface_admits_only_candles_with_volume` pins the
  predicate on all four statements, so a fix that reaches one grain fails CI.
- `packages/prices-clickhouse/tests/views_it.rs` — three `#[ignore]` tests
  (need ClickHouse 26.3.10.60; run with `cargo test -p prices-clickhouse --test
  views_it -- --ignored`):
  - `a_zero_volume_only_base_is_absent_and_its_neighbours_still_publish` —
    the non-peg case at both grains, in both JIT modes; also asserts FOO and
    USDC in the same query keep their values (the availability half of 0198).
  - `a_zero_volume_candle_beside_a_real_one_changes_nothing` — a zero-volume
    print at a different price next to a real one moves nothing, so the fix is
    exactly the omission of the un-computable group.
  - `usd_reference_omits_a_bucket_whose_reference_candles_have_no_volume` —
    the audit item, both grains, both JIT modes.

**Verification (2026-09-14, rootless 26.3.10.60 in the scratchpad):**
red on `develop`'s `views.sql` — `a_zero_volume_only_base…` and
`usd_reference_omits…` fail, with `Code: 349` interpreted and the sentinel row
compiled; green on the fix — 16/16 ignored `views_it` tests and 36/36 unit
tests pass; `cargo fmt --check` clean.

**Prod blast radius (2026-09-14, `dev_read` over mTLS):**

| Query | 1d | 1h |
|-------|----|----|
| candles with `close_usd > 0 AND volume_base = 0` | 0 | 0 |
| `(asset_id, timestamp)` groups whose priced candles all have zero volume | 0 | n/a (memory limit; implied 0 by the row above) |
| XLM/USDC reference candles with `close > 0 AND volume_base = 0` | 0 | 0 |

The view-level `countIf(toFloat64(close_usd) <= 0)` on prod was not run: with
zero trigger candles the arm A population is unchanged, and arm B publishes
`1`. Prod runs `compile_expressions = 1`, `min_count_to_compile_expression = 3`.

**Why the fix is `WHERE volume_base > 0` and not `HAVING sum(w) > 0`:** the
task suggested `HAVING`; the `WHERE` form is equivalent for the group (a
zero-volume candle contributes `v = 0, w = 0` to a weighted mean, so dropping
it changes no value) and keeps the guard on the row the view admits, where the
`close_usd > 0` guard already lives and where the unit test can read it.

## Issues Encountered

- **0171 and 0198 disagreed on the failure mode, and both were right.** On
  26.3.10.60 the outcome of `CAST(NULL AS Decimal(38, 14))` depends on the
  expression JIT. `SETTINGS compile_expressions = 0` — or a cold server before
  the expression has been executed `min_count_to_compile_expression` (3)
  times — raises `CANNOT_INSERT_NULL_IN_ORDINARY_COLUMN` (code 349) and fails
  the whole query. Once compiled, the same CAST publishes `Decimal128::MIN`.
  Found because the very first red run on a fresh server raised 349 and every
  later one published the sentinel; bisected with curl over
  `compress`/format/GET-vs-POST/analyzer/threads before `compile_expressions`
  explained all of it. Consequence for prod: right after a restart the read
  surface raises, once warm it lies — which is why BE's zero-count and 0198's
  measurement could both hold.
- **`dev_read` is a readonly user** and rejects `max_execution_time` in the
  URL; the 1h group count then hit the 3.73 GiB memory cap. The candle-level
  count answers the same question, so it was not retried.
- **The first draft's comments said 349 "was not reproducible".** Wrong;
  corrected in `views.sql` and `views_it.rs` before commit.
- **The first shipped `JIT_MODES` did not deterministically reach the compiled
  path** (PR #312 review, finding 3). Its second entry was the server default,
  and on a cold server the default stays interpreted for the first three
  executions — measured: executions 2 and 3 raised 349, the sentinel only
  appeared from the fourth. The red run had seen the sentinel because earlier
  tests had warmed the expression. Fixed by forcing the mode:
  `SETTINGS compile_expressions = 1, min_count_to_compile_expression = 0`,
  which compiles on the first execution (measured: sentinel on execution 1 of
  a fresh server).
- **`usd_reference*` now carries a contract change worth telling BE** (review,
  non-finding): a bucket whose only XLM/USDC candles carry zero volume reads
  as `no_reference` (systemic blackout) rather than `no_asset_price`.
  Documented in §3.2 of `docs/database-schema/database-schema-overview.md`;
  zero such buckets on prod today.

## Design Decisions

### From Plan

1. **Omit the row (option 1).** BE's 2026-08-11 decision, quoted above. A
   zero-weight `(identity, bucket)` is absent from both grains; §12.3 already
   classifies an absent row as `no_asset_price`.
2. **Assert by value, never by `IS NULL`.** The column is non-Nullable, so
   `IS NULL` is vacuously false. Every test uses
   `countIf(toFloat64(close_usd) <= 0) = 0` or an exact row list.
3. **`usd_reference*` gets the same fix**, not just an audit note: a garbage
   XLM reference poisons every pivot-priced asset.

### Emerged

4. **`WHERE p.volume_base > 0` on arm A instead of `HAVING sum(w) > 0`.**
   Same rows, and the predicate sits next to `close_usd > 0` where the
   `lib.rs` test can pin it textually. Arm B (the peg placeholder, `w = 0` by
   construction) is untouched, so 0165's fallback still fires.
5. **Regression tests run each read twice, interpreted first.** Without the
   `compile_expressions = 0` pass the red run only shows 349 on a cold server,
   which no CI can promise. Interpreted goes first because that is the
   whole-query failure.
6. **No new task for the JIT behaviour itself.** It is a ClickHouse property,
   not ours; the record lives in the `views.sql` header and here. The only
   defence is never handing the CAST a NULL, which is what the fix does.

**Broken/modified tests:**
- `peg_asset_with_only_zero_volume_candles_falls_back_instead_of_publishing_garbage`
  — doc comment only. It said fixture A "RAISES code 349 rather than
  publishing Decimal128::MIN"; now says both happen and which mode gives
  which. The assertions are unchanged and the test still passes: with the
  zero-volume arm-A row gone, USDC's group is the placeholder alone and the
  0165 fallback still fires.

## Future Work

- [[0150]] carries the predicate as §4 and an acceptance criterion; nothing
  else spawned.
