---
title: "Reply to BE on their 0199 report — the three USD read-surface findings"
type: generation
status: developing
spawns: []
tags: [be-facing, contract, prices-api, draft]
links:
  - "../../../../../packages/prices-clickhouse/schema/views.sql"
  - "../../../../../packages/prices-clickhouse/schema/current.sql"
history:
  - date: 2026-08-05
    status: seed
    who: okarcz
    note: "Draft reply. Substance is complete and independent of the prod measurements; figures marked PENDING are filled from G-phase0-prod-queries.md before sending."
---

# Reply to BE — 0199 USD read-surface findings

> **Status: draft, not sent.** Everything below is settled except the figures
> marked **⟪PENDING …⟫**, which come from
> [`G-phase0-prod-queries.md`](G-phase0-prod-queries.md). The argument does not
> depend on them — they only size it. Send once A–E are back.

---

## Short version

All three findings are real. **Two are our bugs and we own them; one is a valid
request we had already pre-authorized in the schema header.** Your numbers are
the first external measurement of these views and we are treating them as ground
truth about what we actually ship.

We also found the defect is **wider than your report** — the same unguarded
aggregate appears in 130 places across six schema files, not the six you
located. That is our problem, not yours, but it changes our fix order and
therefore the dates below.

One thing to action on your side immediately: **the `price_usd_series_1h`
workaround you adopted for finding 1 walks you straight into finding 3.** See
"What to do meanwhile".

---

## Finding 1 — native XLM `price_usd` = 0

**Confirmed, and it is close to chronic for XLM specifically** — not
intermittent as it might look from a single sample. ⟪PENDING A2: XLM publishes
`price_usd = 0` for N of the last 24 hours.⟫

**Cause.** `mv_current_prices` computes the headline price as
`argMax(close_usd, timestamp)` over the trailing 24h of 1-minute candles, with
**no `close_usd > 0` guard**. `close_usd` is baked by a separate enrichment pass
that runs hourly, so the newest candle is usually not yet priced — and `argMax`
returns whatever the newest candle holds, including the zero. XLM is the worst
case because it is the most-traded asset (so it almost always has a candle newer
than the last enrichment pass) and it has the widest set of counter-assets (so
its newest candle is often an exotic-quote pair that will *never* be priced).

The fix is one line — `argMaxIf(close_usd, timestamp, close_usd > 0)`.

**On your 0039 question — the premise doesn't hold, and this is our
documentation failing you.** 0039 is completed and archived, and the "Current
Price Updater" it named was **eliminated, not shipped**: five of the six
`current_prices` columns turned out to be SQL-derivable, so the scheduled Lambda
became the refreshable materialized view `prices.mv_current_prices`. The thing
you observe ticking every minute is that view. So "is native pricing in 0039's
scope" has no yes/no answer as posed — **the owner is our task 0135**, against
`current.sql`, and your XLM measurement is the strongest argument yet for doing
it. It is now our highest-priority quick win.

**One decision we owe you, and you should have an opinion.** Guarding the
`argMax` changes the published contract: `price_usd` becomes *"the latest
**priced** close"* rather than *"the latest close"*. In practice that means the
number can be a few minutes to an hour stale during the enrichment lag, instead
of being zero. We think that is obviously the right trade for your use case, but
say so if a stale-but-real price is worse for you than an explicit absence —
because the third option is to publish `NULL`/absent and make you handle it.

**A consequence we found that you have not hit yet, but will.** The same
unguarded pattern sits in the `per_source` CTE, where it is "rescued" downstream
by a `WHERE src_price > 0`. The effect: **a source whose newest candle is not yet
enriched disappears entirely from the `sources` JSON and from the `vwap_24h`
weighting.** So during enrichment lag you are not just seeing a wrong
`price_usd` — you may be seeing a `vwap_24h` computed over a subset of venues,
with no indication that happened. Same fix, same task.

---

## Finding 2 — materialize `price_usd_series*`

**Granted in principle — this was pre-authorized and your measurement trips the
trigger.** Our own schema header says "promote to a materialized table only if
measured read latency demands it", and the design note says the same. 70.7M read
rows / 4.6 s / 2.1 GiB for a 104-week window is the demand.

**But roughly half of that scan is our bug, not physics, and we want to fix it
before we bake it into a table.** Both series views join

```sql
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
```

and `prices.assets` is a `ReplacingMergeTree` ordered by *natural identity*
(`asset_code, issuer_address, contract_address`) — **not** by `asset_id`. So
`FINAL` dedups on identity, and any `asset_id` mapped to more than one natural
identity multiplies its candles. We have 3,275 such `asset_id`s in production.
That is precisely your "scans every asset's daily candles twice".
⟪PENDING B1: measured fan-out ratio.⟫ ⟪PENDING B2: how many duplicate ids carry
candles.⟫

**This one matters to you beyond cost.** In `price_usd_series` the fan-out feeds
a `GROUP BY` on identity, so one `asset_id`'s candles are attributed to *every*
natural identity sharing that id — meaning **a second identity publishes a price
series for volume it never traded.** The weighted numbers for the real identity
stay correct (uniform duplication cancels), but the extra rows are fiction. If
you are keying LP analytics on natural identity, check whether you are picking
up identities you don't expect.

