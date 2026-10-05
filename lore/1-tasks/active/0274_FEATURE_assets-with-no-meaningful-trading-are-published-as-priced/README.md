---
id: "0274"
title: "1-stroop offer-priced fills still publish a USD price — e.g. XAUa at $4,375 on $0.003 of volume"
type: FEATURE
status: active
related_adr: ["0287"]
related_tasks: ["0116", "0147", "0252", "0236", "0286", "0278", "0310", "0216"]
tags: [layer-backend, layer-api, priority-medium, effort-medium, milestone-M3, data-quality, liquidity, api]
milestone: 3
links:
  - "../../../../packages/prices-api/src/assets/dto.rs"
  - "../../../../packages/prices-clickhouse/schema/current.sql"
  - "notes/R-offer-priced-dust-population-2026-09-29.md"
history:
  - date: 2026-09-10
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0116]]. 0116 set out to find a per-candle rule for absurd
      `close_usd`; measuring it found that the per-candle framing fits only a
      small minority of the population, and that the majority case is an
      asset-level property. Filed rather than absorbed into 0116, because it is
      a different claim about a different subject — 0116 is about a bucket, this
      is about an asset.
  - date: 2026-09-25
    status: backlog
    who: okarcz
    note: >
      NARROWED on the operator's 2026-09-25 decision, from the 2026-09-24 read-only
      check. The original premise is mostly fixed by [[0286]]'s work: live
      dust-only assets are now published as `unpriced` (65 of 65 checked). Still
      open: 1-stroop fills priced at the resting offer's price still publish a
      price, e.g. XAUa at $4,375 on $0.003 of volume. Narrowed to that. Asset
      103120 `USD` ($0.10 published vs a 0.5 XLM close) is out of scope here; it
      goes to its own new task. Retitled, because the old 93% headline describes
      a population that is now `unpriced`. Original text kept below.
  - date: 2026-09-29
    status: active
    who: akot
    note: >
      Activated. Measured read-only on prod (notes/R-offer-priced-dust-population-2026-09-29.md):
      132 of 3 733 traded-priced assets rest only on offer-priced dust, 20 more
      have real fills but a dust print set the price. XAUa's "$4,375" is a
      1-stroop bot ping and on market. Researched via GSD quick task 260929-mi2.
      Decided: keep the price and publish what it rests on in a separate
      `price_basis` field (option C); no `unpriced`, no fourth `price_status` word.
      Converted to a directory for the measurement note.
---

# 1-stroop offer-priced fills still publish a USD price

## Summary (narrowed 2026-09-25)

**The original premise is mostly fixed.** After [[0286]]'s work, live assets
whose only trading is dust are published with `price_status = "unpriced"`
(65 of 65 checked, 2026-09-24).

**What still publishes:** an asset whose recent fills are 1-stroop trades priced
at a resting offer's price. The candle close is the offer's limit price, not a
market-clearing price, and the asset is published as priced. Example: **XAUa at
$4,375 on $0.003 of volume.**

> **2026-09-29:** the XAUa example is a bot buying 1 stroop every 4 h, and its
> price is on market (within 2 % of XAUa's own real fills and of another gold
> token). The defect is not that such prices are wrong — PAXG at −48 % and
> XGold at +56 % are, BTC `GBVFOW…` at +1.8 % is not — but that **nothing on
> the wire says no real fill supports them**, and our data cannot tell the
> right ones from the wrong ones. See
> [the measurement](notes/R-offer-priced-dust-population-2026-09-29.md).

[[0116]]'s caution still applies: a smallest-unit trade of a genuinely
expensive asset is a real order at the right price. So this cannot be a bare size
threshold. The fix has to tell "a real market that happens to be thin" apart
from "one offer being hit for a stroop".

Out of scope: asset 103120 `USD` ($0.10 published while a 0.5 XLM close implies
about $0.20). That is a different defect and has its own task.

## Acceptance Criteria

- [x] ~~Dust-only assets are not published as priced.~~ **Met via [[0286]]:**
      live dust-only assets are `unpriced`, 65 of 65 checked on 2026-09-24.
