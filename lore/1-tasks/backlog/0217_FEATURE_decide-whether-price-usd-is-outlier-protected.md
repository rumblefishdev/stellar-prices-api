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
  - date: 2026-08-27
    status: backlog
    who: stkrolikiewicz
    note: >
      Design input recorded from the 0118 kickoff: the unweighted median is a
      cheap manipulation surface (dust venues can evict the deep market);
      candidate fix is a volume-weighted median (quantileExactWeighted), which
      also kills the even-count interpolation pathology by construction.
      Age-weighted votes considered and argued against — age is pipeline
      artifact, handled by the liveness guard + [[0216]]. See the new
      "Design input" section. [[0123]] is now completed, so the evidence-base
      blocker is met; only [[0118]] remains. **That section has since moved to
      [[0233]] — see the 2026-08-28 split entry.**
  - date: 2026-08-28
    status: backlog
    who: stkrolikiewicz
    note: >
      Read the original SCF RFP against this question for the first time. It
      never defines how Current Price is computed, but its only stated
      aggregation rule is the weighted average across markets — so the
      natural reading is that the headline price IS the aggregate, which is
      the opposite of what we ship. Recorded as a second "Design input"
      section, with its strength stated honestly (an inference from two
      bullets sitting together, not a quotation). Raises the bar for the
      leave-as-is option: that choice now owes [[0128]] a written reason.
  - date: 2026-08-28
    status: backlog
    who: stkrolikiewicz
    note: >
      Split: the §5.5 median MECHANISM (weighted quantile, even-count
      interpolation, argMaxIf tie sets) moved to [[0233]]. Three artifacts had
      routed median questions here while this task's title, options and all
      four ACs cover only `price_usd` — it could have been closed in full
      without touching `median()`. Keeping the cheap product decision
      unblocked by an expensive published-value change.
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

## The median mechanism is NOT this task ([[0233]])

Three questions about the §5.5 median itself were routed here by other
artifacts — `current.sql`'s even-count-interpolation note, [[0123]]'s
tie-set finding, and [[0118]]'s kickoff analysis of the unweighted median as
a manipulation surface. They are **split out into [[0233]]** as of
2026-08-28, because this task's title, options and acceptance criteria cover
only whether `price_usd` is outlier-protected: 0217 could be closed in full
without touching `median()`, which made it a magnet for questions it did not
own.

The split matters for sequencing: deciding `price_usd` is a cheap product
call, while replacing the median is a published-value change that needs a
0123-style reconciliation re-run. Binding them together would block the cheap
decision behind the expensive one. If this task chooses to filter `price_usd`,
it inherits whatever mechanism 0233 settles.

## Design input — what the RFP actually asks of the headline price (2026-08-28)

The original SCF RFP (`RFP 1: Prices API`) was read against this question for
the first time on 2026-08-28. It bears on the decision more than expected,
and in one direction.

**The RFP never defines how the current price is computed.** Its entire
statement of the field is one line under Asset Metadata Required:
*"Current Price (float USD)"*. No "last trade", no "latest close", no
freshness or venue rule. So none of the three options here is constrained by
the RFP on its own terms — including the option to leave it as-is.

**But the RFP's only stated aggregation rule is the weighted average.** Core
Requirements list *"Price Aggregation: Weighted average across major markets
(Soroswap, Aquarius, SDEX, Blend)"* as the method for producing normalized
prices, and the same list names *"Current Price"* as the field a consumer
reads. The natural reading of the pair is that the price a consumer sees IS
the aggregate. Our shipped design does the opposite: the headline `price_usd`
is the unfiltered latest priced close from a single venue, and the aggregate
lives in the secondary `vwap_24h`.

**Weight this honestly.** This is an inference from how the two bullets sit
together, not a quotation — the RFP does not say "Current Price = the
weighted average", and a reviewer may never join them. It does not settle the
task. What it does is change the stakes of the asset-3 shape from an internal
consistency nit to a plausible "does the headline number meet the Core
Requirement" question, asked by someone reading the RFP rather than our code.
That argues against the cheapest option (leave as-is, document) carrying the
decision by default: if `price_usd` stays unaggregated, the reasoning for why
the headline is deliberately *not* the §5.5 aggregate belongs in the
[[0128]] evidence package, not only in this task.

Related but out of scope here: the RFP types the field as a **float** while
§3.3 serialises it as a string to preserve `Decimal(38,14)` (0123 measured
prod prices at 7e-8, which a JSON float would destroy). That deviation needs
a recorded answer for the evidence package; it is a serialization question,
not an outlier-protection one, and is owned by [[0232]].

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
