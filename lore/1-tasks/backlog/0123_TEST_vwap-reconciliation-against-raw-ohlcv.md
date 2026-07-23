---
id: "0123"
title: "VWAP reconciliation — current_prices verifiable against raw price_ohlcv rows for ≥3 assets"
type: TEST
status: backlog
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