- [x] The remaining population (assets published as priced whose price rests on
      1-stroop, offer-priced fills) is measured and dated, with XAUa as the
      reference case. **2026-09-29:** 132 of 3 733, plus 20 whose latest print
      is dust beside real fills
      ([note](notes/R-offer-priced-dust-population-2026-09-29.md)).
      **2026-09-30:** 169 of 3 564, plus 15 (same note).
- [x] A rule separates them from thin-but-real assets, validated against a sample
      of those so they are not swept up. This is the same false-positive
      discipline [[0116]] applied, and it is not a bare size threshold.
      *Rule: see Design. Run on prod 2026-09-30 with the published predicate:
      0 of 224 thin-but-real assets (at most 5 priced minutes, at least $1 in
      24 h) flagged. Known edge: BTC `GBVFOW…`, $86 in 24 h in trades of about
      150 stroops each, reads `offer_dust` — see Implementation Notes.*
- [ ] A consumer can tell, without running their own aggregation, that such a
      price does not rest on a real market (e.g. it is `unpriced`, or carries a
      signal that says so). *Via `price_basis = "offer_dust"`.*
- [ ] `volume_quote_usd` and volume aggregates are explicitly unchanged.
      *By construction: candle tables, rollups and enrichment are not touched.*

## Design (decided 2026-09-29)

**Keep the price; publish what it rests on.** A new field `price_basis` on the
current-price row (`current_prices`, `/assets/{…}/price`, the asset list, the
batch endpoint):

| Value | Meaning |
| --- | --- |
| `trades` | at least one candle in the 24 h window `price_usd` is read from has a price-forming fill above the rounding bound (or the price is the oracle rate) |
| `offer_dust` | every priced candle in that window rests only on order-book fills below the bound — each fill proves an offer existed at that price, not that the price clears |
| `''` | no price (`price_usd = 0`) |

- **Asset-level over the window, not the latest candle.** Flagging from the
  `as_of` candle alone would flag BTC `GDPJAL…` ($212k of real 7-day volume)
  whenever a 1-stroop bot prints last.
- **Separate from `price_status`.** `price_status` is the price's age
  (`carried` = a newer price-forming candle exists); an offer-dust price can
  be carried too, so one word could not say both.
- **Not `unpriced`.** The measured on-market cases (BTC `GBVFOW…`, XAUa) would
  be blanked with the wrong ones, and `unpriced` would change its documented
  meaning. Consumers who want to withhold such prices can key on the field.
- **Derived from existing 1m columns**, with the ingest's integer form of the
  bound (`price.rs`): `source = 'sdex'` and either one fill failing
  `a > 1000 ∧ b > 1000 ∧ (a − 1000)(b − 1000) ≥ 10⁶` in stroops, or
  `pf_volume` / `pf_price_volume` ≤ 1 000 stroops. No candle schema change, so
  0286 phase 3 is unaffected. The test is sufficient, not exact: a minute
  with several dust fills can read `trades` — the conservative direction.
- **Out of scope:** off-market dust beside a real market (group B's junk
  tokens) — [[0310]]; the exact per-candle counter written at ingest —
  follow-up after 0286 phase 3.

## Implementation Notes

- **Commits** on `feat/0274_…`: `f93675fb` ClickHouse (`current_prices.price_basis`,
  `base_tip.confirmed_candles`), `1fe4f936` API, OpenAPI and portal samples,
  `477bafbf` review fixes.
- **Review (2026-09-29, 8 findings).** Fixed: a minute with one price-forming
  fill beside non-forming ones skipped the exact bound; a single fill is now
  judged on its own amounts, so the view matches `price.rs` at the boundary;
  the descriptions say the field describes the 24 h window, not the minute
  `price_usd` came from. Declined: the exact per-candle dust counter written
  at ingest — every 1m writer would carry it, so it waits for 0286 phase 3.
- **Rollout order — it matters.** (1) `ALTER TABLE prices.current_prices ADD
  COLUMN price_basis` (init.sql), (2) `views.sql`, (3) `current.sql` (DROP +
  re-CREATE of the refreshable MV), (4) prices-api. `current.sql` before (1)
  drops the view and fails the CREATE, leaving `current_prices` with no
  writer; the API before (1) fails every price, list and batch request on an
  unknown identifier.
