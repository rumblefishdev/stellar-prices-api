---
id: "0210"
title: "1.56M price_ohlcv_1m rows still carry the USDT $1 peg — 0172 declared it fixed and 0182 repaired only the coarse tiers"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0172", "0182", "0209", "0204", "0145", "0111"]
tags: ["priority-high", "effort-medium", "clickhouse", "data-correctness", "enrichment", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../../packages/enrichment-worker/src/bin/coarse-repair.rs"
history:
  - date: 2026-08-20
    status: backlog
    who: okarcz
    note: >
      Spawned from the 0209 root-cause investigation. Measured on prod:
      1,564,045 USDT-quoted _1m rows carry close_usd / close = 0.999, spanning
      2018-05-15 to 2026-08-13, against ZERO pivot-written rows. This is 0172's
      defect still live on the tier every coarse table rolls from. 0172 closed
      on the writer fix and a 90,741-row purge; 0182 repaired the coarse tiers
      and was archived against them.
---

# 1.56M `_1m` rows still carry the USDT $1 peg

## Summary

`price_ohlcv_1m` holds **1,564,045 USDT-quoted rows valued at `close × $1`**,
spanning 2018-05-15 → 2026-08-13. USDT trades at ~$0.14, so these overstate
`close_usd` by roughly **7.4×** — the identical defect [[0172]] was filed for,
declared fixed, and [[0182]] spent a 10–15 h production run repairing.

Neither task touched this tier. 0172 fixed the *writer* and purged 90,741 rows;
0182 repaired the *coarse* tiers. `_1m` is the tier every coarse table rolls
from, and it was never in either population.

## The measurement

```sql
-- pivot_written = close_usd/close < 0.5  (USDT's real rate, ~0.14)
-- peg_written   = close_usd/close >= 0.9 (close × $1)
pivot_written │ peg_written │ oldest_priced       │ newest_priced
            0 │   1,564,045 │ 2018-05-15 13:43:00 │ 2026-08-13 10:01:00
```

⚠️ `pivot_written = 0` is the companion finding and belongs to [[0209]]: the
pivot that was supposed to replace the peg has never written a `_1m` row, which
is why the peg values were never overwritten and why nothing has been priced
since 2026-08-13.

## Why this was invisible

Every surface a person or an alarm looks at reads the **coarse** tiers, and
those were repaired:

| surface | tier | reads |
|---|---|---|
| `price_usd_series`, BE's consumer | `_1h` | ✅ repaired by 0182 |
| [[0204]] gap-4 peg alarm (built, undeployed) | `_1h` | ✅ repaired — reads 0 |
| 0182's own post-run verification | 5 coarse tiers | ✅ its own output |
| **the tier they all roll from** | **`_1m`** | ⛔ **never checked** |

⚠️ **This is a verification defect as much as a data defect.** 0182 was verified
and archived against precisely the tiers its repair had written — the same shape
as the 2026-08-13 false recovery that [[0204]] exists for, and as the epoch bug
that made 0182 necessary twice.

## Why it is not merely cosmetic

The rollup MVs re-roll a bounded recent window from `_1m`. Today's peg rows sit
outside that window, so they are not currently propagating — **but any
re-roll of history puts them straight back into the coarse tiers**, and
[[0136]]/[[0090]]/[[0095]] all re-rolled history for unrelated reasons. The
repaired coarse values are correct data sitting on a foundation that will
overwrite them the moment anyone re-rolls.

## Implementation

- ⚠️ **Fix [[0209]] FIRST.** Repairing `_1m` while the pivot cannot write leaves
  the rows at zero instead of wrong — trading one defect for the other, which is
  exactly what 0172 did on 2026-08-13.
- Reuse 0182's `reset` + pivot mechanism; the population matches `reset_sql`
  (`close_usd > 0 OR volume_quote_usd > 0`) because these rows hold written
  values. ⚠️ Use epoch **`1612724400`**, never `1612656000` — see
  [[0208]] and the 157 candles the first run destroyed.
- Scope to `_1m` only. The coarse tiers are already correct; re-running them is
  the non-free non-idempotence `CANDIDATE_PRED`'s doc comment warns about.
- Take a `FREEZE` snapshot first (admin-only — `prices_writer` cannot).

## Acceptance Criteria

- [ ] `peg_written` on `_1m` reads **0** for the USDT leg, measured by the query
      above, not by the repair tool's own exit status.
- [ ] `pivot_written` is **> 0** and covers the same span — proving the rows were
      re-valued rather than merely zeroed.
- [ ] ⚠️ Verified on **`_1m`**, and separately on one coarse tier to confirm no
      regression there. A repair verified only on its own output is [[0182]]'s
      mistake repeated.
- [ ] A note records the pre- and post-repair row counts and the span.
