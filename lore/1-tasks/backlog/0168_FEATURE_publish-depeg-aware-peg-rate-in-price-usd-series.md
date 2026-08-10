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
      pre-**2026-03-11**), and is **distinguishable** from a measured `1.0000`.
- [ ] No `NULL` introduced — BE: *"a NULL renders as a dash and removes the pool
      from every USD view we have."*
- [ ] The published value agrees with what the oracle tier baked into candles in
      the same bucket — the internal-consistency check that motivates this task.
- [ ] Applies to every peg asset (USDT too), not a USDC special case.
- [ ] Visible in [[0150]] if that materialises the view, or the fix is lost at
      materialisation time.

## ⚠️ Known adjacent gap this task does NOT close — the enrichment peg tier

**Measured on prod 2026-08-10**, from live `oracle_prices` readings:

| | rate | off par |
|---|---|---|
| USDC | 1.00066784838102 | **+0.067%** |
| USDT | 0.99930223861292 | **−0.070%** |

Three properties, each of which strengthens the case for this task:

1. **The ~0.1% figure is real**, ~0.07% per asset. Not hypothetical, not a
   depeg event — this is an ordinary Sunday afternoon.
2. **The two deviate in OPPOSITE directions**, so the spread *between* them is
   **~0.137%**. A flat `$1` is therefore not a small uniform offset that mostly
   cancels; anything comparing a USDC-denominated value against a
   USDT-denominated one carries the whole 0.14%.
3. **It is a persistent bias, not noise.** Five consecutive 5-minute readings
   held the same sign and magnitude to four decimal places. Jitter around par
   would average out across many candles; a stable offset does not — it is
   present on *every* row, always in the same direction. That is a stronger
   argument than a depeg would be: a depeg is rare and visible, this is
   permanent and invisible.

**The gap:** this task fixes the *view's* peg fallback. The enrichment **peg
tier** bakes the same flat `$1` into `close_usd` itself —

> a USDC- or USDT-quoted candle gets `close_usd = close × $1`, exact and
> oracle-free, back to SDEX genesis  (`ch_enrich.rs`)

— so every USDC-quoted candle's `close_usd` is ~0.067% **low** and every
USDT-quoted one ~0.070% **high**, wherever the oracle tier did not win. That is
all deep history before the oracle window (**2026-03-11**, measured) plus anything outside the
staleness bound. **Shipping this task leaves that untouched**, and a reader
comparing the view against the candles will find them disagreeing by that margin.

**Why it is not folded in here.** Pointing the peg tier at [[0167]]'s
`prices.usd_rate` is the obvious fix and becomes possible once that table
exists — but it is a *write-path* change to the enrichment hot loop, which
[[0111]] is already the open performance task for, and correcting history means
re-enrichment rather than a view swap. Different risk class, different task.

⚠️ **Do not queue it ahead of [[0172]] on magnitude alone.** 0.07% on `close_usd`
is plausibly acceptable for TVL; 0172 is USDT candles reading ~0.14 against USDC,
a ~7× error on 102 live pools. Fix the order-of-magnitude problem first.

## Out of scope

- Building the rate table or populating it — [[0167]].
- **Correcting `close_usd` itself** (the enrichment peg tier above). Noted
  deliberately rather than filed, 2026-08-10 — file it when 0111 makes the
  enrichment write path safe to touch, or when someone needs better than 0.07%.
- `current_price_usd` / `current_prices`, which is suspected to carry the same
  base-only assumption as 0165 but is a refreshable-MV rebuild and must not ride
  along on a view swap.

## Notes

- Deep history stays flat `$1` permanently — there is no oracle reading before
  **2026-03-11** (measured on prod 2026-08-10; earlier task text said ~2025-09,
  which was never verified and is wrong).
  That is a data-availability fact, not a gap to close, and the same shape as the
  pre-Soroban tail having no USD reference at all.
