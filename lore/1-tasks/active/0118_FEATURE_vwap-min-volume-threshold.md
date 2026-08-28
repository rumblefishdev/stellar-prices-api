---
id: "0118"
title: "§5.5 VWAP completion — min_volume_usd source threshold + per-request override"
type: FEATURE
status: active
related_adr: ["0007"]
related_tasks: ["0072", "0039", "0040", "0123", "0144", "0147", "0135"]
tags: [layer-backend, layer-database, priority-high, effort-medium, milestone-M2, vwap, clickhouse, materialized-view, api]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../packages/prices-api/src/assets/handlers.rs"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Scoped to the half of
      §9's "Full VWAP formula wired into the Current Price Updater" that
      [[0072]] does not already own — 0072 covers the per-source `sources`
      breakdown and the inter-source median-outlier filter; this task covers
      the `min_volume_usd` inclusion threshold and its `?min_volume_usd=`
      request-level override.
  - date: 2026-08-27
    status: active
    who: stkrolikiewicz
    note: >
      Promoted to active — starting the threshold pass. Sequenced ahead of
      [[0217]], which waits on this task because the threshold changes which
      sources reach the §5.5 median. Evidence base for tuning landed
      yesterday with [[0123]]'s reconciliation run.
  - date: 2026-08-27
    status: active
    who: stkrolikiewicz
    note: >
      Implementation complete on feat/0118_vwap-min-volume-threshold: MV
      threshold (strict > $100, before the liveness window and the median) +
      API `?min_volume_usd=` override via option (a). 4 new MV fixtures
      (attack fixture verified to fail pre-threshold), 6 dto unit tests,
      CH-less 400 tests, end-to-end override IT with byte-identical default
      path. All suites green locally. Remaining before completion: PR review,
      prod rollout per the 0118 runbook (blast-radius counterfactual +
      post-apply verification, incl. the real-asset narrowing AC), and the
      0123 reconcile-script note about threshold exclusions if 0128 wants a
      fresh green run.
  - date: 2026-08-27
    status: active
    who: stkrolikiewicz
    note: >
      Pre-merge prod measurement flipped the design (Design Decision 6): the
      unconditional threshold would have blanked vwap/sources on 2,960 of
      3,068 priced assets (96.5%; 85% of the table maxes at <= $1 per venue,
      largest casualty $124/day, 0120-list RON included). Reworked to the
      conditional form mirroring the liveness bound — a below-threshold
      source drops only when a funded source survives; the liveness window
      now computes over the threshold's survivors (per_source_funded CTE).
      Explicit ?min_volume_usd= stays strict, asymmetry documented. Fixture
      20 flipped to pin the conditional arm; runbook rewritten (measurement
      recorded, post-apply check is now "no threshold blanking").
  - date: 2026-08-28
    status: active
    who: stkrolikiewicz
    note: >
      Rolled out to production. MV applied (DROP + re-CREATE, refresh clean,
      no exception); post-apply checks green — assets with empty `sources`
      beside a price stayed at 0 (the conditional arm holds), 8 of 15 mixed
      assets lost exactly their sub-$100 venue, and `price_usd` /
      `volume_24h_usd` matched the unfiltered raw aggregates at a pinned tick
      on four subjects. API deployed after a rebase onto develop (the branch
      was 51 commits behind and would have regressed 0229 and the portal).
      **A production defect surfaced during verification and was fixed in the
      same PR:** API Gateway does not key its cache on the query string, so
      the parameter shipped invisible to the cache and one filtered request
      poisoned the default response for every other caller. `cacheKeyParameters`
      now declares it on both routes; re-verified after redeploy. All seven
      acceptance criteria are met.
---

# §5.5 VWAP completion — min_volume_usd threshold

## Summary

Overview §5.5 defines the cross-source weighted price as

```
Weighted Price = Σ(source_price × source_volume_24h) / Σ(source_volume_24h)
Only include sources where volume_24h > configurable_min_threshold_usd (e.g. $100)
```

and §5.5 adds *"Volume threshold is configurable per-request via
`?min_volume_usd=` query param or defaults to the system setting."*

Neither half exists today. `mv_current_prices` sums **every** source
unconditionally (`current.sql:60-66`), and no handler accepts
`min_volume_usd`. A single dust-volume venue can therefore drag `vwap_24h`
toward a price nobody could trade at — the same failure mode [[0116]] documents
at candle level, one layer up.

## Context

