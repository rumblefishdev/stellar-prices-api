---
id: "0135"
title: "Decide whether the headline price_usd should be outlier-protected like vwap_24h"
type: FEATURE
status: active
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
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      **Contract decided.** [[0144]] phase 0 measured the trade on prod and the
      call is made: `price_usd` publishes the **latest *priced* close** —
      `argMaxIf(close_usd, timestamp, close_usd > 0)` in `mv_current_prices`.
      Cost is up to ~50 minutes of staleness (~25 avg, measured from one full
      enrichment cycle) against today's permanent zero; native XLM currently
      publishes 0 essentially always, while the guarded aggregate returns
      0.16720799309045. Rejected: keeping the zero, and absent/`NULL` (correct
      semantically but needs Nullable or a status column — that belongs to
      [[0151]], not here). **Scope correction C2 decided the same way**: guard
      `per_source.src_price` too, so a venue whose newest candle is un-enriched
      carries its latest priced close and stays in `sources` and in the
      `vwap_24h` weighting, instead of silently vanishing. Both are one-line
      changes in `current.sql`, which self-DROPs, so delivery is unblocked.
  - date: 2026-08-19
    status: backlog
    who: stkrolikiewicz
    note: >
      Fresh production evidence from the [[0120]] conformance run, which
      independently rediscovered both failure modes before dedup: 17 of the
      20 fixed major assets publish `price_usd = 0` (88 of the top-200
      volume rows — well past 07-30's 21-of-3,022), and 12 of the 20 hit
      the C2 limit case — every source un-enriched at once, so
      `sources: {}` and `vwap_24h = 0` beside an unfiltered
      `volume_24h_usd > 0` (AQUA and BTC among them). The observed
      "price>0 ⟺ sdex present" split is [[0154]]'s quote-restriction seen
      from the read side: sdex candles are stable-quoted and always enrich,
      AMM candles often never do. Spawned-then-retired duplicates
      from that run fold into this task; the 0120 suite
      (`npm run conformance:0120`) is the regression gate — its
      price-sentinel checks must go green when the decided contract ships.
  - date: 2026-08-20
    status: backlog
    who: stkrolikiewicz
    note: >
      Two [[0120]] runs 16 minutes apart turn the previous day's static
      counts into a much stronger signal: this task's three failure classes
      **churn in both directions** while the structural ones do not. Between
      07:05 and 07:21 UTC the `sources`-empty set lost BTC, CBIJ, USDCAllow,
      AUD and yUSDC but gained XRP (12 → 8); `price_usd = 0` went 18 → 17 →
      13 across three runs. Over the same three runs [[0170]]'s empty-OHLCV
      count held at exactly 12 and [[0178]]'s USDC 404 at exactly 1, same
      assets every time. A per-asset data defect cannot move like that — the
      churn is the enrichment-timing dependence of the third failure mode
      (`current.sql:121` + `:140`), reproduced across many assets rather
      than the single `native` flicker the 0072 runbook recorded. Practical
      consequence for whoever ships the decided `argMaxIf` contract: a
      before/after comparison must be taken from runs minutes apart, not
      days, or normal churn will swamp the effect.
  - date: 2026-08-20
    status: active
    who: stkrolikiewicz
    note: >
      Promoted to active and the decided contract implemented:
      `argMaxIf(close_usd, timestamp, close_usd > 0)` on both
      `unfiltered.price_usd` and `per_source.src_price`; the
      `src_price > 0` filter is now the explicit "no priced candle in the
      whole window" rule rather than an enrichment-timing side effect. §4.2
      documents the latest-priced-close semantics. `current_mv_it.rs`
      fixture 7 rewritten from pinning the bug to asserting the contract
      (price_usd 1.90, change -5%/+90%, vwap weights both venues,
      price_xlm 3.8) — all 3 tests green on the 26.3.10.60 pin. The
      [[0120]] conformance suite is the regression gate — its price-sentinel
      and empty-`sources` classes are this task's failure modes (cite the
      classes, not run counts: they churn between runs). STILL OPEN before
      completion: the failure-mode-1 decision (outlier-filter `price_usd`
      vs leave-and-document vs confidence signal) with its change_*_pct
      propagation, and post-deploy verification (zero_but_vwap_ok = 0,
      XLM publishes a real price, 0120 price checks green).
  - date: 2026-08-20
    status: active
    who: stkrolikiewicz
    note: >
      Multi-agent review of PR #228 found the first cut of the contract was
      unsafe as written; reworked. **The carry is now BOUNDED (2 h)** at both
      sites: an unbounded carry let a chronically-unpriced venue keep voting
      in the §5.5 median — the review demonstrated a 3-source case where the
      only LIVE venue is evicted by two stale ones, turning a correct row
      into a wrong one — and let `price_usd` certify a close up to 24 h old
      while `updated_at` (refresh time, no age signal) read fresh. Also
      fixed: `ref_7d` now reads the [7d, 5d] band with `argMinIf` (it was
      taking the oldest close AVAILABLE, so a freshly-listed asset published
      a 2-hour move labelled 7-day — a defect the old zero sentinel had been
      masking for ~396 assets); `xlm_usd` converted to the same `argMaxIf`
      idiom. Coverage added for the shapes that had none: over-bound asset
      (proves the change_* numerator guards are load-bearing, incl. the only
      test of the 7d one), single-source carry (the XLM AC shape), 3-source
      mask arming over a carried price, and `market_cap_usd` against a
      seeded supply. New static lint
      `current_sql_uses_no_unguarded_argmax_on_close_usd` gives the contract
      CI enforcement (the CH tests are `#[ignore]`d and never run there).
      Contracts synced: `views.sql` sentinel table, the 0072 rollout runbook
      (its post-deploy guidance was inverted by this change), `dto.rs`
      doc-comments (the published OpenAPI descriptions) and §4.2.
  - date: 2026-08-20
    status: active
    who: stkrolikiewicz
    note: >
      **Bound removed from `price_usd`; it survives only in `per_source`.**
      Settled with okarcz, who accepted the shape and corrected the reasoning
      — the correction is the important part of this entry.
      **(a) My premise was wrong.** I argued a bound would blank the very
      population 0135 rescues, inferring it from 0111's backlog figures.
      Enrichment has TWO stages: the oracle stage handles recent candles and
      keeps up (105–213k rows/day); the pivot stage grinds history
      oldest-first and is the one drowning. 0111's note described the second
      and read as the whole. Re-measured 2026-08-20: the current window holds
      **2,810** unpriced XLM-quoted candles against 388M in 2023, and the
      657M figure is whole-table (646M is 0088 backfill history; the current
      window's 10.4M are exotic legs with no pricing path, sentinel under any
      policy). So a bound would not have blanked normal assets. **Do not
      re-import that argument.**
      **(b) The reason that does hold** is scope: the defect 0135 introduced
      is a dead venue voting in the §5.5 median, which is per-venue, so the
      guard belongs there. A stale headline price for an asset that stopped
      trading was never this task's bug — the pre-0135 `argMax` published the
      same close.
      **(c) The measured argument, stronger than either:** on prod
      `current_prices FINAL` — 4,444 assets, **1,091 (24.5%) already publish a
      hard zero** with no bound at all. That is the population this PR
      rescues; a tight bound would push part of it back into the sentinel it
      is already stuck in, and waste the rescue for the rest. A zero is worse
      than an old-but-true price because the consumer cannot separate
      "worthless" from "unknown".
      **(d) N is deliberately not chosen.** Enrichment has been failing on
      every invocation for ~2 days ([[0215]]), so any threshold fitted now
      would encode a broken pipeline. `per_source`'s 2h comes from the
      SCHEDULE (`rate(1 hour)` × 2), not from an observed lag. Revisit after
      [[0111]] and [[0215]]. The 24.5% is an upper bound, not steady state.
      **(e) Constraint recorded, not left to luck:** this MV is REPLACE, not
      APPEND, so `current_prices` becomes exactly what the SELECT returns —
      a guard that FILTERS rows would delete those assets from the table
      rather than blank a field. Every guard here emits a sentinel.
      Review findings #1/#3/#5 dissolve with the bound gone; #3 leaves a tail
      (`price_xlm` is a quotient of two independently dated closes, so its age
      is the older of the two) which goes to [[0216]] along with the age
      column itself. The lint now also asserts the bound appears exactly once,
      so it cannot be silently duplicated again.
      okarcz verified all seven MVs on prod: Scheduled, no exceptions,
      `mv_current_prices` at 222 ms reading 2.7M rows and writing 4,444, and
      the live definition matches `current.sql` including TO-column order —
      so the DROP + re-CREATE apply is safe.
  - date: 2026-08-20
    status: active
    who: stkrolikiewicz
    note: >
      PR #228 merged. Two acceptance criteria corrected — they still described
      the bound as applying to `price_usd` and measured it against the asset's
      newest candle, both of which the final revision changed.
      **Failure mode 1 — the question in this task's own title — split out to
      [[0217]].** It is the one thing here that cannot be settled by working
      harder: this task's own sequencing note puts it after [[0118]] (which
      changes which sources reach the median, and so what "outlier" means),
      with [[0123]] as its evidence base, and both are still in backlog.
      Carrying it would keep 0135 open indefinitely on a question it is not
      allowed to answer yet, while the two failure modes that WERE fixed sit
      shipped and unarchived. 0217 also inherits a consequence worth naming:
      by skipping to the newest *priced* close, this task made the outlier
      case more reachable, not less — the 0138 zero that used to mask it is
      gone. Remaining before archive: prod apply per the 0072 runbook,
      `zero_but_vwap_ok = 0`, XLM publishing a real price (may be blocked by
      [[0215]], not by this change), and a green [[0120]] re-run.
  - date: 2026-08-21
    status: active
    who: stkrolikiewicz
    note: >
      **Applied to prod and ROLLED BACK the same session.** The two things
      this task set out to fix worked; the carry bound did not, and the
      counterfactual is what caught it.
      Measured on ch-prod-01 immediately after the apply, then the OLD
      definition's SELECT re-run over the same data as a control (the
      step-0 rollback artifact wrapped in the same aggregate — read-only):
      `zero_but_vwap_ok` 36 → **0** and `zero_price_usd` 1,129 → **376**
      (753 assets regained a real price), both as intended — but
      `empty_sources` 1,096 → **3,380**, so **2,284 assets (52% of the
      table) lost their `sources` object and had `vwap_24h` zeroed**.
      Cause: the 2h bound was calibrated against the enrichment SCHEDULE
      (`rate(1 hour)` × 2), which is the methodologically right basis and
      still wrong in practice — with enrichment down for two days
      ([[0215]]) almost no venue has a priced close younger than 2h, so a
      rule meant as a rare exception became the common case.
      The trade is negative on its own terms, and by okarcz's own argument:
      "a zero is worse than an old-but-true value, because the consumer
      cannot separate worthless from unknown" applies to `vwap_24h` exactly
      as it applies to `price_usd`. We traded 2,284 usable-if-stale VWAPs
      for zeros to gain 753 prices.
      Rolled back via the step-0 artifact and verified: old definition on
      prod (3 `argMax`, 0 `argMaxIf`), refresh clean at 267 ms, and
      `empty_sources` back to 1,156.
      **Design lesson for the next attempt:** the defect the bound protects
      against — a stale venue outvoting a live one in the unweighted §5.5
      median — exists ONLY when a live venue is present to be outvoted. If
      every venue is stale there is nothing to defend and dropping them all
      is pure loss. The bound should therefore be conditional: exclude
      stale venues only when at least one fresh venue survives. That is a
      contract change, so it goes back to okarcz before another apply.
      Operational note for the runbook: `SHOW CREATE` NORMALISES interval
      syntax — `INTERVAL 2 HOUR` renders as `toIntervalHour(2)` — so the
      documented "grep the definition to confirm the apply landed" check
      reports a successful deploy as failed unless it greps for function
      names. Cost me one false alarm mid-deploy.
  - date: 2026-08-21
    status: active
    who: stkrolikiewicz
    note: >
      **Conditional bound implemented and measured BEFORE applying** — the
      step the previous attempt lacked, now runbook step 1b. okarcz confirmed
      the shape. `per_source` derives two prices (latest priced close, and
      latest priced close within 2h); a new level 1b keeps a stale venue
      unless the asset still has a fresh one, so the guard fires only where
      the defect can occur. Code in PR #240.
      **Counterfactual on prod, both variants over the same data in one
      sitting** (4,152 assets): `zero_but_vwap_ok` 30 → **0**,
      `zero_price_usd` 868 → **368**, and — the metric the unconditional form
      destroyed — `empty_sources` 840 → **368**. Every metric improves, none
      regresses. The 472-asset gain on `empty_sources` comes from C2's carry
      rescuing venues with an un-enriched tip, which the conditional guard
      does not take back.
      Emergent property worth keeping: `empty_sources`, `zero_price_usd` and
      `zero_vwap` all land on **368 exactly**. An asset now has a price,
      sources and a VWAP, or none of the three — the contradiction this task
      exists for is gone by construction rather than by luck.
      Sizing, same session: the defect the guard prevents has **zero** live
      occurrences. Seven assets are at risk (>= 3 sources, mixed fresh/stale)
      and on all seven the fresh-only median sits within 1% of the all-source
      median, against a 20% threshold. Kept regardless — the mixed population
      is small precisely BECAUSE almost nothing is fresh while [[0215]] is
      broken, and BE confirmed that fix is imminent, so the risk grows as the
      pipeline recovers. Re-measure after 0215/0111.
      Both arms pinned by fixtures and proven non-vacuous: disabling the
      filter fails the MIX assertion (both venues survive), making it
      unconditional fails the STA assertion with `sources = {}` — the
      production regression reproduced in miniature.
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

