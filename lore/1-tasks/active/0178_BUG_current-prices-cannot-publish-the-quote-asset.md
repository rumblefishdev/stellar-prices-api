---
id: "0178"
title: "current_prices / current_price_usd cannot publish USDC either — the MV groups on the base leg, so /price has the same structural hole 0165 fixed in the series views"
type: BUG
status: active
related_adr: []
related_tasks: ["0165", "0072", "0095", "0139", "0150", "0170", "0061"]
tags:
  [
    "priority-high",
    "effort-medium",
    "clickhouse",
    "data-correctness",
    "read-surface",
    "be-interop",
    "refreshable-mv",
    "milestone-M2",
  ]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-11
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0165]]'s read-surface audit, which was deliberately scoped
      to the series views. The audit found this defect at code level on 08-10 and
      the confirming prod measurement was taken 08-11: USDC at the canonical
      issuer returns 0 rows from current_price_usd, while USDC at 10 OTHER
      issuers is present - the same issuer-split control that made 0165's
      diagnosis conclusive, reproduced on a second surface. Kept out of 0165 on
      purpose: that fix was an atomic plain-view replace, whereas this one is a
      refreshable-MV DROP + recreate - the operation that wiped the coarse tables
      in 0095 - so it needs its own rollback plan and its own task.
  - date: 2026-08-31
    status: active
    who: okarcz
    note: >
      Activated. Picked as the next piece of work because it is the sole
      remaining blocker on [[0120]], an M2 conformance criterion, and it is the
      second surface of the defect [[0165]] closed in the series views. Starting
      with the "Design question to settle first" section - provenance of a
      measured 1.0000 versus a peg placeholder, and whether the peg identities
      keep their market price - before any schema change, since the fix is a
      refreshable-MV DROP + recreate and needs its rollback plan written first
      per [[0095]].
---

# `/price` cannot return USDC — same defect, harder fix

## Summary

[[0165]] fixed `price_usd_series*`, which emitted one row per **base** asset. The
same base-only assumption sits in `current_prices`, so the **live spot** surface
has the identical hole: **our top-preference quote asset has no current price.**

`current.sql` derives every row from `price_ohlcv_1m` grouped by `asset_id`
(`current.sql:125,142`); **`quote_asset_id` appears nowhere in the MV**.
`current_price_usd` then joins `current_prices` to `assets` on `asset_id` alone
(`views.sql:491-509`), so it can only ever surface assets that appear as a base.

## Measured on prod — 2026-08-11

```
current_price_usd:
  USDC @ canonical issuer (GA5ZSEJY…KZVN)  ->      0
  USDC @ any OTHER issuer                  ->     10
  USDT @ canonical issuer (control)        ->      1
  native XLM              (control)        ->      1
  total rows                               ->  3,428
```

The two controls prove the predicate and the hand-copied issuer literals are
sound, so the zero is a real absence rather than a broken filter.

**The 10-vs-0 issuer split is the evidence, not the raw zero.** Asset code held
fixed, quote-preference the sole variable — the same control that made 0165
conclusive, now reproduced on an independent surface. If the cause were anything
about stablecoins, peg handling or enrichment reach, both halves would behave
alike.

⚠️ **"Absent = it didn't trade in 24 h" does not explain this.** `current_prices`
does only hold assets with a `price_ohlcv_1m` row in the last 24 h, but 0165
established USDC has **zero candles as a base at any time**, so the absence is
structural rather than a quiet day.

## Why this is NOT a copy of 0165's fix

The defect is the same shape; the deployment risk is not.

| | [[0165]] | this task |
|---|---|---|
| object | plain `VIEW` | **refreshable MV + `TO` table** |
| change mechanism | `CREATE OR REPLACE` — **atomic**, no read-side exposure | **`DROP VIEW` + re-`CREATE`** (a refreshable MV's definition is fixed at create time; `ALTER` does not take — `current.sql:18-23`) |
| worst case | none — replacement is atomic | ⚠️ **this is the operation that wiped the coarse tables in [[0095]]** |
| rollback | re-apply previous definition | needs a plan *before* the drop |

`current_prices` keeps serving its last-written rows during the gap, so the
exposure is a staleness window (~1 refresh) rather than an outage — **provided
the recreate succeeds**. The 0095 lesson is that this is exactly where an
apply-time mistake destroys data rather than merely delaying it.

## Design question to settle first

0165's answer was a **zero-weight peg-fill arm unioned before the aggregation**,
so precedence falls out of the weighted average. The same shape probably
transplants, but `current_prices` is materially different:

- It is a **tip** surface (one row per asset), not a per-bucket series, so there
  is no bucket key to union on — precedence has to be decided per asset.
- It carries derived columns the series views do not: `price_xlm`,
  `change_24h_pct`, `change_7d_pct`, `market_cap_usd`, `vwap_24h`, `sources`.
  **What should a peg-filled USDC row report for those?** A `$1` price with a
  fabricated `change_24h_pct` would be a new instance of the
  [[0144]] "one value meaning several things" defect.
- [[0167]]'s `prices.usd_rate` is live, so unlike 0165 this task can reach a
  **real** depeg-aware rate rather than a placeholder — and probably should,
  since [[0168]] is a one-expression change. Consider going straight to the
  measured rate here and skipping the `$1` step entirely.

⚠️ Whatever it emits must carry a **provenance value** distinguishable from a
traded reading, per 0165's requirement 2. `current_prices` has no `method`
column today, so adding one is part of this task.

## Acceptance Criteria

- [ ] `GET /price` (and `current_price_usd`) returns a row for USDC at the
      canonical issuer, with a plausible USD value.
- [ ] Provenance is expressible — a consumer can tell a measured `1.0000` from a
      filled one. Requires adding the column; follow `views.sql:273` (append
      last) and 0165's `'traded'`/`'peg'`/`'oracle'` vocabulary.
- [ ] The derived columns are decided explicitly, not left to fall out of the
      arithmetic — each is either populated meaningfully or lands on its
      documented "unavailable" sentinel. **No fabricated `change_*` values.**
- [ ] USDT and the other peg identities are not flattened from their market
      value — 0165's regression, re-tested here.
      ⚠️ **Sequencing: [[0172]] must land first or this control is unreadable**,
      because USDT's market value is currently wrong (~$0.14).
- [ ] A rollback plan written **before** the DROP, given [[0095]]. At minimum:
      the current definition captured verbatim, and the `TO` table's row count
      recorded immediately before and after.
- [ ] Regression test on the 26.3.10.60 pin.
- [ ] BE told — `/price` is a surface they consume, and "USDC pricing is fixed"
      is currently only true of the series views.

## Notes

- ⚠️ **Do not report "the USDC pricing bug is fixed" while this is open** — the
  same warning [[0165]] carries about [[0170]]. Three surfaces had the USDC hole:
  the series views (fixed), `/assets/{USDC}/ohlcv` ([[0170]], different root
  cause), and this one.
- [[0150]] (materialise `price_usd_series` as a table) overlaps: if that lands
  first it may supply a cleaner source for the tip than re-deriving from `_1m`.
- ⚠️ [[0139]] is open, so any diagnostic here that resolves an identity to an
  `asset_id` and counts on it must check that id for collisions first.
