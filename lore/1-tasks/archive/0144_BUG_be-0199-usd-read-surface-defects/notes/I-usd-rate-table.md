---
title: "Idea — split the USD rate out of close_usd into a per-quote-asset rate table"
type: idea
status: mature
spawned_from: notes/G-be-0199-reply-short.md
spawns: ["0167", "0168"]
tags: [schema, clickhouse, enrichment, zero-as-missing, adr-input]
links:
  - "../../../../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../../../../packages/prices-clickhouse/schema/init.sql"
history:
  - date: 2026-08-06
    status: developing
    who: okarcz
    note: >
      Raised by the operator while reviewing the sent BE 0199 reply — "what if
      we split price_usd out of the candle tables into a dedicated table?".
      Worked through in session; the stronger form is a **rate** table, not a
      price table. Captured here as input to [[0151]], which this widens
      materially. Nothing measured yet — the two load-bearing numbers are
      flagged as open below.
  - date: 2026-08-10
    status: mature
    who: okarcz
    note: >
      LINEAGE RECORDED. This note's narrow form is live as tasks 0167 (build
      prices.usd_rate + populate the peg rates) and 0168 (publish the real rate
      in price_usd_series in place of the hardcoded $1). The schema-wide
      refactor below stays REJECTED per the decision block at the top - see
      0151. Linked because the note was reachable only by knowing it existed in
      an archived task's notes directory, which is how a settled decision gets
      accidentally relitigated.
      Context that post-dates the note: 0165 shipped the $1 placeholder to prod
      on 2026-08-10 with the `method` provenance column, deliberately shaped so
      0168 is a one-expression change. And the clock in this note is now the
      binding constraint - oracle_prices is pruned at INTERVAL 13 MONTH, so
      202509 ages out ~2026-10/11 and that depeg-aware history is unrecoverable
      once dropped.
---

# Split the USD rate out of `close_usd`

> ### ✅ DECIDED 2026-08-06 — read this first
>
> **Adopted narrowly as [[0154]]'s implementation; the schema-wide refactor
> below is rejected for now.** The rate table gets built, keyed on **natural
> identity** rather than `asset_id`, scoped to the granularities 0154 prices,
> and **`close_usd` is not touched** — it stays non-nullable, `DEFAULT 0`, with
> no consumer-visible change.
>
> Reasoning and the revisit trigger live in [[0151]]. In short, BE's 2026-08-06
> response removed three of the four arguments below: `NULL` is actively harmful
> to the only consumer, the bugs this dissolves ([[0145]], [[0146]], half of
> [[0147]]) are one-line fixes already queued, and [[0139]] turned out to be
> genuine `asset_id` collisions — which a table keyed on `quote_asset_id` would
> have inherited.
>
> What survived is enough: it makes [[0154]], the top of the queue, a self-join
> on a small table instead of another pass over a fact table [[0111]] already
> re-scans every batch.
>
> **Everything below is preserved as the reasoning record.** The "what it buys"
> list is written as if the full refactor happens — items 1, 5, 6 and the
> `Nullable` recommendation are **not** what was decided.

## The observation

`close_usd` is not a stored fact, it is a **cached product**. From
`ch_enrich.rs:9-11, 22-30`, all three enrichment tiers compute the same shape:

```
close_usd = close × <USD rate of the candle's QUOTE asset at that time>
```

- **oracle tier** — rate from an `ASOF LEFT JOIN oracle_prices` on
  `quote_asset_id` (depeg-aware; wins where it applies)
- **peg tier** — USDC/USDT ⇒ rate = `$1`, exact, oracle-free, back to genesis
- **pivot tier** — XLM-quoted ⇒ rate = the volume-weighted XLM/USDC close,
  forward-filled by ASOF

In every tier the rate is a function of **`(quote_asset_id, timestamp)` only** —
never of the candle being priced. `volume_quote_usd` uses the identical rate.

So today we look the rate up, multiply it into each candle, store the product on
hundreds of millions of rows, and **discard the rate**. It is never written down
as a first-class value anywhere.

## The idea

Store the rate.

| quote_asset_id | timestamp | usd_rate | method | source |
|---|---|---|---|---|
| USDC | 13:00 | 1.000 | peg | — |
| XLM | 13:00 | 0.167 | pivot | XLM/USDC vwap |
| USDT | 13:00 | 1.000 | peg | — |

A handful of assets per time bucket, instead of one stored price per candle —
roughly two orders of magnitude smaller than the fact tables. `close_usd`
becomes `close × rate`.

## Why this is the better form of "split out the USD price"

