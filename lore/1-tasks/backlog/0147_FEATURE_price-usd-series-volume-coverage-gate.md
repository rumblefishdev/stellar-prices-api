---
id: "0147"
title: "Replace price_usd_series*'s close_usd > 0 filter with a volume-coverage gate"
type: FEATURE
status: backlog
related_adr: ["0292", "0287"]
related_tasks: ["0144", "0118", "0131", "0116", "0146", "0150", "0061", "0151", "0286"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "be-interop", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0144]] future work (phase 5) — BE 0199 finding 3i. Both of
      BE's own proposed fixes were measured and rejected; this is the
      implementable form of their intent.
  - date: "2026-09-17"
    status: backlog
    who: akot
    note: >
      Re-read against [[0286]] (merged, ADR 0287) and ADR 0292 from [[0151]],
      which DECIDES what this task SHIPS. Not solved by 0286: it deliberately
      left `views.sql` / `current.sql` without a price-forming gate, and BE's
      yXLM print (0.764 units = ~7.6M stroops) clears the 0.1 % bound. [[0146]]
      is closed as superseded — the rollup half is 0286's rate-form `close_usd`.
      From ADR 0292: a candle is PRICED iff `close >= 1e-12 AND
      close_usd >= 1e-12`; `priced_volume_share = Σ pf_volume(priced) /
      Σ pf_volume(eligible)`, eligible = quote asset has a conversion path, so
      unpriceable legs AND dust-only candles are outside the denominator; the
      gate is the default, the share is always on the wire, an absolute floor
      sits beside the ratio; X is measured on our own history after the 0286
      rollout (research found no outside number to copy — nobody else has an
      async conversion step; a median would not have fixed the yXLM case). Also
      handed over: the same floor and a pf gate in `views.sql` arm A and
      `usd_reference*`, WITH the cross-surface test under the floor (a test of
      agreement cannot land before its fix); `current.sql`'s `src_is_live` /
      `src_volume` counting dust-only minutes; the untested 1h filter; and the
      additive wire fields `price_status` (priced | carried | unpriced) and
      `as_of` on the current-price surfaces. Gap list:
      `docs/database-schema/close-usd-zero-guardrails.md`.
---

# Volume-coverage gate for `price_usd_series` / `price_usd_series_1h`

## Summary

Both views filter

```sql
WHERE p.close_usd > 0
```

before volume-weighting. The filter was written to stop un-enriched rows
dragging the weighted mean toward zero, and it does — but it makes the
denominator `sum(volume_base)` run **over the enriched subset only**, so the
weighting population depends on how far the hourly enrichment pass happened to
have got. Whichever rows are enriched become 100% of the weight.

BE measured the pathological case on yXLM (2026-08-04 13:00): a **0.764-unit
dust print at 1.3085 USD** was the hour's only enriched row, so the view
returned 1.3085 against ~0.170 in every neighbouring hour — **7.7×**.
Reproduced on CH 26.3.10.60 ([[0144]] `repro/`, TEST B).

The weighting arithmetic is sound — BE said so and they are right. The same
dust print sits in the fully-enriched 12:00 bucket beside 42,038 units of real
volume and moves the result by nothing. It is the population that is wrong.

## Both of BE's proposed fixes fail — measured

- **"Exclude the bucket until every row is enriched" cannot terminate.**
  Enrichment documents a **permanent** exotic-quote floor: candles whose quote
  is not USDC/USDT/XLM and which have no oracle keep `close_usd = 0` forever,
  by design (`ch_enrich.rs:31-32`). Any bucket containing one such row would be
  suppressed in perpetuity. This is the kind of gate that passes every test and
  then strands real assets on prod.
- **"Remove the filter and weight over everything" is worse than the status
  quo.** Measured: **0.000023** against a true ~0.170, because an unpriced row
  enters as a zero numerator against a full-weight denominator.

## The implementable version of their intent

Publish the bucket only when the enriched rows account for **≥ X% of the
bucket's `volume_base`**.

- Prices a bucket as soon as its *real* volume is priced.
- Ignores a permanently-unpriceable dust tail, so it terminates.
- Being a **weight-share** test rather than a row-count test, it is immune to
  the dust-print case **by construction** — 0.764 units against 42,038 can
  never clear the bar alone.

Additionally expose **`priced_volume_share`** so a consumer can set its own bar
rather than inheriting ours. The `views.sql` header already promises
value-or-absent semantics; "partially enriched" is a third state that today
masquerades as a good value.

## Implementation

- Pick X from **[[0144]] query C's real distribution on prod**, not from taste.
  Check what `priced_volume_share` looks like across a normal day first.
- Apply to both `price_usd_series` and `price_usd_series_1h`. Both are
  `CREATE OR REPLACE VIEW` ([[0134]]), so delivery is safe.
- **Ship one definition of "priced enough", not three.** [[0118]]
  (`min_volume_usd` inclusion threshold) and [[0131]] (pre-roll USD coverage
  gate) are proposing the same predicate in two other places. Unify the
  threshold and its naming across all three, or reconcile explicitly why they
  differ.
- [[0116]] is what makes the dust rows junk in the first place; this gate stops
  a junk row *being* the answer. Complementary, not alternative.

## Still needed after [[0286]]

[[0286]] (which superseded [[0146]]) fixes zeros manufactured by the rollup
chain. This gate covers the case
where the **base table's own rows** are unpriced, which no rollup fix can reach.

## Acceptance Criteria

- [ ] Neither view can return a bucket whose published price rests on a
      negligible share of that bucket's volume — regression test on CH
      **26.3.10.60** reproducing BE's yXLM case.
- [ ] A fully unpriceable bucket is absent; a *pending* bucket is
      distinguishable from a *priced* one — not conflated.
- [ ] X justified against query C's measured distribution, recorded in the
      header.
- [ ] `priced_volume_share` exposed to consumers. **Confirmed as wanted by the
      only consumer** — BE, 2026-08-06: *"please do expose the coverage share,
      we'll set our own bar on it."* Not optional; ship it with the gate.
      → `0144/notes/S-be-0199-response-received.md`
- [ ] Threshold definition reconciled with [[0118]] and [[0131]].
- [ ] BE told the gate has shipped and what X is.
