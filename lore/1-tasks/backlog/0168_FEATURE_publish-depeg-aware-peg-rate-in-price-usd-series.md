---
id: "0168"
title: "Publish the real peg rate in price_usd_series instead of a hardcoded $1"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0165", "0167", "0154", "0151", "0150"]
tags:
  ["priority-medium", "effort-small", "clickhouse", "read-surface", "be-interop", "data-correctness", "milestone-M2"]
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-07
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0165]]. 0165 ships a peg-fill arm at a flat `$1` because it
      is the only value available without new infrastructure. That is a ~0.1%
      systematic error on every row — small USDC depegs (0.999/1.001) are routine
      across chains, not a crisis event — and it contradicts our own candles,
      which the oracle tier already prices depeg-aware. Needs [[0167]]'s
      `prices.usd_rate`. Scoped as its own task rather than a caveat in 0165's
      view header, so the header can point at an ID instead of a regret.
---

# Publish the real peg rate, not `$1`

## Summary

[[0165]] gives `price_usd_series` a peg-fill arm so USDC becomes publishable at
all. That arm emits a **constant `$1`**. This task swaps the constant for the
measured rate from [[0167]]'s `prices.usd_rate`, falling back to `$1` only where
no observation exists.

## Context

USDC does not sit at exactly `$1`. It trades at 0.999–1.001 as a matter of
routine, so a hardcoded `1` is not a rare-depeg approximation — it is a **~0.1%
systematic error on every published row, permanently**.

Worse, it is inconsistent with our own data. The enrichment oracle tier is
depeg-aware and *"wins where it applies"* (`ch_enrich.rs:20`), setting a
`TF/USDC` candle's `close_usd` from the Reflector USDC rate. So in the oracle
window our candles already price USDC at 0.9993 while this view would publish
`USDC = 1.0000` for the same bucket. Two of our own surfaces disagreeing is a
defect a consumer will eventually file back at us.

The rate is **already collected** — `oracle_worker` polls it
(`oracle-worker/src/lib.rs:33`) — it is simply never published.

## Why this is a separate task and not part of 0165

- 0165 unblocks **1,433 pools that have no price at all** (67.8% of every
  never-priced pool BE holds). Flat `$1` is 0.1% wrong; absent is 100% wrong.
  Holding that behind new infrastructure is the wrong trade.
- The rate table ([[0167]]) is real work with its own verification gate.
- **The view's shape does not change**, so this is a genuine refinement rather
  than rework — see below.

## Implementation

The entire change is one expression. 0165's peg arm becomes a `LEFT JOIN` onto
`prices.usd_rate` (resolved by `ASOF` at the bucket's end per [[0167]]'s rule),
carrying `peg_rate = coalesce(r.usd_rate, 1)`; arm A carries `peg_rate = 0`.

```sql
if(max(is_peg) = 1 AND sum(w) = 0,
   CAST(max(peg_rate) AS Decimal(38, 14)),        -- was: CAST(1 AS Decimal(38, 14))
   CAST(sum(v) / nullIf(sum(w), 0) AS Decimal(38, 14)))
```

`max()` picks arm B's value because every arm-A row carries `0`.

Same key, same column, same `Nullable(Decimal(38,14))` type, same purely-additive
property. No schema migration, no consumer change, BE integrates once.

## Three things 0165 must do so this stays a one-line change

Fold these into 0165 **before it merges**, or this task turns into a rewrite:

1. **Write the regression test against fallback semantics, not against `$1`.**
   Assert *no rate row → `$1`* and *rate row present → that rate*, with the
   second case simply unseeded for now. A test asserting "peg asset → exactly
   1.0" has to be rewritten here.
2. **Carry the provenance discriminator from day one.** Once both paths exist, a
   consumer cannot tell `1.0000` (a real oracle reading) from `1.0000` (the
   fallback). That is the `close_usd = 0` mistake — one value meaning several
   things — and it is far cheaper to ship the column than to retrofit it. Use
   [[0167]]'s `method`/`hops` vocabulary, do not invent a second.
3. **Point the view header comment at this task ID**, not at a prose caveat.

## Acceptance Criteria

- [ ] `price_usd_series` and `price_usd_series_1h` publish the measured peg rate
      where an observation exists within the staleness window.
- [ ] `$1` remains the fallback where no observation exists (deep history,
      pre-~2025-09), and is **distinguishable** from a measured `1.0000`.
- [ ] No `NULL` introduced — BE: *"a NULL renders as a dash and removes the pool
      from every USD view we have."*
- [ ] The published value agrees with what the oracle tier baked into candles in
      the same bucket — the internal-consistency check that motivates this task.
- [ ] Applies to every peg asset (USDT too), not a USDC special case.
- [ ] Visible in [[0150]] if that materialises the view, or the fix is lost at
      materialisation time.

## Out of scope

- Building the rate table or populating it — [[0167]].
- `current_price_usd` / `current_prices`, which is suspected to carry the same
  base-only assumption as 0165 but is a refreshable-MV rebuild and must not ride
  along on a view swap.

## Notes

- Deep history stays flat `$1` permanently — there is no oracle before ~2025-09.
  That is a data-availability fact, not a gap to close, and the same shape as the
  pre-Soroban tail having no USD reference at all.
