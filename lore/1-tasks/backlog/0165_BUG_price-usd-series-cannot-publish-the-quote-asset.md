---
id: "0165"
title: "price_usd_series can never publish USDC — the view only emits base assets, so the canonical quote asset is structurally unpriceable"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0144", "0154", "0147", "0150", "0151", "0061", "0139", "0136", "0167", "0168"]
tags:
  ["priority-high", "effort-small", "clickhouse", "data-correctness", "be-interop", "read-surface", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-06
    status: backlog
    who: okarcz
    note: >
      Found while running down a lead in BE's second 0199 response. They flagged
      "extremely active USDC-quoted pools whose base leg has never priced" and
      proposed that our trade ingestion covers the order book but not LP fills.
      **Their mechanism is falsified** (`ClaimAtom::LiquidityPool` is handled
      identically to order-book fills, `filter.rs:95`) **but their observation
      was right, and the real cause is the opposite leg**: it is not the exotic
      base that fails to price, it is **USDC itself**. Measured on prod, and
      cross-checked against their 52,373-pool CSV: **0 of 1,433 USDC-legged
      pools are priceable in any window**, which is 67.8% of every never-priced
      pool they have.
---

# `price_usd_series` can never publish USDC

## Summary

`prices.price_usd_series` emits one row per **base** asset per bucket:

```sql
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id   -- BASE only
WHERE p.close_usd > 0
```

`USDC` is our top-preference quote asset, so canonicalisation makes it the
**quote** on essentially every pair it appears in. It therefore almost never
appears as `p.asset_id`, gets no row, and is **unpriceable through the surface
we handed BE in [[0061]]** — despite being the asset the peg tier hardcodes to
exactly `$1`.

The asset we price everything else *against* is the one asset we cannot publish.

## Evidence (prod, 2026-08-06)

```
price_usd_series WHERE asset_code='USDC' AND issuer='GA5ZSEJY…KZVN'   -> 0 rows
price_ohlcv_1d   WHERE asset_id = <USDC>                              -> 0 candles
```

Zero candles with USDC as base, at any time, so this is structural rather than a
gap in a window.

**Cross-checked against BE's per-pool CSV** (52,373 pools,
`pool-price-coverage-2026-08-06.csv`):

| Pool set | count | `priceable_48h` | `priceable_90d` | `priceable_ever` |
|---|---|---|---|---|
| has a **USDC** leg | 1,433 | **0** | **0** | **0** |
| has an **XLM** leg | 11,686 | 3,111 | 7,390 | 11,505 |
| neither | 39,255 | 19,864 | — | 38,755 |

**Zero out of 1,433 in every window** is a categorical signature, not a
distribution. Nothing that depends on trading activity, enrichment lag or
resolver reach produces a clean zero across three independent windows.

### USDT is the control that proves the mechanism

| | pools | `priceable_ever` |
|---|---|---|
| any USDT leg | 106 | **102** |
| canonical-issuer USDT | 35 | **34** |

USDT is *also* a peg asset handled by the *same* enrichment tier — but it is not
the top-preference quote, so it still appears as a **base** in USDT/USDC and
USDT/XLM pairs, gets rows, and prices fine. The defect tracks **quote
preference**, not peg status, not asset class.

### Blast radius

- **1,433 pools**, 100% of them, permanently unpriceable to BE.
- **67.8% of all 2,113 never-priced pools** in their set are explained by this
  one defect. The remaining 678 are genuine [[0154]] / other territory.
- **831** of them are active, carrying **2,319,212** LP state changes in 30 days
  — this is the *most active* end of their list, not the tail: `TF/USDC`
  (167,951), `224/USDC` (157,643), `GOLD/USDC` (126,154).

## Why it was mis-attributed twice

**BE thought the base leg was unpriced.** Their column is `worst_leg_last_priced`
and for `TF/USDC` the worst leg is USDC, not TF. TF prices fine — 22 rows in
`price_usd_series`, 2026-07-03 → 08-05; 46,077 TF/USDC candles in `_1m` of which
39,547 carry a `close_usd`.

**We would have mis-attributed it too.** These pools sit inside the "~68% of
every tier has never had a USD price" figure that [[0144]] phase 0 attributed to
enrichment resolver reach and spawned [[0154]] for. **[[0154]] does not fix any
of these** — a second pivot hop prices candles whose *quote* is exotic, and these
candles are already priced. Nothing was wrong with the candles at all.

## Implementation — settled 2026-08-07

