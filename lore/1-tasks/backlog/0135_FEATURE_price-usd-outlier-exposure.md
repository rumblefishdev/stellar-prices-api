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

### It also propagates into `change_24h_pct` / `change_7d_pct` (found 2026-08-03)

Surfaced by the [[0138]] PR #160 review, and sharper than the `market_cap_usd`
case because the resulting row is **self-contradictory on its face**.

`change_*_pct` reads `price_usd` from the `unfiltered` CTE, so an outlier venue
holding the newest candle becomes the numerator — while the `sources` object
published in the same row **excludes that venue** and `vwap_24h` is computed
without it. `current_mv_it.rs`'s existing fixture demonstrates it exactly: asset
3's `aquarius` leg at 5.00 is asserted absent from `sources` and excluded from
`vwap_24h` (1.0067), yet `change_24h_pct` is asserted at **+525%** against it,
beside a sources object showing only ~1.00/1.02.

This is the same failure class 0138 fixed for zero prices — a confident wrong
answer that passes every `0`-means-unavailable consumer guard — reached by a
different route. **0138's `nullIf` does not address it**; only outlier-filtering
`price_usd` does, which is this task.

Whichever option below is chosen must therefore also state what happens to the
two change columns. "Leave as-is, document" is a materially weaker answer here
than it is for `market_cap_usd`: a caller can reasonably be told "the headline
price is unfiltered, use the VWAP", but there is no filtered alternative to
`change_24h_pct` to point them at.

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
      `market_cap_usd` propagation **and the `change_24h_pct`/`change_7d_pct`
      propagation** (see above — the latter yields rows that contradict their own
      `sources` field).
- [ ] If filtered: `current.sql` updated, `current_mv_it.rs` asserts the new
      behaviour, and the change is called out as a published-value change.
- [ ] If left as-is: the §4.2 `/price` docs state that `price_usd` is unfiltered
      and `vwap_24h` is the de-noised figure.
