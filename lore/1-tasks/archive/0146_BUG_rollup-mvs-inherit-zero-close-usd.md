---
id: "0146"
title: "All six rollup MVs zero a coarse row's close_usd when its newest sub-bucket is un-enriched"
type: BUG
status: completed
assignee: akot
related_adr: ["0287"]
related_tasks: ["0144", "0145", "0142", "0137", "0148", "0149", "0095", "0136", "0286", "0212"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "materialized-view", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
history:
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0144]] future work (phase 3) — BE 0199 finding 3ii,
      reproduced on the prod CH pin. The single highest-value fix in the chain
      and the only one with a real delivery problem.
  - date: "2026-09-14"
    status: backlog
    who: akot
    note: >
      Added an explicit-`ifNull` item for `vwap` to Implementation and the
      acceptance criteria, from PR #312's review of [[0171]]: the rollup and
      preroll `vwap` is a Nullable(Decimal) written into a non-Nullable column
      and relies on `insert_null_as_default = 1`. Nothing wrong on prod today
      (zero `volume_base = 0` rows on any tier); to be made explicit when the
      MVs are re-created here. No other change.
  - date: "2026-09-14"
    status: active
    who: akot
    note: >
      Activated; taken by akot after [[0171]]/[[0198]] (PR #312). First of the
      remaining 0144 chain because all its preconditions are closed (0142,
      0137, 0145) and it changes the data [[0147]] will measure its threshold
      on. Pre-flight before the first DROP: confirm the MV re-aggregation
      windows do not reach back to 2026-08-13, or [[0212]]'s peg-valued _1m
      rows re-enter the coarse tiers.
  - date: "2026-09-17"
    status: completed
    who: akot
    note: >
      Closed as superseded by [[0286]]. The form of `close_usd` and `vwap`
      this task specified is replaced by ADR 0287 / PR #320: coarse
      `close_usd` = `close × argMaxIf(close_usd / close, t.timestamp,
      close_usd > 0 AND close > 0)` (the latest priced child's RATE, which
      skips the un-enriched sentinel as `argMaxIf(close_usd, …)` would have
      and keeps `close`/`close_usd` on one bucket), `vwap` in Float64 with an
      explicit zero. The six MVs are re-created ONCE, under
      `docs/runbooks/0286-candle-definitions-rollout.md` (§4–§5, §8) — a
      separate 0146 re-CREATE would open a second DROP window for nothing.
      No code was written under this task. NOT yet on production: PR #320 is
      open on this date and the re-CREATE + per-MV freshness check are
      tracked by 0286's open rollout criterion, not here. Pre-flight from the
      2026-09-14 entry, answered from the file: 1m→15m reads a 2 h window, so
      [[0212]]'s peg-valued _1m rows cannot re-enter; 1d→1w (60 d) and 1d→1M
      (400 d) reach past 2026-08-13 but read 1d, which [[0182]] repaired —
      confirm on prod before the first DROP.
---

# Rollup MVs inherit `close_usd = 0` from an un-enriched sub-bucket

> **Superseded by [[0286]] (2026-09-17).** The form of `close_usd` and `vwap`
> below is replaced by ADR 0287 / PR #320, and the six MVs are re-created once,
> under `docs/runbooks/0286-candle-definitions-rollout.md`. The text below is
> kept as the original analysis; see the acceptance criteria for what landed
> where.

## Summary

Every rollup tier carries the USD close forward with

```sql
argMax(close_usd, t.timestamp)            AS close_usd
```

(`rollups.sql:90, 111, 132, 153, 174, 195` — all six MVs). `argMax` takes the
value from the **latest** sub-bucket. When that sub-bucket is not yet enriched
its `close_usd` is `0`, so the coarse row inherits 0 and **discards the priced
sub-buckets underneath it**. A partly-enriched hour does not roll up as partly
priced; it rolls up as unpriced.

Downstream, `price_usd_series*`'s `WHERE close_usd > 0` then drops the bucket
entirely — which is the disappearing-bucket half of BE's yXLM observation
([[0144]] finding 3ii, their 14:13 reading).

Reproduced on CH **26.3.10.60** with a single writer and no version
interaction — [[0144]] `repro/03_tests.sql`, TEST A:

```
timestamp             close   volume_base   close_usd_asis   close_usd_if_guarded
2026-08-04 13:00:00   0.172         42000                0                  0.171
```

## Why this one is hard: delivery, not the fix

The fix is one function per MV. Landing it is the problem.

All six are `CREATE MATERIALIZED VIEW IF NOT EXISTS … REFRESH … APPEND TO …`.
**`IF NOT EXISTS` does not redefine an existing object**, and there is no
`CREATE OR REPLACE` form for a refreshable `TO`-table MV. Editing the file and
re-applying it on ch-prod-01 changes nothing and reports success — that is
[[0142]], which must land first.

The only route is DROP + re-CREATE, and that is not free:

- **The DROP window is a data-loss window** — [[0090]]/[[0095]] are the
  precedent. Any re-CREATE must preserve APPEND + `sum(version)` + aligned
  windows or it silently reintroduces the replace-mode wipe.
- **A dropped MV stops rolling up while it is gone** — [[0136]] went nine days
  unnoticed with no alarm. [[0137]]'s freshness alarm should be deployed before
  the first DROP.

## Implementation

1. **[[0142]] first, but only its cheapest deliverable** — drift detection
   comparing `system.tables.create_table_query` against `rollups.sql`. It
   touches no production object and converts a silent no-op into a loud one.
   Do not let 0142 grow into "convert all six" before this ships.
2. **[[0137]] freshness alarm deployed** before any DROP window opens.
3. `argMaxIf(close_usd, t.timestamp, close_usd > 0)` at all six sites.
   **While the six MVs are being re-created anyway**, also make `vwap`
   explicit: `ifNull(volume_quote / nullIf(volume_base, 0), 0) AS vwap`
   (same at the matching sites in `preroll*.sql`, so the two definitions do
   not drift). Today the expression is `Nullable(Decimal)` written into a
   non-Nullable column and it only works because `insert_null_as_default = 1`
   (prod: 1) turns the NULL into the column default 0 — measured on
   26.3.10.60 for both `INSERT SELECT` and a refreshable `APPEND` MV in
   [[0171]]'s review round (PR #312, finding 1). Prod has zero
   `volume_base = 0` rows on `_1m`/`_15m`/`_1h` today, so nothing is wrong;
   the value should simply not depend on a server setting.
4. DROP + re-CREATE **one MV at a time**, with the [[0095]] invariants as a
   pre-flight checklist and expected freshness recovery stated per step. Model
   the procedure on [[0136]]'s per-table recovery runbook.
5. **Document the decoupling in the file header**: `close` and `close_usd` may
   now come from different sub-buckets. That is the right trade — an
   approximately-right USD close beats a fabricated zero — but two columns
   silently ceasing to be same-row is exactly what bites a future reader.
6. Regression test on 26.3.10.60 reproducing TEST A.

## Ordering notes

- Ships **after** [[0145]] (same fix, no delivery problem, has a deadline).
- Once this lands, rows *inside* each MV's re-aggregation window self-heal —
  the MV re-appends a correct value instead of a zero. That is what demotes
  [[0149]] (the version race) from blocker to hygiene.
- Rows *outside* the windows stay frozen and are [[0148]]'s problem.

## Acceptance Criteria

- [x] [[0142]] drift detection in place; a divergence between `rollups.sql` and
      the live definitions is visible rather than silent.
      → 0142 archived; `prices-clickhouse-drift` + the `mv-drift` alarm.
- [x] [[0137]] freshness alarm deployed before the first DROP.
      → 0137 archived; a pre-flight row of the 0286 rollout runbook.
- [x] `vwap` written as `ifNull(volume_quote / nullIf(volume_base, 0), 0)`
      in all six MVs and the matching `preroll*.sql` sites, so a zero-volume
      coarse group writes 0 by construction rather than by
      `insert_null_as_default` (from [[0171]]'s review).
      → Delivered by [[0286]] in a different form: Float64 division converted
      with `ifNull(toDecimal128OrZero(toString(…), 14), toDecimal128(0, 14))`,
      because Decimal division silently overflows past ~1.7e10 on 26.3.10.60.
      One generator (`rollup_sql.rs`) renders `rollups.sql` and the prerolls.
- [ ] All six MVs use `argMaxIf`; APPEND + `sum(version)` + aligned windows
      verifiably preserved after re-CREATE.
      → Superseded, then deferred to [[0286]]: the SQL is the rate form (ADR
      0287 §5), not `argMaxIf(close_usd, …)`; the MV set changes too
      (`mv_ohlcv_1w_to_1M` → `mv_ohlcv_1d_to_1M`). The production re-CREATE is
      0286's open rollout criterion.
- [x] No coarse row carries `close_usd = 0` while `close > 0` and a priced
      sub-bucket exists underneath it — regression test on 26.3.10.60.
      → `rollup_pf_it` (latest priced child un-enriched, rate taken from the
      one before), `rollup_chain_it`, `preroll_close_usd_guard_it` — PR #320.
- [ ] `close` / `close_usd` decoupling disclosed in the header.
      → Obsolete: the rate form re-prices the bucket's own `close`, so the two
      columns are same-bucket by construction; the `rollups.sql` header in
      PR #320 says so.
- [ ] Per-MV freshness confirmed recovered after each re-CREATE.
      → Deferred to [[0286]] (rollout runbook §8e).

## Implementation Notes

None under this task — no code, no production change. Everything it asked for
ships with [[0286]] (PR #320) and its rollout runbook.

## Design Decisions

### Emerged

1. **Closed as superseded rather than kept open as a rollout tracker.** 0286
   already carries the MV re-CREATE as an unchecked criterion; two tasks
   tracking one operator step is how the 0120-style stale blocker happens.
2. **No standalone `argMaxIf` delivery.** It would re-create six MVs twice —
   two data-loss windows ([[0095]]) — for a form ADR 0287 replaces anyway.