- **Verified**: fmt, clippy, 782 Rust tests on local ClickHouse 26.3.10.60,
  270 portal tests, typecheck, lint. `execution_bound_error_it` needs the
  Caddy proxy and was left to CI.
- **Known edge — a dear asset traded in sub-1 000-stroop fills.** The bound is
  in stroops, so for BTC at $84k every fill under about $8.50 is below it.
  BTC `GBVFOW…` had 68 such minutes and $86 in 24 h on 2026-09-30 and reads
  `offer_dust`, with an on-market price. The label is true to the rule (no
  fill pins the price within 0.1 %). **Decided 2026-09-30 (operator): keep
  the rule**, no USD-volume floor — that would be the size threshold this
  task avoids. The field descriptions now say the bound is not pennies for a
  dear asset.

## Original scope (before 2026-09-25 narrowing)

> Kept verbatim for the record (headings demoted one level). Original title: *Assets with no meaningful trading are published as priced*. The Summary and Acceptance Criteria above supersede it.

### Summary

Measuring [[0116]] on prod found that **93% of dust-print candles (2,741 of
2,944 in `1h` 202608) belong to assets with no non-dust trading anywhere in the
month** — not against any quote, not from any source. 1,969 of those carry a
`close_usd` above $1,000.

For those assets there is no reference price to check a candle against, because
the asset has never traded in a size that would establish one. The accurate
statement is not "this candle is wrong" but **"this asset has no market, and we
publish a price for it anyway."**

### Context

[[0116]] documented the per-candle defect and deliberately did not build a
detector, because a size threshold misclassifies a third of the buckets it
catches (a smallest-unit trade of a genuinely expensive asset is a real order at
the right price). The two-stage check that *is* sound — compare a dust candle to
the same pair's non-dust price — can only be applied to the 7% of dust candles
that have such a reference.

This task owns the other 93%.

### Why it is not just a bigger 0116

The subject differs. 0116 asks "is this bucket's close a market price?" — a
per-row question with a per-row answer. This asks "does this asset have a market
at all?" — a property of the asset over a window, which is where it can actually
be answered, and where a consumer would want it surfaced (the asset listing and
the price endpoints, not each candle).

It is close kin to [[0147]] (`price_usd_series` volume-coverage gate) and
[[0252]] (the volume figure overclaims what it measures, 415 issuers publishing
`BTC`). All three are the same underlying gap: **nothing on the wire distinguishes
a real market from a nominal one.** Whether this should be merged into one of
them or stay separate is the first thing to settle.

### Implementation

- Decide the home: fold into [[0147]] / [[0252]], or keep standalone. Settle
  before building.
- Define "meaningful trading" from the measured distribution, not a guess —
  reuse 0116's method (base amount in minimal units, plus trade count), and
  validate against assets that are genuinely thin but real.
- Surface it where a consumer chooses an asset, not per candle: a liquidity or
  coverage signal on the asset listing and price endpoints.
- Re-measure first. 0116's figures are `1h` 202608 and 202502; the traded
  population swings ~25% a day, so the 93% needs a fresh number and a date.

### Acceptance Criteria

- [ ] The 93% figure is re-measured, dated, and confirmed across more than one
      month and granularity.
- [ ] A decision is recorded on whether this merges into [[0147]] / [[0252]] or
      stays its own task, with the reason.
- [ ] "Meaningful trading" is defined from measurement, and validated against a
      sample of thin-but-real assets so they are not swept up — the same
      false-positive discipline [[0116]] applied.
- [ ] A consumer can tell, without running their own aggregation, whether an
      asset's published price rests on a real market.
- [ ] `volume_quote_usd` and volume aggregates are explicitly unchanged.

### Notes

- Do **not** re-propose a bare size threshold on candles. [[0116]] measured it:
  of dust buckets with a non-dust reference for the same pair, a third are
  priced correctly. That result is recorded in `Candle::volume_base`'s doc
  comment so it is not rediscovered from scratch.
- Distinct from [[0236]] (no detector for internally inconsistent OHLCV rows),
  which is about a row contradicting itself, not about the asset's liquidity.
