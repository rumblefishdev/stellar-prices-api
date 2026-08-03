---
id: "0135"
title: "Decide whether the headline price_usd should be outlier-protected like vwap_24h"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0072", "0118", "0123"]
tags: ["phase-future", "effort-medium", "priority-medium", "milestone-M2", "vwap", "clickhouse"]
milestone: 2
links: []
history:
  - date: 2026-07-29
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0072]] future work — flagged in PR #150 as a known gap,
      deliberately not fixed there ("worth its own decision"). 0072 added the
      §5.5 median-outlier filter to `vwap_24h` only.
  - date: 2026-07-30
    status: backlog
    who: okarcz
    note: >
      Scope widened by a second, measured failure mode on the same column. The
      0072 prod rollout showed 21 of 3,022 assets publishing `price_usd = 0`
      while `vwap_24h` and `sources` carry a real price — `price_usd` is
      `argMax(close_usd, timestamp)` with no `close_usd > 0` filter, so an
      un-enriched newest candle zeroes it (and `market_cap_usd` and `price_xlm`
      with it). Pre-existing, not a 0072 regression. Both failure modes are the
      same column and should be decided together.
---

# price_usd is not outlier-protected

## Summary

Task 0072 added the general-overview §5.5 inter-source median-outlier filter to
`vwap_24h`, but `price_usd` — the **headline** number, the one
`GET /assets/{id}/price` leads with and the one `market_cap_usd` multiplies —
remains `argMax(close_usd, timestamp)` across all sources unfiltered. So the
published price can come from a single manipulated or malfunctioning venue while
the VWAP sitting next to it is protected.

0072's own fixture pins the asymmetry concretely: an asset returns
`price_usd: 5.00` alongside `vwap_24h: 1.007`, where 5.00 is the excluded
outlier.

## Context

§7 scopes outlier detection to the VWAP, and `price_usd` is already-live
behaviour, so 0072 declined to change it silently. But "the spec only asked for
the VWAP" is a reason to raise the question, not to settle it — a caller reading
the headline price has no signal that it was not sanity-checked.

Note this compounds with `market_cap_usd`, which is `price_usd × supply`: an
outlier price propagates into market cap, where the error is multiplied by a
potentially very large supply figure.

## Second failure mode — the un-enriched tip zeroes `price_usd` (measured in prod)

Added 2026-07-30 from the [[0072]] rollout. Same column, different mechanism, and
worth fixing in the same pass.

`unfiltered.price_usd` is `argMax(close_usd, timestamp)` with **no
`close_usd > 0` filter** (`current.sql:203`), while the per-source CTE that feeds
`sources` and `vwap_24h` filters `src_price > 0`. Enrichment is a separate,
lagging pass, so whenever an asset's newest `price_ohlcv_1m` candle has not been
enriched yet, `price_usd` reads **0** while the other columns carry a perfectly
good price.

Measured on ch-prod-01, 2026-07-30, over 3,022 assets:

```
zero_price_usd:    777   <- of which 756 are genuinely unpriced (vwap also 0) — correct
zero_but_vwap_ok:   21   <- publishing 0 while demonstrably knowing the price
```

Live example, `asset_id 4`: `price_usd 0`, `vwap_24h 0.17281880272617`,
`sources {"soroswap":{"price":"0.17281880272617", …}}`.

**Pre-existing, not a 0072 regression** — the v1 MV used the identical unfiltered
`argMax`. 0072 only made it visible by publishing a real VWAP beside the zero.

Consequences beyond the headline number: `market_cap_usd` becomes 0 for those
assets (0 × supply), and `price_xlm` becomes 0 too (it divides `price_usd`). The
same asymmetry falsified a check in the 0072 runbook, which asserted XLM's own
`price_xlm` must equal exactly 1 — an un-enriched XLM tip makes it 0.

The obvious fix (`argMaxIf(close_usd, timestamp, close_usd > 0)`) is a one-line
change, but it silently redefines "current price" as "most recent *enriched*
price, however old", so it needs the staleness question answered alongside —
which is why it belongs with the decision below rather than as a drive-by patch.

## Implementation

Decide between, roughly:

- **Leave as-is, document.** `price_usd` is "last trade, as observed"; callers
  wanting a de-noised figure use `vwap_24h`. Cheapest; needs the API docs to say
  so plainly, since today nothing distinguishes them.
- **Filter `price_usd` through the same keep-mask.** Consistent with the VWAP,
  but changes a live, already-published number, and the >= 3-source guard means
  most assets are unaffected anyway.
- **Publish a confidence/divergence signal** — e.g. how far `price_usd` sits
  from the inter-source median — and leave both numbers untouched. Most
  informative, largest API-surface change.

Sequence after [[0118]] (the `min_volume_usd` threshold), which changes which
sources reach the median in the first place and so changes what "outlier" means
here. [[0123]]'s reconciliation against real multi-source assets is the evidence
base for whichever option is chosen.

## Acceptance Criteria

- [ ] Decision recorded (ADR or task note) with the reasoning, including the
      `market_cap_usd` propagation.
- [ ] If filtered: `current.sql` updated, `current_mv_it.rs` asserts the new
      behaviour, and the change is called out as a published-value change.
- [ ] If left as-is: the §4.2 `/price` docs state that `price_usd` is unfiltered
      and `vwap_24h` is the de-noised figure.
- [ ] The un-enriched-tip zero is resolved: either `price_usd` skips unpriced
      candles, or the 0-vs-genuine-price ambiguity is documented and given a
      staleness bound. `zero_but_vwap_ok` should be 0 afterwards.
- [ ] `market_cap_usd` and `price_xlm` no longer collapse to 0 purely because the
      newest candle is un-enriched.