The fix is not "price USDC-as-base candles", because there are none and there is
no reason for there to be. It is that a **USD rate for a peg asset is a fact we
already hold and simply never publish**.

**Form: a zero-weight peg-fill arm, unioned BEFORE the aggregation.** Keep the
numerator and denominator separate in a sub-select; arm A is today's rows, arm B
emits a placeholder per candle where a peg asset is the **quote**:

```sql
SELECT asset_kind, asset_code, issuer_address, contract_address, bucket,
    if(max(is_peg) = 1 AND sum(w) = 0,
       CAST(1 AS Decimal(38, 14)),                              -- peg fallback
       CAST(sum(v) / nullIf(sum(w), 0) AS Decimal(38, 14)))     -- unchanged
FROM (
    -- arm A: today's rows.  v = close_usd*volume_base, w = volume_base, is_peg = 0
    -- arm B: INNER JOIN peg AS q ON q.asset_id = p.quote_asset_id
    --        SELECT 'credit', q.asset_code, q.issuer_address, '', p.timestamp, 0, 0, 1
)
GROUP BY asset_kind, asset_code, issuer_address, contract_address, bucket
```

`peg` CTE = `prices.assets FINAL WHERE contract_address = ''` AND (USDC or USDT
at its canonical issuer), literals hand-synced from
`prices_clickhouse::{USDC_ISSUER, USDT_ISSUER}` per the `views.sql` header rule.

**Why this shape — the precedence falls out of the arithmetic:**

| case | result |
|---|---|
| any non-peg asset | arm B contributes nothing → reduces to today's expression **byte-identically**, including the `nullIf` → NULL edge at zero volume |
| USDC, bucket with a USDC-quoted trade | no arm-A rows → `sum(w) = 0` → **$1** |
| USDT in a bucket where it also traded as base | `sum(w) > 0` → **market value wins**; the zero-weight row adds 0 to both numerator and denominator so it cannot perturb the average |

⚠️ **Two shapes that look simpler and are wrong.** Appending a `$1` row per peg
asset emits **two rows for the same key** wherever USDT also trades as a base,
and BE joins on `(identity, bucket)` — duplicate keys silently double every
downstream aggregate. Letting the peg arm *own* the peg identities flattens
USDT's 102 currently-priceable pools from their market rate to `$1` — a
regression dressed as a fix. And do **not** express precedence as an anti-join
(`… WHERE key NOT IN (SELECT … FROM traded)`): ClickHouse substitutes CTEs
textually, so `traded` referenced twice means two full `price_ohlcv_1d FINAL`
scans per query.

SAC legs come free — §12.4 collapses a SAC-wrapped leg to its classic identity at
*write* time, so a Soroswap USDC pool's `quote_asset_id` is already classic USDC.

### ⚠️ Three forward-compatibility requirements — from [[0168]]

The `$1` is a **placeholder**, not the answer. `oracle_worker` already polls
Reflector for USDC and USDT (`oracle-worker/src/lib.rs:33`), so a real
depeg-aware rate exists; [[0167]] snapshots it into `prices.usd_rate` and
[[0168]] swaps the constant for it. That swap is one expression **only if this
task ships the following**. Get them wrong and 0168 becomes a rewrite.

1. **Test the fallback semantics, not the constant.** Assert *no rate available
   → `$1`* and *rate available → that rate* (the second case simply unseeded for
   now). A test asserting "peg asset → exactly 1.0" has to be rewritten by 0168.
2. **Ship the provenance column now.** Once both paths exist, a consumer cannot
   tell `1.0000` (a real oracle reading) from `1.0000` (the fallback) — the
   `close_usd = 0` mistake of one value meaning several things, in a new surface.
   Far cheaper to append the column than to retrofit it.
   - Column: `method LowCardinality(String)`, **appended last** per the
     `views.sql:273` rule (new columns are appended, never inserted — that
     protects order, not arity, so in-cluster consumers must pin an explicit
     column list rather than `SELECT *`).
   - Values: `'traded'` for arm A, `'peg'` for the arm-B `$1` fallback, and
     `'oracle'` once 0168 lands a measured rate.
   - ⚠️ **`'traded'` is an addition to [[0167]]'s enum, not a reuse of it, and
     that is deliberate.** 0167's `method` describes how a *rate* was derived
     (`oracle`/`peg`/`pivot`/`pivot2`). An arm-A row is not a rate — it is a
     volume-weighted aggregate of candles that some tier already priced, and the
     view cannot know which. Claiming one of 0167's values there would be a
     category error. Reuse `'peg'`/`'oracle'` where they genuinely apply; add
     exactly one value for the case 0167 does not model, and say so in the
     header.
