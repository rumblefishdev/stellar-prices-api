---
id: "0165"
title: "price_usd_series can never publish USDC — the view only emits base assets, so the canonical quote asset is structurally unpriceable"
type: BUG
status: active
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
  - date: 2026-08-07
    status: backlog
    who: okarcz
    note: >
      **Fully verified.** All three prod claims re-run: `price_usd_series` USDC
      = 0 rows (20.77M rows scanned, so the view was really evaluated),
      `price_ohlcv_1d` USDC-as-base = 0 candles, and the [[0139]] collision guard
      passes — USDC is `asset_id = 3`, uniquely USDC. Every CSV figure
      re-derived independently and reproduces exactly. A **second control**
      emerged from that re-derivation and is now the strongest evidence here:
      USDC at the canonical issuer is 0/1,433 priceable, USDC at 56 other
      issuers is 228/233 (97.9%) — same asset code, preference the sole variable.
  - date: 2026-08-10
    status: active
    who: okarcz
    note: >
      Activated for implementation. Nothing left to investigate — the defect is
      verified on prod, the fix is designed (zero-weight peg-fill arm unioned
      before the GROUP BY, so precedence falls out of the weighted average with
      no anti-join), and 0168's three prerequisites are folded into the ACs.
      Sequencing note: this edits views.sql, which 0134 made all CREATE OR
      REPLACE, so the change will actually land — unlike rollups.sql, which
      still carries the no-op footgun 0142 must fix first.
      Scope guard restated: current_prices / current_price_usd is SUSPECTED to
      carry the same base-only defect but that stays OUT of this task. It is a
      refreshable-MV drop + recreate, the operation that wiped the coarse tables
      in 0095. Audit query only here; any fix is its own task.
  - date: 2026-08-10
    status: active
    who: okarcz
    note: >
      IMPLEMENTED. Both grains of price_usd_series rewritten as a two-arm
      UNION ALL with a zero-weight peg placeholder keyed on the quote leg, plus
      the appended `method` provenance column. 11 unit + 15 Docker-gated CH
      integration tests green on the 26.3.10.60 prod pin; fmt clean; the 2
      clippy warnings are pre-existing on develop (verified by stashing).
      The new three-case test was confirmed to FAIL without the fix, two ways:
      on the missing `method` column, and - the behavioural proof - on the
      pre-existing count assertion going 3 -> 4, which is USDC appearing in the
      view for the first time.
      One deviation from the settled design, recorded as Emerged decision 2:
      the guard is countIf(is_peg = 0) = 0, not sum(w) = 0. The specified form
      would have converted a genuine NULL (peg asset whose only candles carry
      volume_base = 0) into a fabricated $1, which the "no NULL introduced in
      either direction" AC forbids.
      Audit AC discharged at code level: usd_reference/_1h are clean (they join
      both legs), identity_by_contract is N/A, and current_price_usd is
      CONFIRMED to share the base-only defect - current.sql groups by asset_id
      and quote_asset_id appears nowhere in the MV. Scoped out as agreed; it is
      a refreshable-MV rebuild. Prod measurement handed over rather than run.
      NOT DEPLOYED: views.sql needs a privileged applier on ch-prod-01 (DROP
      VIEW grant, which no scoped runtime user has), so this merges as a repo
      change and takes effect on an operator apply. Verification queries -
      including the USDT non-regression check and its rollback trigger - are in
      the new Deploy section.
  - date: 2026-08-10
    status: active
    who: okarcz
    note: >
      REVIEW FIXES (PR #188, second pass). Code review found the one deliberate
      deviation was wrong and it is RETRACTED - the guard is now the originally
      specified sum(w) = 0.
      Root cause of my error: I asserted the historical expression "yields NULL
      via nullIf" without testing it. It does not. close_usd is a NON-NULLABLE
      Decimal(38,14), so CAST strips the Nullable and a zero denominator lands
      as Decimal128::MIN (-1.7e24), not NULL. The countIf guard therefore
      published that garbage - flagged method='traded' - for a peg asset whose
      only candles carry zero volume, in the column BE multiplies into TVL.
      Two further review findings fixed: the "no NULL introduced" test assertion
      was VACUOUS (IS NULL on a non-Nullable column is structurally always 0) and
      now asserts on the value; and the 1h fixture was hoisted out of the loop
      where it had been a precondition disguised as an iteration step.
      Two review claims were corrected rather than accepted. (a) The finding's
      reproduction fixture - a single USDT/USDC candle - fails under BOTH guards,
      because USDT is a base only there so arm B emits no placeholder and
      max(is_peg)=0. sum(w)=0 only wins where the peg asset is ALSO a quote leg.
      Measured both fixtures before changing anything; the shipped test uses the
      one that actually discriminates. (b) The residual - any asset whose only
      priced candles carry zero volume still publishes Decimal128::MIN - is
      PRE-EXISTING, not peg-specific, and is spawned as 0171 rather than widened
      into this task, because the fix requires a contract decision with BE
      (omit the row vs substitute an unweighted statistic).
      Also corrected two overstated comments the review flagged: arm B is a full
      second FINAL pass, not a cheap narrow projection; and arm B is not subject
      to the close_usd > 0 predicate, so a peg identity can read status='ok' in a
      bucket where usd_reference is empty - intended, but it means the 12.3
      discriminator is not universal and that is now documented at the view.
      11 unit + 16 CH integration tests green on the 26.3.10.60 pin; the new
      zero-volume test was confirmed to fail with the countIf guard (got
      -1701411834604692300000000) and pass with sum(w) = 0.
  - date: 2026-08-11
    status: active
    who: okarcz
    note: >
      BE RE-MEASURED AND CONFIRMED - the last open acceptance criterion is met.
      Same methodology, same population, 52,494 pools (+125 since the CSV). The
      canonical-USDC cohort is 1,436 today: priceable_ever 0 -> 1,430 (99.6%),
      priceable_90d -> 1,089, priceable_48h -> 873, and 745 of the 956 currently
      active now price. TF/USDC, 224/USDC and the active GOLD/USDC all price. The
      6 stragglers at ever=0 are blocked by their OTHER leg, so they are genuine
      0154 territory, not residue of this defect. Overall never-priced pools went
      2,113 -> 682 (-67.7%), landing on the 67.8% predicted in Blast radius to
      the decimal.
      They also settled three other open items. 0171: OMIT THE ROW - "misses are
      absent" is what their whole read path assumes; carried into 0171, which is
      no longer blocked. The method column: no impact, every read pins an
      explicit column list. The $1 placeholder: inside their documented 1%
      tolerance, 0168 tightens it for free.
      THE USDT NON-REGRESSION FAILS ON THEIR SIDE - and it is 0172, not this
      change. Canonical USDT publishes traded closes of 0.129-0.143 for 08-04 ->
      08-10 with the newest bucket at the peg $1, so a consumer sees Tether
      flapping between $0.14 and $1.00. That escalates 0172 from "distortion on
      the USDT/USDC pair" to "a wrong published price for USDT's own identity
      series"; carried there. Note this does NOT trip the Deploy rollback
      trigger, which fires only if query 2 returns method='peg' ONLY - it returns
      both, so the peg arm did not swallow USDT's market data. Do not roll back.
      Methodology lesson recorded from their decomposition: the 48h coverage
      figure breathes +-2-3pp week to week (21,313 today vs 22,975 on 08-06 is
      mostly staleness drift, with this fix contributing +873), so single
      snapshots must not be compared without pinning the date - which applies to
      our own 0154 headroom quotes too.
  - date: 2026-08-11
    status: completed
    who: okarcz
    note: >
      COMPLETE - every acceptance criterion met. The last one was the read-surface
      audit's outstanding prod measurement, taken today: current_price_usd returns
      0 rows for USDC at the canonical issuer and 10 at other issuers, with USDT
      and native XLM as positive controls proving the predicate and the literals
      are sound. That is the issuer-split control from the original diagnosis
      reproducing on a second, independent surface - asset code fixed,
      quote-preference the sole variable.
      Shipped: a zero-weight peg-fill arm unioned BEFORE the aggregation on both
      grains of price_usd_series, plus an appended `method` provenance column.
      Live on prod. BE re-measured and confirmed the population: 0 -> 1,430 of
      1,436 canonical-USDC pools priceable, overall never-priced 2,113 -> 682
      (-67.7% against the 67.8% predicted).
      Spawned 0178 for the same defect in current_prices / current_price_usd,
      deliberately NOT fixed here: that is a refreshable-MV DROP + recreate, the
      operation that wiped the coarse tables in 0095, so it needs its own
      rollback plan. 0171 (Decimal128::MIN at zero volume) was spawned earlier
      from the code review and now has BE's contract decision.
      NOT fixed by this task and must not be reported as such: 0170
      (/assets/{USDC}/ohlcv still returns an empty 200 - different code path,
      a USDC/USDC self-pair) and 0178. Three surfaces carried the USDC hole; this
      task closed one of them.
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

## Evidence (prod, 2026-08-06 — re-verified 2026-08-07)

```
price_usd_series WHERE asset_code='USDC' AND issuer='GA5ZSEJY…KZVN'   -> 0 rows
price_ohlcv_1d   WHERE asset_id = <USDC>                              -> 0 candles
prices.assets    WHERE asset_id = <USDC>                              -> n=1, ['USDC']
```

Zero candles with USDC as base, at any time, so this is structural rather than a
gap in a window. The first query scanned 20.77M rows, so the view was genuinely
evaluated rather than short-circuited on an empty predicate.

✅ **The `asset_id` collision guard passes.** USDC resolves to **`asset_id = 3`**,
which is uniquely USDC — so the zero-candle count above is trustworthy. This
check is mandatory, not decorative: per Trap 1 below, while [[0139]] is open no
count keyed on a resolved `asset_id` means anything without it. (A low id is
expected here — USDC is among the first assets ever registered — unlike the `TF →
19` case that triggered the collision hypothesis.)

**Cross-checked against BE's per-pool CSV** (52,373 pools,
`pool-price-coverage-2026-08-06.csv`):

