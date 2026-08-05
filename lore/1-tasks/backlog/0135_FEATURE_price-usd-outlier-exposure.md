---
id: "0135"
title: "Decide whether the headline price_usd should be outlier-protected like vwap_24h"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0072", "0118", "0123", "0144", "0145", "0146", "0147"]
tags: ["phase-future", "effort-large", "priority-high", "milestone-M2", "vwap", "clickhouse"]
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
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Raised to priority-high and scope widened again from [[0144]]'s BE 0199
      triage. BE measured the un-enriched-tip zero **on native XLM itself** —
      the worst possible asset for it, and close to chronic rather than
      intermittent. [[0144]] also found a **third** unguarded site in the same
      file (`current.sql:121`, `per_source.src_price`) whose downstream
      `WHERE src_price > 0` rescue makes `sources` and `vwap_24h`
      enrichment-timing-dependent. This task remains the owner of finding 1;
      no duplicate was spawned.
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

> **How this relates to the outlier propagation above, and to [[0138]].** These
> are two distinct mechanisms that both corrupt `price_usd` — an *outlier* venue
> holding the newest candle, and an *un-enriched* newest candle reading 0 — and
> both then propagate into `market_cap_usd`, `price_xlm` and the change columns.
>
> **0138 has closed only the change-column consequence of the ZERO case**, by
> guarding the numerator so a zero `price_usd` yields the `0` = "no signal"
> sentinel instead of a fabricated `-100`. It measured **817 of 4,165 assets
> (19.6%)** affected on ch-prod-01, 2026-08-03.
>
> Everything else here is still open: `price_usd` itself is corrupted in both
> cases, `market_cap_usd` and `price_xlm` still inherit it, and the **outlier**
> case still produces a wrong `change_*_pct` that no guard catches — because
> unlike a zero, an outlier price is a perfectly ordinary-looking number.
> **This task owns all of that.**

## Third failure mode — the `src_price > 0` rescue is itself a population shift

Found 2026-08-05 in [[0144]]'s full-schema audit (its scope correction **C2**),
and it complicates the framing above.

The text above treats `per_source`'s `src_price > 0` filter as the *correct*
counterpart to `unfiltered`'s missing guard. It is not — it is the same defect
one step later. `per_source.src_price` is **also** an unguarded
`argMax(close_usd, timestamp)` (`current.sql:121`); the zero is then dropped by
`WHERE src_price > 0` at `current.sql:140`. That fixes the arithmetic by
**silently changing the population**, which is exactly the mechanism [[0144]]
filed as BE 0199 finding 3i against `price_usd_series*`.

Consequence, not previously recorded here:

- **A source whose newest 1m candle is un-enriched disappears entirely from the
  `sources` JSON and from the `vwap_24h` weighting** — not because it is an
  outlier, but because enrichment has not reached it yet. A consumer cannot
  distinguish the two.
- The median filter's documented "**no-op below 3 sources by construction, not
  by luck**" property (`current.sql:72-76`) is then evaluated against that
  shrunken population. A three-source asset with one lagging source is silently
  a two-source asset for that refresh, so the safety argument for the filter is
  itself enrichment-timing-dependent.

This means the decision below cannot be made for `price_usd` alone. Whichever
option is chosen must also state **whether an unpriced source should be absent
from `sources` / `vwap_24h`, or carried at its last known price** — and BE will
hit this next, having already switched to a `price_usd_series_1h` workaround.

[[0147]] is settling the same question ("what counts as priced enough") for the
read surfaces; the answers should agree.

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
- [ ] The un-enriched-tip zero is resolved: either `price_usd` skips unpriced
      candles, or the 0-vs-genuine-price ambiguity is documented and given a
      staleness bound. `zero_but_vwap_ok` should be 0 afterwards.
- [ ] `market_cap_usd` and `price_xlm` no longer collapse to 0 purely because the
      newest candle is un-enriched.
- [ ] Native **XLM** specifically publishes a real `price_usd` — the case BE
      measured and the one [[0144]] shows is close to chronic, since XLM's
      newest candle is both usually newer than the last enrichment pass and
      often an exotic-quote pair that will never be enriched at all.
- [ ] The C2 question answered: an unpriced source is either absent from
      `sources`/`vwap_24h` *by an explicit rule*, or carried at its last known
      price — not dropped as a side effect of enrichment timing. Answer
      consistent with [[0147]]'s coverage gate.
