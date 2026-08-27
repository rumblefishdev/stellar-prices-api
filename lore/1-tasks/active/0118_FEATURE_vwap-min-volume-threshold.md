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
- **Cache-key impact:** API Gateway caches on query params (§6), so
  `min_volume_usd` becomes part of the key. Confirm this does not shred the hit
  rate for the default path — the param must be *absent* (not
  `min_volume_usd=100`) on the common request for the cached entry to be shared.
  Flag to [[0122]].

## Acceptance Criteria

- [ ] Sources with trailing-24h USD volume below the system default are excluded
      from `vwap_24h` weighting and absent from the `sources` JSON object
- [ ] `price_usd` and `volume_24h_usd` are demonstrably **unaffected** by the
      threshold (regression test with a below-threshold source present)
- [ ] `?min_volume_usd=` accepted on `GET /assets/{id}/price` and `GET /assets`;
      a higher value provably narrows the source set for at least one real
      multi-source asset
- [ ] Invalid `min_volume_usd` values return `400` with the standard error body
- [ ] Default-path responses are byte-identical whether the param is omitted or
      explicitly set to the system default
- [ ] MV redeploy runbook step documented (DROP + re-CREATE, expected staleness
      window, verification query)
- [ ] Test proving a dust-volume source cannot move `vwap_24h` — the actual
      defect this closes

## Notes

- Interacts with [[0116]]: the threshold filters a *source*, not a *candle*. A
  single dust trade inside an otherwise-liquid source still produces an absurd
  `close_usd`, and this task does not fix that.
- Interacts with [[0115]]: a source whose quotes are all exotic reports
  `volume_24h_usd = 0` and will be excluded by the threshold — correct
  behaviour, but worth stating so it is not misread as a regression.
