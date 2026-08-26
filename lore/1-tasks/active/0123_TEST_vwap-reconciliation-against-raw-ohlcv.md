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

## Acceptance Criteria

- [ ] ≥3 multi-source assets reconciled, at least one with both SDEX and AMM
      sources in the window
- [ ] The 24h window is pinned and recorded; the comparison is reproducible
- [ ] Recomputation is independent of the MV's own SQL
- [ ] Weighted price matches within a stated, justified tolerance
- [ ] `volume_24h_usd` matches tightly (it is a plain sum)
- [ ] The **set of excluded sources** matches the threshold + outlier rules,
      asserted explicitly
- [ ] `sources` JSON per-source values reconcile
- [ ] End-to-end check through the public API confirms no precision loss in
      JSON serialization
- [ ] Method + results written up as citable evidence for [[0128]]

## Notes

- If a reconciliation fails, resist adjusting the tolerance. The 0114 precedent
  is that a failing gate is sometimes the wrong gate — but establish *which*
  before relaxing anything, and record the reasoning either way.
- Assets affected by [[0116]] (dust-trade candles producing absurd `close_usd`)
  make poor subjects; if one is unavoidable, note the interaction rather than
  silently excluding it.