The §5.5 formula lives in the **L2 layer** (overview §5.5 layering table). Per
task 0039 Q#1 the "Current Price Updater Lambda" was replaced by
`prices.mv_current_prices` — a `REFRESH EVERY 1 MINUTE` materialised view that
is the **sole writer** of `current_prices`. So "wire the formula into the
Current Price Updater" means "extend that MV", not "write a Lambda".

**Ordering with [[0072]].** 0072 rewrites the same MV to add `sources`,
`price_xlm`, `change_24h_pct`, `change_7d_pct` and the median-outlier filter.
Both tasks touch `current.sql`; **0072 lands first**, and this task adds the
threshold to the shape 0072 leaves behind. Sequencing them avoids two
`DROP VIEW` + re-`CREATE` cycles against prod.

> ⚠️ **Refreshable-MV redeploy gotcha** (recorded in 0072, repeated here so it
> is not re-learned): a refreshable MV's definition is fixed at create time.
> Changing the SELECT requires `DROP VIEW prices.mv_current_prices` followed by
> a re-`CREATE` — an `ALTER` does not take. `current_prices` keeps serving its
> last-written rows in the gap, so the window is a staleness window, not an
> outage.

## Implementation

**Producer side (`current.sql`)**

- Compute per-source 24h volume in the same pass 0072 already needs for
  `sources`, then exclude sources below the system threshold from the
  `vwap_24h` weighting *before* the outlier filter runs (cheap sources are
  dropped first, so they cannot skew the median either).
- Threshold value: system default **$100** per §5.5's worked example. Store it
  as a literal in the MV with a comment, not as a settings table — the MV is
  redeployed by DDL anyway, so a second indirection buys nothing.
- **Do not** apply the threshold to `price_usd` (an `argMax`, not a weighted
  average) or to `volume_24h_usd` (a total — filtering it would misreport
  actual traded volume). The threshold is a **weighting** rule only.
- Preserve the `sources` JSON semantics from §3.3: *"sources excluded by
  `min_volume_usd` or outlier detection are absent from the object"* — so an
  excluded source must be dropped from `sources` too, not merely down-weighted.

**API side (`packages/prices-api`)**

- Accept `?min_volume_usd=` on `GET /assets/{id}/price` and `GET /assets`.
- The MV is precomputed at the **system default**, so a request-level override
  cannot be served by pass-through. Two viable shapes — pick one and record the
  choice under *Design Decisions → Emerged*:
  - **(a) Recompute from `sources`.** Re-weight in the handler from the
    per-source JSON `current_prices.sources` already carries. No extra CH
    round-trip, bounded work, and it degrades to pass-through when the param is
    absent or equals the default. **Recommended** — it protects the p95<200ms
    SLO that motivated 0072's "fill it producer-side" decision.
  - **(b) Live CH query** with the caller's threshold. Correct but adds a 24h
    scan + `GROUP BY` to the hottest endpoint; 0072 explicitly rejected that
    shape for the base path.
- Validate the param: non-negative finite number, sane upper bound, `400` on
  anything else (coordinate with [[0119]]).
- **Cache-key impact — the original note here was WRONG and cost a production
  defect; corrected 2026-08-28.** It read: *"API Gateway caches on query params
  (§6), so `min_volume_usd` becomes part of the key"*, and worried only about
  diluting the hit rate. API Gateway does **not** key on the query string: it
  keys on the parameters declared as `cacheKeyParameters` on the method, and
  collapses every value of an undeclared one onto a single entry. `/price`
  declared only the path id and `/assets` its pre-0118 list, so shipping the
  parameter made a filtered request **poison the default response for every
  other caller** for the TTL — measured on prod right after the deploy: one
  `?min_volume_usd=200000` on `native` made the next param-less request serve
  the narrowed `sources` and reweighted `vwap_24h`. Fixed by declaring
  `qs('min_volume_usd')` on both routes in `api-gateway-stack.ts`, with the
  rule written into the file so the next query param does not repeat it. The
  original hit-rate point still stands on top of the fix: the common request
  must omit the param (not send `min_volume_usd=100`) to share the cached
  entry. Flag to [[0122]].

## Acceptance Criteria

- [x] Sources with trailing-24h USD volume below the system default are excluded
      from `vwap_24h` weighting and absent from the `sources` JSON object —
      **`current.sql` per_source_kept/per_source_funded + fixtures 17/18 in
      `current_mv_it.rs`; conditionally, per Design Decision 6 — an all-dust
      asset keeps its sources (fixture 20)**