The operator's original phrasing was a per-candle USD table. That version fights
ClickHouse's grain: it repeats the 4-column key `(asset_id, quote_asset_id,
source, timestamp)` on every row, and puts a hundreds-of-millions × hundreds-of-
millions join on the read path — the join shape CH is worst at, on the surface
BE is already calling slow (4.6 s).

The rate table has the same benefits and none of that. The right side is small
enough to be a genuine dimension table, which is the one join shape CH handles
well.

## What it buys

1. **Absence becomes representable.** No rate row ⇒ `NULL` after a LEFT JOIN,
   and CH aggregates skip NULL by default. `argMax(close_usd, ts)` returns the
   latest *priced* close **with no guard**. That is the whole [[0151]]
   zero-as-missing class, dissolved rather than guarded — and it makes [[0145]],
   [[0146]] and half of [[0147]] unnecessary rather than fixed.
2. **[[0154]] becomes a self-join on a small table.** The second pivot hop is
   "price anything quoted in an asset we already price" — a transitive closure
   over rates. We *already* price yXLM fine (114,330 candles in 7 days) yet
   never price yXLM-**quoted** candles, purely because the first fact was never
   stored in a form the second step could consume. One new rate row makes every
   yXLM-quoted candle in history derivable.
3. **[[0111]] loses its cause.** The hourly full-table re-scan exists because
   enrichment asks *"which candles are missing a price?"*, answerable only by
   reading every candle (`FINAL WHERE volume_quote_usd = 0`). With rates it asks
   *"which rates are new since I last ran?"* — a small query against a small
   table.
4. **Finding 1 goes away rather than being mitigated.** See triggers below.
5. **[[0149]] resolves structurally.** The sweep-vs-MV version war exists because
   two writers own one column with incompatible arithmetic. A derived cache has
   one owner and one derivation rule.
6. **[[0148]] mostly evaporates.** A lost value that is *derivable* is not lost;
   you re-derive instead of sweeping for it.
7. **Provenance.** `method` / `reference_asset` / `priced_at` on the rate row
   answers "where did this number come from", which today is unanswerable — and
   which [[0147]] needs in order to define *priceable* volume.

## Keep `close_usd`, but demote it

Not "delete the column". Keep it, and change what it **is**: from the only copy
to a **derived cache** of `close × rate`, rebuildable at will.

- **Don't delete it** — (a) read cost: BE is already at 4.6 s and would gain a
  join; (b) reproducibility: a stored value is a fact ("this candle was worth
  $0.170"), a computed-on-read value can silently change next week when a rate
  is corrected. For price data consumers build on, that matters.
- **Don't leave it as-is either** — if it stays `DEFAULT 0` non-nullable, the
  whole 0144 class survives on the read side and the rate table has fixed
  nothing consumers can see. **Make it `Nullable`** in the same move.

## When does the recompute happen?

The trigger changes from a clock to an event.

| # | Trigger | Scope of work |
|---|---|---|
| 1 | **A candle is written** | Price it in-hand if the rate exists. USDC/USDT rates are known permanently in advance ⇒ instant. XLM's rate arrives in the same tick ⇒ ≤1 min. |
| 2 | A new rate lands (oracle sample, XLM/USDC close) | Exactly the affected `(quote_asset, minutes)` slice |
| 3 | A quote asset *becomes* priceable ([[0154]]) | One deliberate bulk pass, not a permanent scan |
| 4 | A rate is corrected (depeg, bad print) | That window. Today effectively impossible. |

**Trigger 1 is where nearly all the value is, and it is what kills finding 1.**
Not "mitigated by a ~25-min-stale fallback" — actually gone: XLM's newest candle
carries a real price within a minute of existing, because nothing waits on the
hourly pass. The hourly Lambda demotes to a small bounded reconciler.

Candles whose quote has no rate at all stay honestly absent (`NULL`), and fill
in when trigger 3 later makes that quote priceable.

## Open questions — do not build before these are answered

1. ⚠️ **Finding the affected candles is not free.** Triggers 2–4 all need "every
   candle quoted in X between T1 and T2". `price_ohlcv_1m` is
   `ORDER BY (asset_id, quote_asset_id, source, timestamp)` (`init.sql:122`) —
   `quote_asset_id` is the **second** key column, not a prefix. CH can skip some
   granules on a non-leading key column but far less effectively. The likely
   answer is a projection on `(quote_asset_id, timestamp)`, which costs storage
   and write time. **Measure this; do not assume it.** Trigger 1 is unaffected.
2. ⚠️ **Rate time-resolution across the six granularities.** 1m is the obvious
   base, but a `_1d` candle needs a daily rate — average? close? volume-weighted?
   Getting this wrong invents a subtler restatement of finding 3. Settle it in
   the ADR before any DDL.
3. `volume_quote_usd` needs the identical treatment and the identical rate.
4. Tier precedence must survive: the oracle tier is depeg-aware and **beats** the
   peg tier. The rate row must record which tier produced it, and the ordering.
5. The pivot tier's rate is itself derived from candles (XLM/USDC vwap) — so the
   rate table has an internal dependency order. [[0143]]'s "no `DEPENDS ON`
   anywhere in the cascade" applies here too.

## Migration is additive — that is the best part

Nothing breaks until the last step:

1. Build the rate table alongside everything. No reader changes.
2. Backfill it from what enrichment already knows.
3. **Verify it reproduces today's `close_usd`.** Same inputs, same answers. A
   mismatch is a bug found either way.
4. *Then* decide whether `close_usd` stays a cache or goes.

## Status

Design idea only. **Nothing here is measured** — the two load-bearing unknowns
are open questions 1 and 2. Input to [[0151]], whose scope this widens from a
narrow zero-as-missing ADR to "what is the source of truth for a USD price".
