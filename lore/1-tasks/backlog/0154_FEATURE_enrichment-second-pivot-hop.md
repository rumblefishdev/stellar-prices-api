---
id: "0154"
title: "Enrichment resolves only USDC/USDT/XLM quotes — two thirds of every OHLCV tier has no USD price"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0144", "0061", "0111", "0147", "0151", "0116", "0135"]
tags:
  [
    "priority-high",
    "effort-medium",
    "layer-enrichment",
    "clickhouse",
    "data-coverage",
    "be-interop",
  ]
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0144]] phase 0. Not a defect BE reported — found while
      sizing finding 3's blast radius on prod, and it turned out to dwarf all
      three of their findings. Measured, not inferred: 945,731 of 945,752
      exotic-quote daily rows carry `close_usd = 0`, a share of 1.0000, and
      that class is 71.9% of the table.
---

# Enrichment resolves only USDC/USDT/XLM quotes

## Summary

`close_usd` is absent on **~68% of every OHLCV tier**, has been for at least 24
months, and the cause is not a bug in any read surface — it is the **reach of
the enrichment resolver**. A candle gets a USD price only when its **quote**
asset is USDC, USDT or XLM, or has a Reflector oracle row. Every other quote
keeps `close_usd = 0` permanently, by construction (`ch_enrich.rs:25-31`).

Adding a **second pivot hop** — price a candle whose quote is any asset we
already hold a USD close for — resolves most of that tail. yXLM is the clearest
case: we price yXLM itself perfectly well and publish its USD value, yet
**114,330 yXLM-quoted candles in seven days carry no price** because yXLM is not
one of the three recognised quotes.

## The consumer sized this (BE, 2026-08-06)

Replying to [[0144]]'s report, BE measured **both legs priceable across all
52,369 classic pools**:

| What their code reads | Pools | % |
|---|---|---|
| `price_usd_series_1h`, 48h — **their headline TVL** | 23,228 | **44.4%** |
| `price_usd_series` daily, 90d (chart) | 35,673 | 68.1% |
| `price_usd_series` daily, ever | 50,258 | 96.0% |

They pre-emptively falsified the recency explanation — worst-leg staleness per
pool **≤2d 44.7%, ≤7d 46.3%, ≤30d 57.1%, never priced 4.0%**, so a 3.5× looser
staleness rule buys **+1.6pp**. Their conclusion matches ours: *"the gap isn't
recency, it's the quote-asset restriction."*

**They rank this task above the materialised table they filed the original
request for** ([[0150]], consequently dropped to priority-low).

> ⚠️ **Do not quote 44.4% → 96.0% as this task's headroom.** The spread between
> "48h" and "ever" is largely pools whose legs *stopped* being priced — quite
> possibly dead long-tail pools that stopped trading at all, in which case
> pricing them helps nobody. **Answer O1 first:** of the pools priceable "ever"
> but not within 48h, how many still trade? BE appear to hold the per-pool list;
> ask before measuring it ourselves. Until then the honest claim is "moves the
> 44.4%", magnitude unknown.

## Context

Found in [[0144]] phase 0 while sizing finding 3. Full measurements and method:
[`0144/notes/G-phase0-prod-queries.md`](../active/0144_BUG_be-0199-usd-read-surface-defects/notes/G-phase0-prod-queries.md).

**Measured on prod 2026-08-05**, `price_ohlcv_1d` over 90 days, split by the
class of the candle's quote asset:

| quote class | rows | zeroed | share |
|---|---|---|---|
| **anything else** | 945,752 | 945,731 | **1.0000** |
| XLM | 308,436 | 354 | 0.0011 |
| USDC | 59,229 | 932 | 0.0157 |
| USDT | 1,943 | 351 | **0.1806** |

Per-month, `_1d` runs **65–72% unpriced for 24 consecutive months** — with no
spike during [[0136]]'s 17-day freeze or [[0111]]'s four-day outage, which is
what rules out every incident-shaped explanation. `_1w` runs ~55–65% and `_1M`
~48–52%. **The share is rising** — 0.58 in 202408 → 0.73 in 202607 — as the long
tail grows against a fixed resolver.

### Why this outranks the tasks it was spawned beside