- [x] `price_usd` and `volume_24h_usd` are demonstrably **unaffected** by the
      threshold (regression test with a below-threshold source present) —
      **fixture 17 pins price_usd = the dust venue's own close; fixture 20 pins
      the unfiltered volume total; the override IT re-checks both API-side**
- [x] `?min_volume_usd=` accepted on `GET /assets/{id}/price` and `GET /assets`;
      a higher value provably narrows the source set for at least one real
      multi-source asset — **verified on production 2026-08-28 through the
      gateway, all responses at one MV tick (`updated_at 13:32:00`) so the
      deltas are the parameter's and not drift. XLM on `/price`: no param →
      {aquarius, sdex, soroswap}; `=5000` → {aquarius, sdex}; `=200000` →
      {aquarius}. `/assets` narrows the same way per row and EURC lands on the
      all-excluded sentinel (`sources {}`, `vwap_24h 0`) at `=200000`.
      `price_usd` identical across every threshold — the weighting rule holds
      end to end**
- [x] Invalid `min_volume_usd` values return `400` with the standard error body
      — **`min_volume_error` (finite, ≥ 0, ≤ 1e15; serde rejects non-numeric);
      CH-less tests on both routes prove the 400 fires before any DB call**
- [x] Default-path responses are byte-identical whether the param is omitted or
      explicitly set to the system default — **on an asset with a funded venue,
      which is the whole population the default ever filtered: the producer
      already made that cut, so the strict handler filter drops nothing and
      never reformats the Decimal strings (byte-compare in the IT). On an
      all-dust asset the two deliberately differ — see Design Decision 3**
- [x] MV redeploy runbook step documented (DROP + re-CREATE, expected staleness
      window, verification query) —
      **`docs/runbooks/0118-min-volume-threshold-rollout.md`, delegating the
      generic procedure to the 0072 runbook and adding the blast-radius
      counterfactual**
- [x] Test proving a dust-volume source cannot move `vwap_24h` — the actual
      defect this closes — **fixture 18 ATK: two live dust venues that own the
      unweighted median and evict the deep market pre-0118; verified to FAIL
      against the pre-threshold SQL (vwap ~1.3615) and pass with it (1.00)**

## Design Decisions

### From Plan

1. **Threshold before the median, producer-side, as a `WHERE` in
   `per_source_kept`** — a literal `> 100` with a comment, no settings table
   (the MV is redeployed by DDL anyway).
2. **API override = option (a), recompute from `sources`** — the recorded
   choice this task asked for. Reweighting happens in the handler from the
   JSON the row already carries; the hottest endpoint gains zero ClickHouse
   work, protecting the p95 SLO that motivated 0072's producer-side design.

### Emerged

3. **An explicit `?min_volume_usd=` always filters strictly** at exactly the
   value sent, with no pass-through band around the system default —
   **corrected during code review after Design Decision 6 landed.** The first
   draft short-circuited at `threshold <= 100` on the reasoning that "the MV
   already dropped those sources and they cannot be re-admitted". Decision 6
   made that false: the producer default is conditional, so an all-dust asset
   *keeps* its sub-$100 venues, and the short circuit handed a $50 venue back
   to a caller who explicitly asked for $100 — while `100.01` emptied the
   object, a cliff at exactly the documented default. Byte-identity on the
   default path survives as a *consequence* rather than a rule: on an asset
   with a funded venue the producer already made that cut, so the strict
   filter finds nothing to drop and never reformats the producer's Decimal
   strings. Pinned by
   `price_min_volume_cuts_an_all_dust_asset_at_the_system_default`, verified
   to fail against the pre-fix handler.
4. **Strict `>` on both sides** (MV and handler), per §5.5's literal
   "volume_24h > threshold"; a volume exactly equal to the threshold is
   excluded, and one unit test pins the strictness.
5. **Threshold ordered before the `asset_has_live` window too**, not only
   before the median: the liveness guard must not defend a venue the
   threshold is about to erase. Fixture 19 THL discriminates the orders — a
   live dust venue beside a stale real one must not evict it and then vanish.
