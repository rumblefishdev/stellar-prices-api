---
id: "0217"
title: "Decide whether the headline price_usd should be outlier-protected like vwap_24h — the question 0135 was opened for and could not answer"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0135", "0118", "0123", "0072", "0138", "0216"]
tags: [layer-backend, priority-medium, effort-medium, milestone-M3, vwap, clickhouse, read-surface]
milestone: 3
links:
  - "../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../packages/prices-clickhouse/tests/current_mv_it.rs"
history:
  - date: 2026-08-20
    status: backlog
    who: stkrolikiewicz
    note: >
      Split out of [[0135]] on close. 0135 grew three independent failure
      modes on one column; two were fixed and shipped (the un-enriched-tip
      zero, and C2's enrichment-timing-dependent `sources` membership), while
      this one — the question in 0135's own title — was never touched. It is
      split rather than carried because it is **blocked by sequencing 0135
      itself recorded**: [[0118]] changes which sources reach the median and
      therefore what "outlier" means, and [[0123]] is the evidence base for
      whichever option is chosen. Both are in backlog, so 0135 would have
      stayed open indefinitely on a question it could not answer.
---

# Is the headline price outlier-protected, or not?

## Summary

Task 0072 applied the general-overview §5.5 inter-source median-outlier filter
to `vwap_24h` only. `price_usd` — the number `GET /assets/{id}/price` leads
with, and the one `market_cap_usd` multiplies — is the newest priced close
**regardless of venue**, so it can come from a venue the filter rejected.

§7 scopes outlier detection to the VWAP and `price_usd` is already-live
behaviour, which is why 0072 declined to change it silently. But "the spec
only asked for the VWAP" is a reason to raise the question, not to settle it.

## The shape of the problem, in one row

`current_mv_it.rs`'s asset-3 fixture pins it exactly, and is asserted today as
*pinned-but-not-endorsed*: the `aquarius` leg at 5.00 is excluded from
`vwap_24h` (1.0067) and absent from `sources`, yet it is the newest priced
candle, so `argMaxIf` makes it `price_usd` — and the row publishes
**+525% on `change_24h_pct`** beside a `sources` object showing only ~1.00/1.02.

That is a row which **contradicts its own `sources` field**. It is the same
class of defect [[0138]] fixed for zero prices — a confident wrong answer that
passes every `0`-means-unavailable consumer guard — reached by a different
route. Unlike a zero, an outlier price is a perfectly ordinary-looking number,
so no consumer guard catches it.

## Why this is sharper than it looks

- **`market_cap_usd`** is `price_usd × supply`, so an outlier price is
  multiplied by a potentially very large supply figure.
- **`change_24h_pct` / `change_7d_pct`** read `price_usd`, so an outlier venue
  holding the newest priced candle becomes the numerator. And here the usual
  fallback answer is weaker than elsewhere: a caller can reasonably be told
  "the headline price is unfiltered, use `vwap_24h`" — but there is **no
  filtered alternative to `change_24h_pct`** to point them at.
- [[0135]] made this population *more* reachable, not less. Before it,
  `price_usd` was an unguarded `argMax` that frequently landed on an
  un-enriched 0; those rows were loudly broken instead of quietly wrong. Now
  the column skips to the newest *priced* close — which, for an asset whose
  dominant venue has an un-enriched tip, may be exactly the thin outlier
  venue the mask rejected. The 0138 sentinel that used to hide it is gone.

## Options

- **Leave as-is, document.** `price_usd` is "last trade, as observed"; callers
  wanting a de-noised figure use `vwap_24h`. Cheapest, and §4.2 already states
  the asymmetry (shipped by 0135). Must still answer what happens to the two
  change columns.
- **Filter `price_usd` through the same keep-mask.** Consistent with the VWAP,
  but changes a live published number — and the ≥3-source guard means most
  assets are unaffected anyway, so measure the blast radius before assuming
  it is large.
- **Publish a confidence/divergence signal** — e.g. how far `price_usd` sits
  from the inter-source median — and leave both numbers untouched. Most
  informative, largest API-surface change. Pairs naturally with [[0216]]'s
  age column: both answer "how much should I trust this number?".

## Sequencing (why this is not startable today)

- **After [[0118]]** — the §5.5 `min_volume_usd` threshold drops cheap sources
  *before* the median is taken, so it changes which venues reach the vote and
  therefore what counts as an outlier at all. Deciding first would be deciding
  against a population that is about to change.
- **[[0123]]** — VWAP reconciliation against real multi-source assets is the
  evidence base for whichever option is chosen. Without it, any threshold
  argument is a guess.

## Acceptance Criteria

- [ ] Decision recorded (ADR or task note) with the reasoning, explicitly
      covering the `market_cap_usd` propagation **and** the
      `change_24h_pct`/`change_7d_pct` propagation
- [ ] If filtered: `current.sql` updated, `current_mv_it.rs` asserts the new
      behaviour, and the change is called out as a **published-value change**
- [ ] If left as-is: §4.2 already says `price_usd` is unfiltered (0135) — the
      remaining gap is the change columns, which must be documented too
- [ ] The asset-3 fixture stops being "pinned but not endorsed" — it either
      asserts the new behaviour, or carries the recorded decision by ID