- [~] Decision recorded on failure mode 1 (should `price_usd` go through the
      §5.5 keep-mask), with the `market_cap_usd` and `change_*_pct`
      propagation — **moved to [[0217]]**. It cannot be settled here: this
      task's own sequencing note puts it after [[0118]], which changes which
      sources reach the median and therefore what "outlier" means, and
      [[0123]] is its evidence base. Both are still in backlog.
- [~] If filtered: `current.sql` + `current_mv_it.rs` + published-value
      callout — **moved to [[0217]]** with the decision itself.
- [x] If left as-is: the §4.2 `/price` docs state that `price_usd` is unfiltered
      and `vwap_24h` is the de-noised figure. (Done — failure mode 1 itself is
      still undecided; §4.2 now states the asymmetry rather than hiding it.)
- [x] The un-enriched-tip zero is resolved: `price_usd` skips unpriced candles.
      It is deliberately **not** age-bounded — bounding it would trade a known
      price for the 0 sentinel, and 24.5% of prod assets already sit on that
      sentinel. The age itself is published by [[0216]]; the per-venue
      pipeline is where the bound lives. `zero_but_vwap_ok` verification is
      post-deploy and still open.
- [x] `market_cap_usd` and `price_xlm` no longer collapse to 0 purely because the
      newest candle is un-enriched. (Both asserted on fixture 7 in
      `current_mv_it.rs`: `price_xlm` 3.8 and `market_cap_usd` 1900 against a
      seeded supply — the first version of this PR ticked the box citing a
      `market_cap_usd` assertion that did not exist.)
- [ ] Native **XLM** specifically publishes a real `price_usd` — the case BE
      measured and the one [[0144]] shows is close to chronic, since XLM's
      newest candle is both usually newer than the last enrichment pass and
      often an exotic-quote pair that will never be enriched at all.
- [x] The C2 question answered by an **explicit, bounded rule**: a source is
      carried at its latest priced close while that close is younger than 2 h
      (measured against `now()`, not against the asset's own newest candle —
      that reference spans legs enrichment can never price, so it never
      resolves), and is absent from `sources`/`vwap_24h` beyond that or with
      no priced candle at all. The membership rule is now a stated policy
      rather than whatever enrichment happened to have reached, though a
      chronically-unpriced venue still drops out — that population is
      [[0154]]'s. Alignment with [[0147]]'s coverage gate: still open.
