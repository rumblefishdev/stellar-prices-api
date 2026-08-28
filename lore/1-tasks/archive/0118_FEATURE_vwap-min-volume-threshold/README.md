---
id: "0118"
title: "§5.5 VWAP completion — min_volume_usd source threshold + per-request override"
type: FEATURE
status: completed
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
  - date: 2026-08-28
    status: completed
    who: stkrolikiewicz
    note: >
      COMPLETE — all 7 acceptance criteria met and verified on production.
      Producer: conditional MIN_VOLUME_USD = 100 in `current.sql`, ordered
      before both the liveness window and the §5.5 median. API: strict
      per-request `?min_volume_usd=` on `/price` and `/assets`, recomputed
      from the row's own `sources` JSON with no extra ClickHouse work.
      Tests: 4 new MV fixtures (17 DST / 18 ATK / 19 THL / 20 SUB), 7 dto
      unit tests, CH-less 400 tests on both routes, 2 end-to-end override
      ITs; the attack fixture and the all-dust IT were each verified to fail
      against the pre-fix code. Two decisions reversed mid-task: the
      conditional default (a pre-merge prod measurement — the unconditional
      form would have blanked 2,960 of 3,068 priced assets) and the strict
      explicit override (code review). A production defect was found by the
      post-deploy verification and fixed in the same PR: API Gateway keys its
      cache on declared `cacheKeyParameters`, not the query string, so a
      filtered request poisoned the default response for other callers.
      Converted to a directory at close (README 324 -> ~160 lines of body;
      decisions and the rollout log moved to `notes/`). Spawned [[0237]],
      [[0238]], [[0239]], [[0240]]; contributed design input to [[0217]].
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

Producer side (`current.sql`): exclude below-threshold sources from the
`vwap_24h` weighting and from `sources` *before* the median runs, so cheap
venues cannot skew the vote they are not allowed to weight in. `$100` as a
literal with a comment, not a settings table — the MV is redeployed by DDL
anyway. The threshold is a **weighting rule only**: `price_usd` (an `argMax`)
and `volume_24h_usd` (a traded total) are untouched.

API side: accept `?min_volume_usd=` on `GET /assets/{id}/price` and
`GET /assets`, recomputing from the `sources` JSON the row already carries
(option (a)) rather than issuing a live ClickHouse query (option (b), rejected
— it would add a 24h scan + `GROUP BY` to the hottest endpoint and undermine
the p95 SLO that motivated 0072's producer-side design). Validate the param and
return `400` on anything invalid, coordinating with [[0119]].

What the shipped shape actually is — the conditional producer default, the
strict request-level override, and the ordering against the liveness window —
is in [`notes/S-design-decisions.md`](notes/S-design-decisions.md), because two
of those decisions reversed during the task.

> ⚠️ **Refreshable-MV redeploy gotcha** (from 0072): a refreshable MV's
> definition is fixed at create time. Changing the SELECT requires
> `DROP VIEW prices.mv_current_prices` + re-`CREATE` — an `ALTER` does not
> take. `current_prices` keeps serving its last-written rows in the gap, so the
> window is staleness, not an outage.

## Acceptance Criteria

All seven met. Production evidence for the last three is in
[`notes/G-production-rollout-2026-08-28.md`](notes/G-production-rollout-2026-08-28.md);
the reasoning behind the two reversed decisions is in
[`notes/S-design-decisions.md`](notes/S-design-decisions.md).

- [x] Below-threshold sources excluded from `vwap_24h` weighting and absent
      from `sources` — `per_source_kept`/`per_source_funded` + fixtures 17/18;
      conditionally, per decision 6 (an all-dust asset keeps its sources,
      fixture 20)
- [x] `price_usd` and `volume_24h_usd` demonstrably unaffected — fixtures 17
      and 20, and on prod both matched the *unfiltered* raw aggregates at a
      pinned tick on four assets
- [x] `?min_volume_usd=` on both routes, narrowing proven on a real
      multi-source asset — XLM at one MV tick: no param → 3 sources,
      `=5000` → 2, `=200000` → 1, `price_usd` unchanged throughout
- [x] Invalid values return `400` with the standard body — `min_volume_error`
      (finite, ≥ 0, ≤ 1e15); CH-less tests prove the 400 precedes any DB call
- [x] Default path byte-identical with the param omitted or set to the default
      — on a funded asset, which is the whole population the default filtered;
      on an all-dust asset they deliberately differ (decision 3)
- [x] MV redeploy runbook — `docs/runbooks/0118-min-volume-threshold-rollout.md`
- [x] Test proving a dust source cannot move `vwap_24h` — fixture 18 ATK,
      verified to FAIL against the pre-threshold SQL (vwap ~1.3615) and pass
      with it (1.00)

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

## Issues Encountered

- **The MV's `sources` narrowing was invisible through the gateway.** API
  Gateway keys its cache on declared `cacheKeyParameters`, not on the query
  string, so every value of the new parameter shared one entry and a filtered
  request poisoned the default response for other callers. Found by the
  post-deploy verification, fixed in the same PR. The planning note in this
  task had asserted the opposite; it is corrected above. Not a regression —
  the parameter never worked through the cache.
- **A first before/after comparison of `price_usd`/`volume_24h_usd` proved
  nothing** and looked alarming: the snapshots were minutes apart while the MV
  refreshes every minute over a sliding window. Redone pinned to one tick.
- **Two undocumented prerequisites for a local deploy from macOS** — the
  lambda build dies with `ProcessFdQuotaExceeded` under the default file
  descriptor limit (zig links ~250 objects per binary), and
  `tools/scripts/lambda-assets.sh` needs bash ≥ 4 for `mapfile` while macOS
  ships 3.2. Neither is in a runbook; spawned as [[0239]].
- **Task-id collision with `develop`.** 0232/0233 were taken by 0193's own
  renumbering while this branch was in flight; ours moved to 0237/0238. Git
  cannot see this (filenames differ), so it would have merged two tasks onto
  each id.

**Broken/modified tests:** fixture 15 EVN's volumes raised $100 → $200 and
fixture 16 LIV's soroswap $10 → $150, so both keep exercising the mask and the
liveness discrimination rather than the new threshold. Intentional, not a
regression; fixture 15's stated reason was corrected after the threshold went
conditional.

## Future Work

- [[0239]] — document the macOS local-deploy prerequisites (ulimit, bash 4).
- [[0240]] — the deferred `/code-review` cleanups: the $100 literal duplicated
  across two SQL predicates, validation as a typed newtype rather than two
  hand-wired call sites, and the measurement narrative repeated in five places.

## Notes

- Interacts with [[0116]]: the threshold filters a *source*, not a *candle*. A
  single dust trade inside an otherwise-liquid source still produces an absurd
  `close_usd`, and this task does not fix that.
- Interacts with [[0115]]: a source whose quotes are all exotic reports
  `volume_24h_usd = 0` and will be excluded by the threshold — correct
  behaviour, but worth stating so it is not misread as a regression.
