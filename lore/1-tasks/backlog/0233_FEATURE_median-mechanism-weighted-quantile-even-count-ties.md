---
id: "0233"
title: "The §5.5 median mechanism itself — volume-weighted quantile, even-count interpolation, and argMaxIf tie sets"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0217", "0123", "0072", "0118", "0135", "0128"]
tags: [layer-database, priority-medium, effort-medium, milestone-M3, vwap, clickhouse, materialized-view, security]
milestone: 3
links:
  - "../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../packages/prices-clickhouse/tests/current_mv_it.rs"
history:
  - date: 2026-08-28
    status: backlog
    who: stkrolikiewicz
    note: >
      Split out of [[0217]]. Three separate artifacts had routed median
      questions there — `current.sql` delegates even-count interpolation,
      [[0123]]'s close landed the tie-set question, and [[0118]]'s kickoff
      added the weighted-median analysis — while 0217's own title, options
      and all four acceptance criteria cover only `price_usd`'s outlier
      protection. 0217 could be closed in full without touching `median()`.
      Splitting keeps that cheap product decision unblocked by this expensive
      published-value change.
---

# The median mechanism itself

## Summary

`vwap_24h`'s §5.5 outlier filter compares each venue against an **unweighted**
inter-source median (`arrayReduce('median', prices_f)`, i.e. `quantile(0.5)`
with interpolation). Three known problems live in that one expression. This
task owns all three, because any fix to one changes the others.

## Problem 1 — one venue, one vote is a manipulation surface

Every venue's price counts equally regardless of its 24h volume, so two dust
venues can outvote and evict the deep market — and once the deep venue is
evicted, **100% of the vwap weight lands on the manipulated pools**.

Pinned today by fixture 18 ATK in `current_mv_it.rs`: sdex quotes 1.00 on
$500,000, soroswap 1.35 on $40, phoenix 1.38 on $25. Unweighted, the median
is 1.35, sdex deviates 25.9% > OUTLIER_PCT and is the one excluded; the row
publishes a `vwap_24h` of ~1.3615 drawn from $65 of turnover beside a
`volume_24h_usd` of $500,065. [[0118]]'s $100 floor raises the bar only to
"$101 per dust venue", which on-chain costs fees.

## Problem 2 — interpolation can clear every source

`median` interpolates on an even count, producing a reference nobody quoted.
Verified on prod 26.3.10.60 and pinned by fixture 15 EVN:
`[1.00, 1.00, 3.00, 3.00] → median 2.00 → every element deviates 50% →
sources = '{}'`, `vwap_24h = 0`, while `price_usd` still publishes 1.00.
`current.sql` explicitly defers this question here.

## Problem 3 — the reference price is a tie SET, not a value

[[0123]] measured that **4 of 6** reconciled assets had two or more rows
sharing the newest priced timestamp (different quote legs, same source, same
minute), so `argMaxIf`'s pick among ties is non-contractual. A naive
reconciliation that picks an arbitrary tie member mis-reports a 3.0e-04
deviation as an MV bug; the same non-determinism disqualified ETH from
0123's subject list.

## Candidate: `quantileExactWeighted(0.5)(price, weight)`

One function swap that addresses problems 1 and 2 together. The reference
becomes "the price at which half the traded volume sits":

1. **Dust cannot steer the reference.** The attack cost moves from "shift two
   thin pools" to "control half the asset's 24h volume" — proportional to
   liquidity, which is the right shape. Not an absolute defence: wash trading
   on a thin asset is still cheap, so this raises the bar rather than closing
   the hole.
2. **No interpolation pathology.** `quantileExactWeighted` never interpolates
   — it returns a real element of the input, so the median-holder always has
   deviation 0 and survives. Clearing the whole set becomes impossible by
   construction. (ClickHouse's `quantileInterpolatedWeighted` is the variant
   we would NOT want.)
3. **The `>= 3` guard stops being load-bearing.** At two sources the
   reference is the deeper venue, so the test becomes "does the thin venue
   agree with the deep one?" — a defensible asymmetry rather than the
   current keep-both-or-drop-both no-op.