**Order of work:** fix the fan-out (our task 0139) → settle the population rule
from finding 3 → then materialize, identity-keyed as you asked. Materializing
first would make both defects durable instead of transient.

We will come back to you with a date on the materialized table once the two
prerequisites land. We are not going to give you one now and miss it.

---

## Finding 3 — a dust print becoming the whole bucket price

**Confirmed, and you diagnosed it correctly** — the weighting maths is sound;
the `WHERE close_usd > 0` filter is silently changing the *population*. We
reproduced your yXLM case exactly on the production ClickHouse version.

There are actually **two** mechanisms in what you saw, and separating them
matters because they have different fixes:

**(i) Your 13:29 reading — partial enrichment.** Enrichment runs hourly, so a
live bucket is routinely part-priced. Whichever rows happen to be enriched become
100% of the weight. A dust print is the pathological case: near-zero volume but a
wildly off unit price. ⟪PENDING C1: yXLM `priced_volume_share` by hour.⟫

**(ii) Your 14:13 reading — the bucket reverting to unpriced.** Partial
enrichment does not explain a bucket that *had* a priced row and then had none —
that is data moving backwards. Different cause: every rollup tier carries
`close_usd` forward with the same unguarded `argMax` as finding 1. When the
latest sub-bucket is not yet enriched, the coarse row inherits **0**, discarding
the priced sub-buckets underneath it. **A partly-enriched hour does not roll up
as partly priced — it rolls up as unpriced.** ⟪PENDING D2/E: how much of the
coarse estate this has already zeroed.⟫

### On your two proposed options

**Option A — "exclude the bucket until all its rows are enriched" — cannot
terminate, and we want to be blunt about why.** Our enrichment has a
**permanent** floor by design: candles whose quote is not USDC/USDT/XLM and which
have no oracle keep `close_usd = 0` **forever**. Any bucket containing one such
row would be suppressed in perpetuity. This is the kind of gate that looks
correct in a test fixture and silently strands real assets in production.

**Option B — "weight over the unenriched rows too" — taken literally, is worse
than the status quo.** We measured it: removing the filter returns **0.000023**
against a true ~0.170, because an unpriced row enters as a zero numerator against
a full-weight denominator. Your wording ("once they land") suggests you actually
mean *defer until enriched*, which is the coverage gate below — but the literal
reading is a trap and we would rather say so than let it get implemented.

### What we will ship instead

**A volume-coverage gate.** Publish the bucket only when the enriched rows
account for at least X% of the bucket's `volume_base`. This:

- prices a bucket as soon as its *real* volume is priced, rather than waiting for
  a permanently-unpriceable dust tail;
- is immune to the dust-print case **by construction**, because it is a
  weight-share test rather than a row-count test — 0.764 units out of 42,038 is
  not 90% of anything;
- terminates, unlike option A.

We are picking X from the measured distribution of coverage across a normal day,
not from taste. ⟪PENDING C2: the distribution, and the X it implies.⟫

**Plus a coverage column, which we think you actually want more than the gate.**
Any single threshold we choose is our judgment imposed on your use case. We
intend to expose `priced_volume_share` on the row so you can set your own bar —
suppress at 0.99 for headline TVL, accept 0.5 for a chart. Tell us if you'd
rather not have the extra column.

The underlying design flaw is that `close_usd` is a non-nullable column
defaulting to `0`, so **"not yet priced", "will never be priced" and "genuinely
worth nothing" are the same value.** Every one of these three findings is a
different aggregate meeting that ambiguity. We are writing that up as an ADR so
the next surface we build doesn't inherit it.

---

## What to do meanwhile

**Your `price_usd_series_1h` workaround for finding 1 routes you straight into
finding 3.** Until the coverage gate ships, a single-hour close can be a dust
print — that is exactly the 7.7× you measured. Until then:

- prefer a **multi-hour median** over a single-hour close, or
- check that **neighbouring hours agree** before trusting a spot value, and
- treat any hour whose value differs from its neighbours by more than ~2× as
  suspect rather than as a real move.

The same junk single-trade candles reach every OHLCV granularity, so `/ohlcv`
has the same exposure. The difference in these views is that the filter promotes
a junk *row* into the *whole bucket's* answer.

## What we are doing, in order

| | Work | Fixes | Status |
|---|---|---|---|
| 1 | Guard the aggregate in the four pre-roll scripts | prevents new bad data at backfill scale | ⏰ deadline-driven, unblocked |
| 2 | Guard it in `current.sql` + settle the `sources`/`vwap_24h` contract | **finding 1** | needs the contract call above |
| 3 | Guard it in the six rollup views | **finding 3(ii)** | blocked on our own delivery mechanism |
| 4 | Repair the already-zeroed history | historical `close_usd` | after 3 |
| 5 | Volume-coverage gate + `priced_volume_share` | **finding 3(i)** | needs the distribution |
| 6 | Fix the identity fan-out | **finding 2a** | running in parallel now |
| 7 | Materialize `price_usd_series*`, identity-keyed | **finding 2** | after 5 + 6 |

Items 1–3 are one-line changes; the schedule risk is all in delivery and
verification, not in the code. **We will not have item 7 before items 5 and 6.**

Ask for the current state of any of these at any time. And please keep
measuring — this report found things our own tests did not.