| Pool set | count | `priceable_48h` | `priceable_90d` | `priceable_ever` |
|---|---|---|---|---|
| has a **USDC** leg (canonical issuer) | 1,433 | **0** | **0** | **0** |
| has a **native XLM** leg | 11,686 | 3,111 | 7,390 | 11,505 |
| neither | 39,255 | 19,864 | — | 38,755 |

> ✅ **Every figure in this section re-derived from the CSV on 2026-08-07** and
> reproduces exactly, including the blast radius and top-three below.
>
> ⚠️ **The XLM row means *native* XLM** — `kind = 'native'`, equivalently
> `code = 'XLM' AND issuer = ''`. Counting *any* leg coded `XLM` instead gives
> 12,129 / 3,306 / 7,674 / 11,928 and silently disagrees with this table. The
> looser predicate was tried first during re-derivation and produced exactly that
> mismatch, so state the definition when quoting these numbers.

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

### A second control isolates the variable completely (added 2026-08-07)

Split the USDC-legged pools by **issuer**, holding the asset code fixed:

| USDC leg | pools | `priceable_ever` | |
|---|---|---|---|
| at the **canonical** issuer | 1,433 | **0** | **0.0%** |
| at **56 other** issuers | 233 | 228 | **97.9%** |

Same asset code, same peg semantics, same enrichment path, same view. The *only*
difference is that the canonical issuer is our top-preference quote and the
others are not — so the others still appear as a base, get rows, and price
normally.