[[0144]] split into [[0145]]–[[0151]], all of which address rows the `argMax` or
the `close_usd > 0` filter mishandles. Phase 0 measured that population: the
`argMax` defect reaches **only the bucket currently being formed** (115 `_1h`
rows, 449 `_1d` rows, all in-flight). Those fixes are correct and still worth
shipping — they fix the live edge BE actually reads — but they move a few hundred
rows. **This moves two thirds of the table.**

### Two dependents

- **[[0147]]'s coverage gate** cannot use a static quote allowlist for its
  "priceable volume" denominator, because this task grows the priceable set.
  The gate must derive priceability, which is also what [[0151]] must specify.
- **[[0116]]** owns junk candles; some exotic-quote rows carry nonsense unit
  prices (`close 32111.9` on `volume_base 0.0000631` was in the 0144 sample).
  Pricing them makes that junk *visible in USD* — so 0116's threshold work
  should land alongside, not after.

## Implementation

Sketch only; measure before committing to a shape.

### Build it on a rate table — decided 2026-08-06 in [[0151]]

**Do not implement this as another pass over the fact table.** Introduce a
narrow **USD rate table** and make this tier a lookup against it. Full reasoning
and the rejected wider refactor: [[0151]]; origin note:
[[I-usd-rate-table]] (under 0144).

The insight is that `close_usd` is already a cached product — all three existing
tiers compute `close × <USD rate of the candle's QUOTE asset at that time>`
(`ch_enrich.rs:9-11, 22-30`), and that rate is a function of the quote asset and
the timestamp **only**, never of the candle being priced. We currently look it
up, multiply it into hundreds of millions of rows, and discard it. This task is
where storing it pays for itself: the second hop becomes *"yXLM already has a USD
rate — write it down, and every yXLM-quoted candle is derivable"* rather than a
re-scan of a table [[0111]] already re-scans every batch.

**Shape:**

```
prices.usd_rate
  asset_kind        LowCardinality(String)   -- natural identity, NOT asset_id
  asset_code        String
  issuer_address    String
  contract_address  String
  timestamp         DateTime
  usd_rate          Decimal(38,14)
  method            LowCardinality(String)   -- 'oracle' | 'peg' | 'pivot' | 'pivot2'
  reference_asset   String                   -- what it pivoted through ('' for oracle/peg)
  hops              UInt8                    -- 0 oracle/peg, 1 XLM pivot, 2 this tier
  version           UInt64
ENGINE = ReplacingMergeTree(version)
ORDER BY (asset_kind, asset_code, issuer_address, contract_address, timestamp)
```

**Five constraints, each load-bearing:**

1. ⚠️ **Key it on natural identity, not `asset_id`.** [[0139]] is now confirmed
   as **genuine `asset_id` collisions between unrelated assets** — `asset_id
   4194` is both `STW` and `ARBRIDGE`. A rate keyed on `quote_asset_id` would be
   ambiguous for exactly those 3,279 ids, and would bake a non-unique key into
   new infrastructure. Natural identity sidesteps 0139 entirely, whichever way
   that fix lands.
2. **Scope it to this task.** Rates only at the granularity this tier prices —
   do **not** generalise to all six tiers. That was the wider refactor 0151
   rejected, and it is what makes the two open unknowns in [[I-usd-rate-table]]
   small enough to answer here.
3. **`close_usd` stays exactly as it is** — non-nullable, `DEFAULT 0`, written
   in place. This tier still writes the product into the fact table; the rate
   table is where the *rate* comes from, not a replacement read path. **No
   consumer-visible change**, which is required: BE state that a `NULL` "renders
   as a dash and removes the pool from every USD view we have".
4. **`hops` carries the confidence** point in item 3 of the old sketch below —
   this is the provenance column, and this tier is its first consumer.
5. **Additive and verifiable before it is trusted:** build the table, backfill
   it from what enrichment already knows, and **check it reproduces today's
   `close_usd` for the tiers that already work** before letting it price
   anything new. A mismatch is a bug found either way.

### Then the tier itself

1. **A second pivot tier** after the existing peg/pivot step: for a candle whose
   quote `Q` is not USDC/USDT/XLM and has no oracle, look up `Q`'s rate at or
   before the bucket (`ASOF LEFT JOIN prices.usd_rate`, same shape as the
   XLM/USDC pivot at `ch_enrich.rs:27-30`) and set `close_usd = close × Q_usd`.
   Emit `Q`'s own rate row while you are there — that is what makes the next
   asset cheap.