Details to settle: weights must be an unsigned integer type, so
`toUInt64(greatest(1, src_volume))` — which floors every sub-dollar venue at
weight 1, i.e. dust falls back to one-vote-per-venue among itself. At an
exact 50% boundary which of the two neighbours is returned is an
implementation detail; it is always a real venue price, but a reconciliation
must treat it as a set, the way [[0123]] treats `argMaxIf` ties.

## Deliberately NOT proposed: age-weighted votes

Per-venue price age is today ~100% an artifact of the hourly enrichment
cadence, not a market signal. Folding it into vote weights would penalise
venues for our own pipeline lag — re-introducing the defect [[0135]]'s carry
removed — and would make the AC-4 hand-reconciliation substantially harder.
Age is already handled at the right layers: the 2h conditional liveness
guard, carry, and [[0216]]'s age column. The residual gap (a >20% move inside
2h where a lagged majority evicts the first mover) is unobserved: 0123
measured max deviation **0.824%** against the 20% band.

## Also considered and rejected: a common evaluation timestamp

Snapshot-at-T / consolidated-tape style — take the laggard venue's newest
priced minute and read every venue as of that minute. Rejected 2026-08-27 for
four reasons: (1) a candle at the common T usually does not exist, since
candles are per traded minute, so sparse venues get an ASOF-backward read
anyway and the skew merely moves its anchor into the past; (2) the hourly
enrichment pass already synchronises priced tips to the pass boundary —
residual cross-venue skew is trading sparsity, which no timestamp choice
fixes; (3) it inverts failure isolation: today a lagging venue degrades only
its own vote, synchronised it would pin the whole asset's freshness to the
worst venue; (4) the defect it closes needs a >20% move inside a <=2h window,
unobserved per the measurement above.

## Tuning input already in hand

[[0123]] measured per-venue deviation from the unweighted median where the
mask armed: XLM {sdex 0.0025%, soroswap 0.0025%, aquarius 0.088%, phoenix
0.246%}, EURC {aquarius 0.019%, sdex 0.019%, soroswap 0.111%, phoenix
0.824%}. Max observed **0.824%** against the 20% band — ~24x headroom, and
the mask excluded nothing on live data. OUTLIER_PCT retuning belongs here
too, on that evidence.

## Cost, and why this is not free

Tranche 2 AC 4 ("VWAP verifiable against raw `price_ohlcv` rows") was
evidenced on the **current** semantics. Changing the median invalidates that
evidence, so this task must budget for:

- a 0123-style reconciliation re-run against the new mechanism, since
  `benchmark/reconcile.py` reimplements the contract independently;
- `current_mv_it.rs` fixture updates — 15 EVN and 18 ATK both assert the
  current behaviour and would flip;
- a **published-value change** callout: `vwap_24h` and `sources` move for
  real assets, and [[0128]] cites the old run.

## Acceptance Criteria

- [ ] Decision recorded for each of the three problems, with reasoning —
      weighted vs unweighted median, even-count behaviour, tie handling
- [ ] If the mechanism changes: `current.sql` updated, `current_mv_it.rs`
      fixtures 15 and 18 assert the new behaviour, and the `>= 3` guard's
      fate is stated explicitly
- [ ] OUTLIER_PCT either retuned on 0123's measured distribution or
      explicitly left at 0.20 with the reasoning
- [ ] The change is called out as a published-value change, with a
      reconciliation re-run so [[0128]] cites evidence that matches what
      production serves
- [ ] `current.sql`'s comments stop delegating these questions elsewhere —
      they either describe the shipped decision or cite this task by id

## Notes

- Sequencing: needs [[0118]] merged (the threshold changes which venues reach
  the vote). Independent of [[0217]] — that task decides whether `price_usd`
  is filtered at all; this one decides what the filter *is*. If 0217 chooses
  to filter `price_usd`, it inherits whatever mechanism this task settles.