This is stronger than the USDT control, which varies asset *and* preference
together. Here the asset code is held constant and preference is the sole
variable, across 1,666 pools: **0.0% vs 97.9%**. If the cause were anything about
stablecoins, peg handling, LP fills or resolver reach, both halves would behave
the same way.

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

- [x] `price_usd_series` and `price_usd_series_1h` return rows for USDC at the
      canonical issuer, for every bucket where any USDC-quoted trade occurred.
      **Verified locally on the 26.3.10.60 pin**; prod is an operator apply
      (see §Deploy below).
- [x] USDT and any other peg asset handled by the same rule — not a USDC
      special case. The arm is keyed on the peg **identity set**, and USDT is
      exercised as the both-legs control.
- [x] A regression test on CH **26.3.10.60** (`views_it.rs`) covering three
      cases: a peg asset seeded **quote-only** returns the fallback (fails
      today); a peg asset seeded as **both** returns the market value, **not**
      the fallback (catches the USDT-flattening regression); a non-peg asset is
      **bit-identical** before and after.
      `price_usd_series_fills_peg_assets_without_overriding_market_data`, run
      over **both grains**. **Confirmed to fail without the fix** — see
      §Issues Encountered for the two distinct pre-fix failures.
- [x] That test asserts **fallback semantics**, not the literal `1` — see
      §Implementation requirement 1, or [[0168]] rewrites it. Via a
      `PEG_FALLBACK` const documented as 0168's swap point.
