---
id: "0171"
title: "price_usd_series* publish Decimal128::MIN (-1.7e24) for any asset whose only priced candles carry zero volume — a non-Nullable CAST swallows the nullIf"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0165", "0116", "0144", "0151", "0150", "0061"]
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
- [ ] Neither grain of `price_usd_series*` can publish a non-positive
      `close_usd`; asserted by **value**, never by `IS NULL`.
- [ ] `usd_reference*` audited for the same shape and fixed or cleared on
      the record.
- [ ] Regression test on 26.3.10.60 covering the non-peg zero-volume case —
      the one 0165's peg guard deliberately does **not** reach.
- [ ] A prod count of how many `(identity, bucket)` rows are affected today,
      so the blast radius is known before changing behaviour.
- [ ] If any row is omitted, [[0150]] (materialising the series) is checked so
      the new predicate is not lost at materialisation time.

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
