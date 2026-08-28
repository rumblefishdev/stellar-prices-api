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
      blocker is met; only [[0118]] remains.
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

## Design input — volume-weighted median (2026-08-27, from 0118 planning)

This task also owns the median mechanism itself (current.sql delegates the
even-count-interpolation question here; 0123's close landed the tie-set
question here too). Analysis from the 0118 kickoff discussion, recorded so
the decision is made with it on the table:

**The unweighted median is an attack surface.** Every venue votes equally, so
two dust venues can outvote and evict the deep market — and after eviction,
100% of the vwap weight lands on the manipulated pools. Cost of the attack is
moving two thin AMM pools >20%; 0118's $100 floor raises that bar only
slightly.

**Candidate fix: `quantileExactWeighted(0.5)(price, volume)`** — the
reference becomes "the price where half the volume sits". One function swap,
three properties:

1. Dust cannot steer the reference — the attack now requires controlling
   half the asset's 24h volume.
2. No interpolation pathology: the weighted quantile returns a *real* venue
   price, so the median-holder always has deviation 0 and survives — the
   `[1,1,3,3] → sources = '{}'` case becomes impossible by construction.
3. The >= 3 guard stops being load-bearing: at 2 sources the reference is
   the deeper venue, turning the test into "does the thin venue agree with
   the deep one?" — a defensible asymmetry (deep markets are expensive to
   manipulate).

Weights must be integer: `toUInt64(greatest(1, src_volume))` so a
zero-volume source still counts as 1.

**Deliberately NOT proposed: age-weighted votes.** Per-venue price age is
today ~100% an artifact of the hourly enrichment cadence, not a market
signal — folding it into vote weights would penalise venues for our own
pipeline lag, re-introducing the defect 0135's carry removed, and would make
the AC-4 hand-reconciliation (0123) substantially harder. Age is already
handled at the right layers: the 2h conditional liveness guard, carry, and
[[0216]]'s age column. The one residual gap (a >20% move inside 2h where the
lagged majority evicts the first mover) is unobserved in practice — 0123
measured max deviation 0.824% against the 20% band.

**Also considered and rejected: evaluating every venue at a common
timestamp** (snapshot-at-T, consolidated-tape style — take the laggard
venue's newest priced minute and read every other venue as of that minute).
Rejected 2026-08-27, four reasons: (1) a candle at the common T usually does
not exist — candles are per traded minute, so sparse venues get an
ASOF-backward read anyway and the skew merely moves its anchor into the
past; (2) the hourly enrichment pass already synchronises priced tips to the
pass boundary — residual cross-venue skew is trading sparsity, which no
timestamp choice fixes; (3) it inverts failure isolation: today a lagging
venue degrades only its own vote, synchronised it would pin the whole
asset's freshness to the worst venue; (4) the defect it would close needs a
>20% move inside a <=2h skew window — unobserved (0123: max deviation
0.824%). The residual concern (a fresh first-mover judged against a carried
majority) is better served by the volume-weighted median above plus
[[0216]]'s explicit age column.

**Cost of any semantic change:** AC 4 (Tranche 2) was evidenced on the
current semantics — changing the median invalidates that evidence, so the
change requires a 0123-style re-run plus `current_mv_it.rs` fixture updates,
and must be called out as a published-value change.

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