- [x] `method` column shipped and appended last, with `'traded'`/`'peg'`
      distinguishable, so a measured `1.0000` and a fallback `1.0000` are never
      confusable — requirement 2. `'oracle'` documented as reserved for 0168.
- [x] The view header names [[0168]] by ID and states the ~0.1% error and the
      inconsistency with the oracle-tier candles — requirement 3.
- [x] The other `views.sql` surfaces audited for the same base-only assumption,
      with the result recorded either way. ✅ **COMPLETE — the outstanding prod
      measurement was taken 2026-08-11 and confirms the code-level finding.**
      Results:
      - `usd_reference` / `usd_reference_1h` — **clean.** Both join base *and*
        quote (`views.sql:207,328`) because they are a fixed XLM/USDC pair, not
        a per-asset series.
      - `identity_by_contract` — **N/A**, resolves contract→identity from
        `assets` and reads no candles.
      - `current_price_usd` — 🔴 **the suspicion is CONFIRMED in code.**
        `current.sql` aggregates `price_ohlcv_1m` with `GROUP BY asset_id,
        source` then `GROUP BY asset_id` (`current.sql:125,142`) and
        `quote_asset_id` appears **nowhere** in the MV. So `current_prices` has
        the identical base-only assumption and `/price` cannot return USDC.
        ✅ **MEASURED ON PROD 2026-08-11 — confirmed, and the issuer-split
        control reproduces on this surface:**

        | | count |
        |---|---|
        | USDC @ canonical issuer | **0** |
        | USDC @ **any other** issuer | **10** |
        | USDT @ canonical issuer (control) | 1 |
        | native XLM (control) | 1 |
        | total rows | 3,428 |

        The two controls prove the predicate and the hand-copied literals are
        sound, so the zero is a real absence and not a broken filter. The
        10-vs-0 issuer split is the same evidence that made the original
        diagnosis conclusive — asset code held fixed, quote-preference the sole
        variable — now reproduced on a **second, independent surface**.
        ⚠️ The usual "absent = didn't trade in 24 h" escape hatch does **not**
        apply: USDC has zero candles as a base at *any* time (§Evidence), so
        this absence is structural.
        **The fix stays scoped out and is filed as [[0178]]** — it is a
        refreshable-MV DROP + recreate, the operation that wiped the coarse
        tables in 0095, so it needs its own rollback plan.
- [x] No `NULL` introduced into either view's `close_usd`. ⚠️ **This AC's
      premise was FALSE and is corrected on the record: there is no NULL edge.**
      `close_usd` is non-Nullable `Decimal(38,14)`, so `CAST` strips the Nullable
      `nullIf` produces and a zero denominator publishes `Decimal128::MIN`, not
      NULL. The AC is met in the only sense that has meaning — no row publishes a
      non-positive `close_usd` for the cases this task covers — and the test now
      asserts on the **value** (`countIf(toFloat64(close_usd) <= 0) = 0`) rather
      than the vacuous `IS NULL`, which is structurally always 0. The uncovered
      cases are [[0171]].
- [x] **BE re-measures**: the 1,433 USDC-legged pools become priceable, and they
      confirm the count against their own CSV. ✅ **CONFIRMED 2026-08-11 ~09:00Z**,
      same methodology and population definition, on 52,494 pools (+125 since the
      CSV). **This closes the task's last open criterion.**

      | metric | before | after |
      |---|---|---|
      | canonical-USDC cohort | 1,433 | 1,436 |
      | `priceable_ever` | **0** | **1,430 (99.6%)** |
      | `priceable_90d` | 0 | 1,089 |
      | `priceable_48h` | 0 | 873 |
      | of the 956 currently active | 0 | 745 |
      | **overall never-priced pools** | **2,113** | **682 (−67.7%)** |

      The three named heavy pools all price (`TF/USDC`, `224/USDC`, active
      `GOLD/USDC`). The **6** stragglers at `ever = 0` are blocked by their
      *other* leg, not by USDC — i.e. genuine [[0154]] territory, not residue of
      this defect. The −67.7% lands on the **67.8%** predicted in §Blast radius,
      to the decimal.
