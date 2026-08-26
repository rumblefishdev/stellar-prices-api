---
id: "0123"
title: "VWAP reconciliation — current_prices verifiable against raw price_ohlcv rows for ≥3 assets"
type: TEST
status: active
related_adr: ["0004", "0007"]
related_tasks: ["0072", "0118", "0116", "0120", "0128"]
tags: [layer-database, priority-high, effort-medium, milestone-M2, vwap, clickhouse, verification, acceptance]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Owns Tranche 2
      acceptance criterion 4 — the only AC that checks the §5.5 formula is
      arithmetically what the doc says, rather than merely that a number is
      returned.
  - date: 2026-08-26
    status: active
    who: stkrolikiewicz
    note: >
      Promoted to active — starting the reconciliation pass. Plan: recompute
      the §5.5 pipeline independently (own script, not a copy of the CTEs)
      from price_ohlcv_1m FINAL for assets off [[0120]]'s 20-major list —
      at least one with >= 3 sources (mask arms), one single-source control,
      one with a venue in carry. Reconcile vwap_24h/sources (masked
      population) separately from price_usd/volume_24h_usd (unmasked).
      Compare at pinned updated_at; measure on a healthy pipeline
      ([[0215]] fixed 2026-08-24, check [[0220]] before reading).
      Side product to record: the distribution of per-venue deviations from
      the median — the tuning basis current.sql's OUTLIER_PCT comment defers
      to this task, and an input [[0217]] waits on.
  - date: 2026-08-26
    status: active
    who: stkrolikiewicz
    note: >
      Assets selected from a prod measurement (~13:03 UTC), not from memory —
      per-venue 24h profile of all 20 majors. Subjects: XLM (full pipeline,
      mask armed), EURC (same shape, independent), BTC (2-source weighting
      arithmetic), AQUA (guard cuts 3→2, mask must not arm). Controls:
      USDCAllow (all-quiet keep arm, rank-1 volume), SCOP (src_price=0
      population filter vs guard). ETH rejected: cross-source argMaxIf tie
      on newest_priced makes price_usd non-deterministic — recorded as an
      explainable-delta class instead. Enrichment tip lag ~45 min, carry
      engaged nearly everywhere: the recompute must use priced-close
      semantics or it will mismatch by construction.
  - date: 2026-08-26
    status: active
    who: stkrolikiewicz
    note: >
      Run 1 executed and reconciled clean: 41/41 checks over 6 assets at a
      pinned T = 13:22:00 UTC — volumes exactly equal, vwap within 1.4e-11
      of the Float64 tolerance, sources JSON Decimal-exact, exclusions
      attributed to the right rule (guard vs population filter; mask empty
      as measured). Task converted to directory; evidence in benchmark/.
      Two findings recorded: cross-quote argMaxIf ties on 4 of 6 assets
      (published values asserted as tie-set members; a naive recompute
      false-positives at 3.0e-04), and window-state drift between selection
      and capture (phoenix revived, guard case moved to AQUA). Remaining:
      the public-API serialization AC — needs API_KEY/BASE_URL.
---

# VWAP reconciliation against raw OHLCV rows

## Summary

Tranche 2 AC 4: *"VWAP calculation verifiable against raw `price_ohlcv` rows for
at least 3 assets."*

Recompute §5.5's cross-source weighted price **by hand** from
`price_ohlcv_1m` for at least three assets and show it matches what
`current_prices` (and therefore `GET /assets/{id}/price`) serves.

## Context

This is the AC that catches a whole class of silent errors the other Tranche 2
checks cannot: a `vwap_24h` that is *plausible* but computed over the wrong
window, the wrong sources, the wrong weight, or the wrong precision still passes
schema validation ([[0120]]) and still meets the latency bar ([[0121]]).

The 0114 experience is the precedent worth carrying: a repaired month passed
every count-based check while an *absurd-value* gate failed for a reason that
turned out to be the gate, not the data. The lesson recorded there —
**verify the reference, not just the output** — applies directly here.

Known arithmetic hazards already documented in `current.sql`:

- `vwap_24h` is computed in **Float64** and cast back with `toDecimal128(…,14)`
  because `Decimal × Decimal` overflows `Decimal(38,14)`'s scale budget. So an
  exact equality check will fail on the last digits; the reconciliation needs a
  **relative tolerance**, stated and justified, not `==`.
- `price_usd` is `argMax(close_usd, timestamp)` over the 24h window — a *latest*
  value, not an average. Do not reconcile it as one.
- Every USD column derives from `close_usd` / `volume_quote_usd`, which the
  ingest path writes as 0 and enrichment fills later. A window whose enrichment
  has not run yet will reconcile to zero on both sides and prove nothing —
  choose windows with confirmed enrichment coverage.

**Dependency:** meaningful reconciliation needs [[0072]] (so `sources` exists to
check the per-source breakdown against) and ideally [[0118]] (so the threshold
rule is part of what is verified).

