---
title: "min_volume_usd — the eight decisions, and the two that reversed"
type: synthesis
status: mature
spawned_from: ../README.md
spawns: []
tags: [vwap, clickhouse, materialized-view, api, scf]
links:
  - "../../../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-08-28
    status: mature
    who: stkrolikiewicz
    note: >
      Split out of the task README when it passed 300 lines. Two of these
      decisions reversed during the task — 3 in code review and 6 on a
      pre-merge production measurement — so the reasoning is kept in full
      rather than compressed to the outcome.
---

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