- [x] [[0154]]'s headroom framing corrected — these pools were never resolver-
      limited and must not be counted as part of the pivot step's win.
      **Already done when 0165 was filed** (`0154:79-84` carves the population
      out explicitly); re-checked 2026-08-10, no further edit needed.

## BE's re-measurement response — 2026-08-11, everything else they said

The confirmation above came with four answers that settle open questions on
*other* tasks. Recorded here because this is where they were asked.

### 🔴 The USDT non-regression FAILS — and it is [[0172]], not this change

This is the one item that needs action, and it **escalates 0172**. BE observe
canonical USDT (`GCQTGZQQ…TG6V`) publishing **`method = 'traded'` daily closes of
0.129–0.143 for 08-04 → 08-10**, with the **newest bucket at the peg `$1`**.

Two things follow that were not previously on the record:

1. **0172 is not "distortion on the USDT/USDC pair".** It is a **wrong published
   price for USDT's own identity series** in `price_usd_series` — the surface BE
   consume. 0172 was filed off the pair; its blast radius is the asset.
2. **0165 and 0172 interact badly, and only in the direction of visibility.**
   A last-close consumer now sees Tether **flapping between $0.14 and $1.00**,
   because the traded buckets carry 0172's bad value while the newest
   (untraded-as-base) bucket takes this task's peg fallback. ⚠️ The peg arm does
   **not** cause this and does not make the data worse — arm B contributes 0/0
   wherever `sum(w) > 0`, so every traded value is arithmetically identical to
   what the old view published. What it does is put a *correct* $1 next to a
   *wrong* $0.14 in the same series, turning a uniformly-wrong column into a
   visibly discontinuous one. **That is a diagnostic improvement, not a
   regression** — but it means 0172 now presents as flapping rather than as a
   quiet 7× understatement, and it should be described that way.

**BE's ask: bump 0172's priority.** Carried into 0172 with their evidence.

⚠️ Note this against §Deploy's rollback trigger: that trigger fires only if
query 2 returns **`method = 'peg'` only**. It returns both, so the peg arm did
not swallow USDT's market data — **do not roll back**. The failure BE report is
a different one, on a different task, and the deploy check behaved correctly.

### ✅ `method` column — no consumer impact