3. **Point the view header at [[0168]] by ID**, not at a prose caveat. The flat
   `$1` is a ~0.1% systematic error (small depegs are routine) and it
   **contradicts our own candles** — the oracle enrichment tier already prices a
   `TF/USDC` candle at `close × 0.9993` (`ch_enrich.rs:20`). A reader who finds
   that inconsistency must land on a task, not rediscover it as a bug.

**Constraints:**

- ⚠️ **No `NULL`.** BE: *"a NULL renders as a dash and removes the pool from
  every USD view we have."* Same constraint as [[0151]].
- **Check `price_usd_series_1h` too** — identical shape, same defect.
- **Check the other read surfaces** for the same base-only assumption before
  closing. `usd_reference*` joins base *and* quote so it is probably fine, but
  0144's C1/C2 lesson is that this class of defect is always wider than the
  instance you found.
- **Whatever ships must be visible in [[0150]]** if that materialises the view
  later, or the fix is lost at materialisation time.

## Acceptance Criteria

- [ ] `price_usd_series` and `price_usd_series_1h` return rows for USDC at the
      canonical issuer, for every bucket where any USDC-quoted trade occurred.
- [ ] USDT and any other peg asset handled by the same rule — not a USDC
      special case.
- [ ] A regression test on CH **26.3.10.60** (`views_it.rs`) covering three
      cases: a peg asset seeded **quote-only** returns the fallback (fails
      today); a peg asset seeded as **both** returns the market value, **not**
      the fallback (catches the USDT-flattening regression); a non-peg asset is
      **bit-identical** before and after.
- [ ] That test asserts **fallback semantics**, not the literal `1` — see
      §Implementation requirement 1, or [[0168]] rewrites it.
- [ ] `method` column shipped and appended last, with `'traded'`/`'peg'`
      distinguishable, so a measured `1.0000` and a fallback `1.0000` are never
      confusable — requirement 2.
- [ ] The view header names [[0168]] by ID and states the ~0.1% error and the
      inconsistency with the oracle-tier candles — requirement 3.
- [ ] The other `views.sql` surfaces audited for the same base-only assumption,
      with the result recorded either way. (Suspected: `current_prices` groups
      on the **base** `asset_id` at `current.sql:125,142,192,208`, so
      `current_price_usd` likely cannot return USDC either — **measure before
      believing it**, and scope any fix separately: that is a refreshable-MV
      DROP + recreate, the operation that wiped the coarse tables in 0095.)
- [ ] No `NULL` introduced into either view's `close_usd`. (The existing
      `nullIf` → NULL edge at zero volume is **preserved**, not added — the fix
      must not change it in either direction.)
- [ ] **BE re-measures**: the 1,433 USDC-legged pools become priceable, and they
      confirm the count against their own CSV.
- [ ] [[0154]]'s headroom framing corrected — these pools were never resolver-
      limited and must not be counted as part of the pivot step's win.

## Notes

- **BE's proposed mechanism was wrong but their report was still the most
  valuable thing in the exchange.** They could see a pattern in their own data
  that we could not see in ours, and they said so with the caveat "happy to be
  wrong about the mechanism" — which is exactly why the lead was worth chasing
  rather than answering from the schema.
- ⚠️ **The first diagnostic query was misleading and nearly sent this the wrong
  way.** Resolving `TF` → `asset_id 19` and counting candles on that id returned
  182,747 rows, which looks like "TF is fine". `19` is an implausibly low
  surrogate for an obscure credit asset, so [[0139]] collision was the working
  hypothesis for a while. It was wrong — `asset_id 19` is uniquely TF — but the
  general point stands: **while 0139 is open, no query that resolves an identity
  to an `asset_id` and then counts on that `asset_id` can be trusted without
  first checking the id for collisions.**
- Confirms [[0154]]'s thesis on a second asset, incidentally: every TF-quoted
  candle is unpriced (AQUA, BTC, ETH, all `priced = 0`) while TF itself prices
  fine as a base — the same shape as the yXLM case 0154 was filed on.
- `priceable_ever` is bounded by asset age. TF's candles begin 2026-07-03, so
  "ever" is ~5 weeks for it. Do not read the column as "since genesis" for
  recently-discovered assets.
- TF's 22 daily rows across a 34-day span are consistent with [[0136]]'s
  2026-07-21 → 08-03 coarse freeze — the gap pre-roll for that is unblocked as
  of [[0145]] and will fill them.