## Implementation

- Pick ≥3 assets with **multi-source** liquidity — the whole point is
  cross-source weighting, so a single-source asset is not a valid subject. At
  least one must have both an SDEX and an AMM source in the window.
- Freeze a specific 24h window and record its bounds; `current_prices` refreshes
  every minute, so an un-pinned comparison is a moving target. Capture the
  `current_prices` row and the raw `price_ohlcv_1m` rows in the same breath.
- Recompute independently — ideally outside ClickHouse (a script over an
  exported CSV) so a bug in the MV's SQL cannot reproduce itself in the check:
  - per-source 24h volume and latest close
  - the §5.5 weighted price `Σ(price × volume) / Σ(volume)`
  - the `min_volume_usd` exclusion ([[0118]]) and the median-outlier exclusion
    ([[0072]]) — assert the *excluded set*, not only the final number, since two
    different exclusion rules can coincidentally produce the same output
- Reconcile `volume_24h_usd` (a plain sum — this one **can** be checked tightly)
  and the `sources` JSON breakdown per source.
- State a tolerance and justify it from the Float64 path above.
- Also reconcile through the **public API** for at least one asset, so the check
  covers serialization: `Decimal(38,14)` values are serialised as **strings**
  by design (§3.3) precisely to avoid float truncation — confirm no precision is
  lost between CH and JSON.

## Asset selection — measured on prod 2026-08-26 ~13:03 UTC