Every BE read pins an explicit column list ("your own §2 rule, adopted on day
one"), so appending `method` broke nothing. The `views.sql:273` append-only rule
plus the pinned-column-list requirement did the job they were written for.

### ✅ The `$1` placeholder is inside their tolerance

~0.1% is well inside BE's documented **1%** tolerance; they take rows as-is and
[[0168]] tightens it for free. So the placeholder is not accruing consumer debt
while 0168 waits.

### ✅ [[0171]] — BE gave the contract decision: **omit the row**

Quoted, because it is the whole basis for 0171's fix: *"'Misses are absent' is
the contract our whole read path assumes — argMax over present rows, NULL when
nothing matches. A published sentinel forces every consumer to know a magic
constant forever, and this thread is the proof nobody reads release notes in
time."* They are adding a `close_usd > 0` guard on their side regardless — zero
occurrences in their read windows today, so insurance rather than a fix.
**0171 is no longer blocked on a contract decision.** Carried into 0171.

### ⚠️ Methodology lesson — the 48h coverage number breathes

Their overall 48h coverage read **21,313** today against **22,975** on 08-06,
which looks like a regression and is not. Decomposed: excluding `method = 'peg'`
it is **20,440** today vs **23,294** reconstructed as-of 08-06 — so this fix
contributes **+873** and the remainder is ordinary staleness drift over five
days. **The 48h figure moves ±2–3pp week to week, so single snapshots must not
be compared without pinning the date.** Applies to our own coverage reporting as
much as theirs — [[0154]]'s headroom numbers are quoted from 48h windows.

## Deploy — operator action, nothing auto-applies

`views.sql` is **not** applied by any deploy. `prices-clickhouse-init` applies
`INIT`/`VIEWS`/`ROLLUPS`/`SEED`, but on ch-prod-01 this file needs a
**privileged applier**: `CREATE OR REPLACE VIEW` requires a `DROP VIEW` grant
unconditionally on 26.3.10.60, and neither `prices_writer` nor `prices_reader`
has it (`views.sql:25-42`). So this merges as a repo change and takes effect
only when an operator applies it as the container's `default` user:

```bash
docker exec -i app-clickhouse-1 clickhouse-client   # no --user
```

Plain views replace **atomically** — no DROP window, no read-side exposure.
That is the reason this task is small and `current_price_usd` is not.

**Two prod queries to run at apply time** (hand-over, not run from here):

```sql
-- 1. The fix, on real data: USDC must now return rows, flagged 'peg'.
SELECT count() AS buckets, min(bucket), max(bucket), any(method)
FROM prices.price_usd_series
WHERE asset_code = 'USDC'
  AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN';

-- 2. The USDT non-regression: these must stay at their MARKET value, and
--    method must be 'traded' wherever USDT actually traded as a base.
SELECT method, count() AS rows, round(avg(toFloat64(close_usd)), 4) AS avg_close
FROM prices.price_usd_series
WHERE asset_code = 'USDT'
  AND issuer_address = 'GCQTGZQQ5G4PTM2GL7CDIFKUBIPEC52BROAQIAPW53XBRJVN6ZJVTG6V'
GROUP BY method;
```

⚠️ If query 2 returns **only** `method = 'peg'`, the peg arm has swallowed
USDT's real market data — that is the flattening regression, and the view should
be rolled back to the previous definition immediately.

**And the audit measurement for `current_price_usd`** (drives a separate task,
do not fix here):

```sql
SELECT count() FROM prices.current_price_usd
WHERE asset_code = 'USDC'
  AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN';
-- expect 0, confirming the code-level finding above
```

## Implementation Notes

- `packages/prices-clickhouse/schema/views.sql` — both `price_usd_series` and
  `price_usd_series_1h` rewritten as a two-arm `UNION ALL` sub-select with the
  aggregate applied over the union; `method` appended last on both. Header gains
  a peg-fill section (naming [[0168]], the ~0.1% error, the `ch_enrich.rs:20`
  contradiction), a `method` entry in the JOIN interop contract, and the USDT
  issuer added to the hand-synced-literal rule.
- `packages/prices-clickhouse/tests/views_it.rs` — new three-case test across
  both grains, plus assertions folded into the existing test.
- **Test results:** 11 unit + 15 Docker-gated CH integration tests green on the
  26.3.10.60 pin (the whole `prices-clickhouse` IT suite, not just `views_it`).
  `cargo fmt --check` clean; the 2 clippy warnings in `rollup_append_it` are
  **pre-existing on `develop`**, verified by stashing.
- No Rust consumer decodes these views, so the appended column breaks no
  in-repo positional decode (`grep` found only doc comments and the stub-name
  list at `lib.rs:288-290`).

## Issues Encountered

- **The `sum(w) = 0` guard from the settled design would have moved the NULL
  edge.** Writing it exactly as specified converts a peg asset whose only
  candles carry `volume_base = 0` from `NULL` (today's `nullIf` result) into a
  fabricated `$1` — which the "no NULL introduced … in either direction" AC
  forbids. Replaced with `countIf(is_peg = 0) = 0`. See §Design Decisions 1.
- **A pre-existing assertion had to change.** `views_expose_usd_series_and_reference`
  asserted `count() == 3` on `price_usd_series`. USDC is a quote on three of its
  seeded candles and a base on none, so it now gets a peg row → **4**.
  **Intentional, and it is the defect made visible**: with the fix reverted the
  assertion fails `left: 3, right: 4`. Not a regression.
- **Two distinct pre-fix failures were needed to prove the test bites.** Run
  against the pre-fix schema the new test fails on `Unknown expression
  identifier 'method'` — which proves the column is new but says nothing about
  behaviour. The behavioural proof is the `3 vs 4` count above. Recorded because
  a column-missing error is a weak pass/fail signal and could mask a peg arm
  that ships the column but never fires.

## Design Decisions

### From Plan

1. Zero-weight arm unioned **before** the `GROUP BY`, so precedence is
   arithmetic rather than policy. Rejected: appending `$1` after the group
   (duplicate keys → BE's join silently doubles aggregates), letting the peg arm
   own peg identities (flattens USDT), and the anti-join (two `FINAL` scans).

### Emerged

2. ~~**`countIf(is_peg = 0) = 0` instead of the specified `sum(w) = 0`.**~~
   🔴 **RETRACTED — this was wrong, and code review caught it. The shipped guard
   is the originally specified `sum(w) = 0`.**

   The reasoning above rested on "a genuine `NULL`", and **there is no NULL**.
   `close_usd` is a non-Nullable `Decimal(38,14)`, so `CAST` strips the Nullable
   that `nullIf` introduces; a zero denominator lands as **`Decimal128::MIN`
   (≈ -1.7e24)**, not NULL. Verified on the 26.3.10.60 pin:
   `toTypeName(CAST(sum(v)/nullIf(sum(w),0) AS Decimal(38,14)))` → `Decimal(38,14)`.

   So `countIf` did not "preserve an edge" — it **published a catastrophic
   negative number flagged `method = 'traded'`** for a peg asset whose only
   candles carry zero volume, in the column BE multiplies into TVL. `sum(w) = 0`
   returns the fallback there. The AC that drove the deviation ("preserve the
   `nullIf` → NULL edge") was itself premised on a NULL that does not exist —
   see §Acceptance Criteria.

   ⚠️ **The review's reproduction was directionally right but its fixture was
   not.** It cited a *single* `USDT/USDC` candle; in that fixture USDT is a base
   only, so arm B emits no placeholder, `max(is_peg) = 0`, and **both** guards
   publish the garbage. `sum(w) = 0` only wins where the peg asset is *also* a
   quote leg in the bucket. Measured both fixtures explicitly before changing
   the guard rather than taking the reproduction at face value; the shipped
   test uses the fixture that actually discriminates.
3. **The residual is split out as [[0171]], not widened into this task.** A peg
   asset appearing only as a zero-volume base — and every non-peg asset in that
   state — still publishes `Decimal128::MIN`. That is pre-existing (the historical
   view carried the identical expression) and fixing it means deciding whether
   such a row should be omitted, a change to the "misses are absent" contract
   that needs BE input.
4. **Arm B re-derives `asset_kind`/`asset_code` with the same `multiIf`/`if`
   normalisation as arm A** rather than hardcoding `'credit'`. The peg filter
   guarantees `'credit'` today, so this is redundant — but it keeps the two arms
   textually identical in their key construction, so the union cannot drift into
   emitting a differently-shaped key if the peg set ever widens.
5. **Explicit `toFloat64(0)` / `toUInt8(…)` in arm B** rather than bare literals,
   so the `UNION ALL` supertype is pinned instead of inferred.
6. **`method` values `'traded'`/`'peg'` with `'oracle'` reserved and documented
   but not emitted.** `'traded'` is deliberately an *addition* to [[0167]]'s enum,
   not a reuse: 0167's `method` describes how a **rate** was derived, whereas an
   arm-A row is a volume-weighted aggregate of candles some tier already priced
   — the view cannot know which, so claiming one of 0167's values would be a
   category error.
7. **Arm B costs a second pass over the candle table, accepted knowingly.** It
   projects only `(timestamp, quote_asset_id)`, so on a column store it reads far
   fewer columns than arm A rather than doubling cost — and it is strictly
   cheaper than the anti-join the design rejected. **Not measured on
   prod-scale data**; if the read latency regresses, [[0150]] (materialise the
   series as a table) is the escape hatch.

## Notes

- ⚠️ **This task does NOT fix `GET /assets/{USDC}/ohlcv` — see [[0170]].** Same
  root cause (USDC is always the quote), different code path: `/ohlcv` never
  reads `price_usd_series`, it queries `price_ohlcv_1d` directly with its own
  base+quote filter (`queries_ch.rs:545`), and `base_currency=USD` resolves the
  quote to USDC, so the default request asks for a **USDC/USDC self-pair**.
  Merging 0165 leaves that endpoint returning an empty `200` exactly as it does
  today. Do not report "the USDC pricing bug is fixed" on this task alone —
  0170 blocks two of [[0127]]'s M2 acceptance criteria.
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