2. **Decide the transitivity limit.** One hop covers yXLM and XRP. Two hops
   invite cycles and error compounding. Recommend **exactly one hop** and a
   documented refusal to chain further — write it down, because the next reader
   will want to generalise it.
3. **Confidence must be representable.** A one-hop price is strictly weaker
   evidence than a peg. Carried by `usd_rate.method` / `hops` above — this tier
   is that column's first real consumer. If [[0151]] later adds a status column
   on the fact table, use the same vocabulary rather than inventing a second.
4. **Cost — [[0111]] is a hard blocker, decided 2026-08-05.** Enrichment
   already re-scans the whole table each batch (490–545M rows, ~35 s/batch
   under load, and a four-day production outage on that account). This task
   adds another join over a larger candidate set. **0111 ships first.** The
   alternative — prototype and measure — was considered and rejected: 0111's
   own baseline was taken on a quiet cluster and proved 80× optimistic, so a
   measurement here would carry the same risk of being reassuring and wrong.
5. **Backfill.** Fixing the resolver prices new candles. The two-year historical
   estate needs a re-run — likely the largest part of the work, and it collides
   with the same pre-roll/window hazards as [[0148]].

### The USDT anomaly, in scope here

USDT quotes are **18.06% unpriced against USDC's 1.57%** — 11× the rate for a
stablecoin handled by the *same* peg tier. Likeliest cause is a **second USDT
issuer** not matched by the `USDT_ISSUER` constant (`ch_enrich.rs:67`). Only
1,943 rows in 90 days, but it is nearly free to fix and it is the same code
path.

## Acceptance Criteria

- [ ] A candle whose quote has a known USD close at or before its bucket gets a
      `close_usd`, measured as a drop in the unpriced share on the prod pin.
- [ ] yXLM-quoted and XRP-quoted candles price. These are the named regression
      cases — 114,330 and 42,296 rows per seven days respectively.
- [ ] `prices.usd_rate` exists, keyed on **natural identity** (not `asset_id`),
      and reproduces today's `close_usd` for the oracle/peg/pivot tiers before it
      is trusted to price anything new.
- [ ] No consumer-visible change to `close_usd`'s type or null-ness — BE's
      dash-renders-as-missing constraint holds. → [[0151]]
- [ ] The transitivity limit is enforced in code and stated in the header, with
      the reason.
- [ ] A one-hop price is distinguishable from a peg/oracle price by a consumer,
      or the decision not to distinguish them is recorded with its rationale.
- [ ] The USDT gap is diagnosed and either fixed or written off with the issuer
      list that explains it.
- [ ] Enrichment duration under backfill load does not regress — measured
      against [[0111]]'s baseline, on a loaded cluster, not a quiet one.
- [ ] The historical estate is repaired or explicitly written off, coordinated
      with [[0148]] so both do not rewrite the same rows.
- [ ] BE re-measures coverage for their LP asset set and confirms the change.

## Out of scope

- The `argMax` guards ([[0145]], [[0146]]) and the coverage gate ([[0147]]) —
  different defect, different population, already split out.
- Pricing pairs with **no** reachable USD reference at any hop. Those remain a
  genuine permanent floor and are the reason a "wait for full enrichment" gate
  can never terminate — see 0144's reply to BE.

## Notes

- **This is the honest answer to "is historical `close_usd` trustworthy?"** It is
  ~68% absent, stably, for reasons unrelated to anything BE reported. 0144's
  reply says so plainly; do not let a later fix imply otherwise.
- The floor was documented as **permanent** (`ch_enrich.rs:31-32`,
  `count_remaining_at_volume_zero`). That documentation is accurate about
  today's behaviour and misleading about what is *achievable* — it reads as a
  property of the market rather than of our resolver. Fix the wording here too.
- ⚠️ **Check [[0151]] before designing the hop.** A rate-table normalisation was
  raised 2026-08-06 —
  [`0144/notes/I-usd-rate-table.md`](../active/0144_BUG_be-0199-usd-read-surface-defects/notes/I-usd-rate-table.md).
  Under it this task becomes a **transitive closure over a small
  `(quote_asset_id, timestamp)` rate table** — "we already price yXLM fine; write
  that down as a rate and every yXLM-quoted candle becomes derivable" — rather
  than another pass over the fact table. Same outcome, very different
  implementation. Do not commit to a shape until 0151 decides.