Per-venue profile over the trailing 24h (`price_ohlcv_1m FINAL`, read-only via
the 0072 runbook's ssh path), all 20 of [[0120]]'s majors; identity→id
resolution collision-checked per [[0139]] (20 identities → 20 distinct ids).

**Subjects (multi-source, count toward the ≥3 AC):**

| asset | id | venues in window | why this one |
|---|---|---|---|
| XLM | 4 | sdex+aquarius+soroswap live, phoenix stale | Exercises **every** stage: conditional guard drops phoenix (live=0 at 10:49), mask **arms** over the 3 survivors, carry engaged on all three. Canary asset (`price_xlm = 1`). |
| EURC | 430 | sdex+soroswap+aquarius live, phoenix stale | Same full shape as XLM on an independent asset — mask armed at exactly 3 kept. |
| BTC | 108 | sdex+aquarius, both live | 2-source: mask all-true, so the weighted mean is checkable in isolation; real 0.67% venue spread (78,375 vs 78,898) makes the weighting arithmetic non-trivial. |
| AQUA | 5 | 3 raw venues; soroswap stale → guard cuts to 2 | Pins the guard→mask interaction: mask must **not** arm, because it counts the *kept* population (2), not the raw one (3). |

**Controls (extra, not counted toward the AC):**

- **USDCAllow (741)** — single venue, newest candle 08:55 (>4h stale) yet
  published: the guard's *all-quiet → keep everything* arm. Also store rank-1
  by volume ($36.6M), so the largest row in the table gets reconciled.
- **SCOP (70)** — aquarius present in the window with `src_price = 0` (25
  candles, none ever priced): excluded by the `WHERE src_price > 0` population
  filter, **not** by the liveness guard. The excluded-set assertion must
  attribute each exclusion to the right rule.

**Excluded, with reasons:** USDC (structural, [[0178]] — no rows as base);
AUD (zero `_1m` rows in the window — did not trade); RON (stale-only, $4
volume, price 7e-8 — near the precision floor ADR 0011 §7a warns about);
**ETH deliberately rejected** — both venues share `newest_priced = 12:16`, and
`price_usd` resolved the `argMaxIf` tie to aquarius (2448.57) over sdex
(2427.08); tie-break across sources is non-contractual, so any `price_usd`
assertion on it would be flaky. Recorded instead as an explainable-delta class
for the reconciliation.

**Window-state facts to carry into the recompute:** enrichment tip lag ~45 min
(newest candles 13:02, newest priced 12:16) — carry engaged on nearly every
live venue, so the recompute must mirror `argMaxIf(close_usd, ts, >0)`
semantics, a plain `argMax` will mismatch. Phoenix's exclusion on XLM/EURC is
**guard**, not mask — its price sits 0.66% from the median, far inside the 20%
band; today's live data therefore exercises guard-exclusions, and the
mask-exclusion assertion is expected to hold on an **empty** set.

## Reconciliation run 1 — 2026-08-26, T = 13:22:00 UTC

**Result: ALL RECONCILED — 41/41 checks across 6 assets.** Evidence in
`benchmark/`: `reconcile.py` (independent recompute, Python stdlib, contract
reimplemented from §5.5 + the current.sql column contract — not a translation
of the CTEs), `current.csv` (the pinned `current_prices` rows, full Decimal
precision), `raw-T1322.csv.gz` (29,108 `price_ohlcv_1m FINAL` rows in
`[T−24h, T]`, captured 80 s after the tick), `q-raw.sql`, `report.txt` (full
output). Re-run: `python3 benchmark/reconcile.py benchmark/current.csv <(gunzip -c benchmark/raw-T1322.csv.gz)`.

| asset | sources | vwap rel. delta | volume delta | exclusions (attributed) |
|---|---|---|---|---|
| XLM | 4, mask armed | 5.1e-14 | exact (0E-14) | none; max deviation phoenix 0.246% |
| EURC | 4, mask armed | 8.1e-15 | exact | none; max deviation phoenix 0.824% |
| BTC | 2 | 1.9e-16 | exact | none |
| AQUA | 3 → guard cuts to 2 | 1.4e-11 | exact | **guard**: soroswap (stale 07:53) |
| SCOP | 2 → filter cuts to 1 | 0 | exact | **population filter**: aquarius (25 candles, none priced) |
| USDCAllow | 1 | 0 | exact | none |

Tolerances, stated and justified: `volume_24h_usd` asserted **exactly equal**
(Decimal sum of Decimals — and it held, 0E-14 on all six); `vwap_24h` at
rel ≤ 1e-9 (the MV computes over Float64 arrays before the Decimal(38,14)
cast; ~15–16 significant digits); per-source `sources` values Decimal-exact;
`price_xlm` ratio ≤ 1e-9 with XLM itself asserted exactly 1.

**Finding 1 — cross-quote `argMaxIf` ties are common, not exotic.** 4 of 6
assets had ≥2 rows sharing the newest-priced timestamp (different quote legs,
same source, same minute), so "the latest priced close" is a **set**, and the
MV's pick among ties is non-contractual. The recompute therefore asserts
set-membership for prices and enumerates tie combinations for the vwap
(published matched: AQUA best-of-4 at 1.4e-11). First draft of the script
picked an arbitrary tie member and produced a false 3.0e-04 "mismatch" on
AQUA — the deviation a naive reconciliation would misreport as an MV bug.
Feeds the same non-determinism class that disqualified ETH at selection.

**Finding 2 — window-state drift between selection and capture.** At
selection (13:03) phoenix was stale on XLM/EURC (guard-dropped, `sources`
showed 3); by T=13:22 phoenix had fresh candles and re-entered — mask armed
over 4, exclusions empty. The guard arm is instead exercised by AQUA in this
run. Confirms exclusion sets are time-sensitive: any re-run must re-derive
the expected exclusions from the raw rows, never reuse a previous run's.

**Side product for [[0217]] / OUTLIER_PCT tuning:** per-venue deviations from
the unweighted median where the mask armed — XLM {sdex 0.0025%, soroswap
0.0025%, aquarius 0.088%, phoenix 0.246%}, EURC {aquarius 0.019%, sdex
0.019%, soroswap 0.111%, phoenix 0.824%}. Max observed 0.824% against the
20% band — ~24× headroom; the mask excluded nothing, consistent with
[[0135]]'s at-risk measurements.

## Acceptance Criteria

- [x] ≥3 multi-source assets reconciled, at least one with both SDEX and AMM
      sources in the window — **4 multi-source (XLM, EURC, BTC, AQUA), all
      mixing sdex + AMM venues**
- [x] The 24h window is pinned and recorded; the comparison is reproducible —
      **T = 2026-08-26 13:22:00, capture files + query committed**
- [x] Recomputation is independent of the MV's own SQL — **Python stdlib
      reimplementation of the contract; ties handled as sets, which the SQL
      cannot even express**
- [x] Weighted price matches within a stated, justified tolerance — **≤1.4e-11
      observed vs 1e-9 stated (Float64 rationale above)**
- [x] `volume_24h_usd` matches tightly (it is a plain sum) — **exactly, 0E-14
      on all six**
- [x] The **set of excluded sources** matches the threshold + outlier rules,
      asserted explicitly — **guard (AQUA/soroswap) and population filter
      (SCOP/aquarius) attributed separately; mask exclusions empty and
      asserted empty; `min_volume_usd` does not exist yet ([[0118]]), so no
      threshold exclusions are expected or found**
- [x] `sources` JSON per-source values reconcile — **keys and Decimal values
      exact on all six**
- [ ] End-to-end check through the public API confirms no precision loss in
      JSON serialization — **open: needs `API_KEY`/`BASE_URL` (same
      convention as the 0120 suite, `.env.local`); one asset, compare the
      string-serialised Decimals against the pinned CH row**
- [x] Method + results written up as citable evidence for [[0128]] — **this
      section + `benchmark/`; cite as run 1, T=13:22:00Z**

## Notes

- If a reconciliation fails, resist adjusting the tolerance. The 0114 precedent
  is that a failing gate is sometimes the wrong gate — but establish *which*
  before relaxing anything, and record the reasoning either way.
- Assets affected by [[0116]] (dust-trade candles producing absurd `close_usd`)
  make poor subjects; if one is unavoidable, note the interaction rather than
  silently excluding it.