6. **The system default is CONDITIONAL — reversed from the first draft by a
   pre-merge prod measurement.** The unconditional form (spec-literal) was
   implemented first and measured before merge: it would have blanked
   `vwap_24h`/`sources` on **2,960 of 3,068 priced assets (96.5%)** — ~85% of
   the table has a max per-venue volume of ≤ $1, the largest casualty traded
   $124/day, and 0120-list assets like RON ($4/day) were in the blast radius.
   That is the 2026-08-21 liveness-rollback shape, so the same conditional
   argument applies: the defect needs a funded venue to victimise, and on an
   all-dust asset dropping everything defends nothing. Decided with the team
   2026-08-27 (options considered: unconditional $100 / unconditional $1 —
   still 85% blanked / conditional). A below-threshold source is now dropped
   only when a source above the threshold survives (fixture 20 pins the
   conditional arm); §5.5's "$100" is an "e.g.", but the conditional shape is
   a recorded deviation from its literal reading. The **explicit**
   `?min_volume_usd=` still filters strictly — the caller asked for exactly
   that cut — and the asymmetry is documented in the OpenAPI descriptions.
7. **Handler recompute runs in f64** — deliberately the MV's own numeric
   strategy (it computes the vwap over Float64 arrays before the Decimal
   cast), so the override can never claim more precision than the value it
   overrides; formatting mirrors ClickHouse's trailing-zero-trimmed Decimal
   strings.
8. **SCF scope check — CORRECTED 2026-08-28 against the actual RFP.** The
   first version of this note read the submission page only, found no mention
   of `min_volume_usd`, concluded the sole contractual hook was the "Full VWAP
   formula (§5.5)" work item, and recorded that scope pressure should hit the
   API half first. **That is wrong, and the API half is committed work.**
   - The binding text is **our own proposal**: general-overview §5.5 says the
     threshold is *"configurable per-request via `?min_volume_usd=` query param
     or defaults to the system setting"*. We wrote that, submitted it, and it
     was awarded — so the request-level override is promised regardless of how
     the RFP is read.
   - The RFP (`RFP 1: Prices API`) does carry a Core Requirement of its own:
     **"Adjustable Volume Threshold: VWAP with configurable USD-denominated
     thresholds"**. State its strength honestly: the line never says
     "per-request" or "query parameter", and "configurable" alone is
     compatible with an operator-set constant. What tilts it consumer-ward is
     the company it keeps — every other Core Requirement (Asset Coverage,
     Oracle Coverage, Price Aggregation, Data Endpoints, timeframes,
     availability) describes a capability the delivered API exposes, not a
     deployment setting. That is a contextual argument, not a quotation, and
     it must not be cited as if the RFP demanded the param outright. The
     plural "thresholds" carries no weight either way and was over-read in a
     draft of this note.
   - Consequence for scope pressure: the producer side alone does **not**
     discharge the commitment; "adjustable" is the part §5.5 sold. Both halves
     ship. Also note Design Decision 3's fix matters here — a pass-through
     band around the default would have made the parameter adjustable only
     *above* $100, which is a thinner claim than the one we made.

## Implementation Notes

- `packages/prices-clickhouse/schema/current.sql` — the threshold predicate +
  header note (order vs median and liveness window; weighting-rule-only).
- `packages/prices-clickhouse/tests/current_mv_it.rs` — fixtures 17 DST /
  18 ATK / 19 THL / 20 SUB; fixture 15's volumes raised to $200 and 16's
  soroswap to $150 so both keep testing what they claim under the threshold.
- `packages/prices-api/src/assets/dto.rs` — `SYSTEM_MIN_VOLUME_USD`,
  `apply_min_volume`, `format_decimal` + 6 unit tests.
- `packages/prices-api/src/assets/handlers.rs` — `PriceParams`, the
  `min_volume_usd` field on `ListParams`, shared `min_volume_error`
  validation, OpenAPI param docs on both routes.
- `packages/prices-api/tests/{price,list}.rs` — CH-less 400 tests;
  `tests/price_it.rs` — end-to-end override IT (byte-identical, narrowing,
  all-excluded sentinels).
- `docs/runbooks/0118-min-volume-threshold-rollout.md` — rollout + blast
  radius + post-apply verification.
- Side note recorded in [[0217]]: the weighted-median design input from this
  task's kickoff (unweighted-median manipulation surface, `quantileExactWeighted`
  candidate, the case against age-weighted votes).

## Notes

- Interacts with [[0116]]: the threshold filters a *source*, not a *candle*. A
  single dust trade inside an otherwise-liquid source still produces an absurd
  `close_usd`, and this task does not fix that.
- Interacts with [[0115]]: a source whose quotes are all exotic reports
  `volume_24h_usd = 0` and will be excluded by the threshold — correct
  behaviour, but worth stating so it is not misread as a regression.
