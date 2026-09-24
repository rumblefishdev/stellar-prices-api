---
id: "0310"
title: "Off-market prints publish as the bucket's USD price — a deviation filter (Hampel/MAD) against the asset's own neighbouring buckets, if one ever hits an asset that matters"
type: FEATURE
status: backlog
related_adr: ["0287", "0292"]
related_tasks: ["0147", "0286", "0209"]
tags: [layer-database, priority-low, effort-medium, data-correctness, enrichment, clickhouse]
links:
  - "../archive/0147_FEATURE_price-usd-series-volume-coverage-gate.md"
history:
  - date: "2026-09-23"
    status: backlog
    who: akot
    note: >
      Spawned from 0147 phase 2. The coverage gate stops a small print from
      standing in for UNPRICED volume; nothing stops a fully priced bucket
      whose trade itself was off-market. Measured the same day: 0.3–0.5 % of
      hourly buckets sit > 2x from the asset's own ±12 h median, none on the
      7 assets with ≥ $1M 3-week volume. Parked at low priority on that
      evidence, with the trigger below.
---

# Filter off-market bucket prices by deviation from the asset's neighbours

## Summary

A real, correctly recorded trade at a price far from the market becomes the
bucket's USD price on `price_usd_series*` and `current_price_usd`. No rule
catches it:

- [[0286]] drops only rounding dust (ADR 0287 bound), not a normal-sized trade.
- [[0147]] withholds a bucket only when the priced share of its volume is
  below X; this bucket is fully priced.
- There is no absolute USD floor, by measurement: a floor carries no
  information about price quality (0147 phase 2).

The standard answer is a deviation filter: compare the bucket's price with the
same asset's neighbouring buckets and flag or withhold it when it is too far out.

## Evidence (prod, `dev_read`, 2026-09-23)

Hourly buckets 2026-09-01 → 09-22, price computed as the 0147 gate computes it;
reference = median of the same asset's OTHER hourly buckets within ±12 h
(≥ 3 required). 274,754 checkable buckets, 18 % had no reference.

| Asset's 3-week priced USD | Assets | Buckets > 2x off | Share of those |
| --- | --- | --- | --- |
| < $100 | 15,955 | 572 | 41 % |
| $100 – 10k | 409 | 516 | 37 % |
| $10k – 1M | 81 | 297 | 21 % |
| ≥ $1M | 7 | 0 | 0 % |

- 1,385 buckets (0.50 %) > 2x off, in 380 assets. Post-0286 day: 0.31 %.
- Of the 55 on assets ≥ $100k, 35 are one token (VORIXLM, `GBXYDAPK…`) whose
  price spans 4e-5 → 60 in the window — wash-trading-shaped; a filter would
  not make its price meaningful.
- Neither dollar volume nor trade count predicts the error; RELATIVE thinness
  does (bucket < 1 % of the asset's typical volume: 6–13x the rate), but
  catches only ~10 % of outliers.
- Some "outliers" are genuine moves on a thin market; a self-referential
  median cannot tell the two apart.

Scripts and raw data: not committed (0147 session scratchpad).

## When to pick this up

Any of:

- BE reports a wrong price on an asset that carries TVL, and it is an
  off-market print rather than 0147's partial-enrichment case;
- an asset with ≥ $1M 3-week volume shows a > 2x bucket;
- a consumer asks for a price-quality flag beyond `priced_volume_share`.

## Industry practice (for the design)

| Provider | Rule |
| --- | --- |
| CoinGecko | median ± 4 × 1.4826 × MAD with ≥ 3 sources; below that a 100x band around the previous price |
| CCData CCIX | drop a price > 4x or < ¼ of the last index value |
| CF Benchmarks | drop a venue > 5 % off the other venues' median |
| Barndorff-Nielsen et al. 2009 | drop an observation > 10 mean absolute deviations from a centred rolling median of 50 |

## Decisions to make

1. Reference: neighbouring buckets of the same asset, a second pair of the
   same asset, or the previous day.
2. Threshold k and window; what to do below 3 neighbours (18 % of buckets).
3. Action: withhold, flag (a new column, like `priced_volume_share`), or
   replace with the median.
4. Where it runs: not in the view (a window per read is expensive) — in
   enrichment or a materialised table.
5. Alternative worth weighing first: a volume-weighted median instead of the
   mean (CF, Kaiko). A single print cannot move it unless it is over half the
   bucket's volume.

## Acceptance Criteria

- [ ] The decisions above made and recorded, with a measured blast radius on
      prod (how many published buckets change, per grain).
- [ ] A regression test reproducing a > 2x print on a fully priced bucket.
- [ ] BE told what changes on the wire, if anything does.
