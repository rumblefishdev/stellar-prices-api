---
title: "Reply to BE on their 0199 report — the three USD read-surface findings"
type: generation
status: mature
spawns: []
tags: [be-facing, contract, prices-api, ready-to-send]
links:
  - "../../../../../packages/prices-clickhouse/schema/views.sql"
  - "../../../../../packages/prices-clickhouse/schema/current.sql"
history:
  - date: 2026-08-05
    status: seed
    who: okarcz
    note: "Draft reply. Substance is complete and independent of the prod measurements; figures marked PENDING are filled from G-phase0-prod-queries.md before sending."
  - date: 2026-08-05
    status: developing
    who: okarcz
    note: >
      All PENDING figures filled from the completed A-E measurements. Finding 1
      and 2 sections final. Finding 3 rewritten around two results that changed
      the answer: the argMax defect reaches only the in-flight bucket, and the
      68% unpriced estate is the resolver's reach rather than any bug in this
      report. Blocked on 0135's contract call before sending.
  - date: 2026-08-05
    status: mature
    who: okarcz
    note: >
      **Ready to send.** 0135's contract call settled: `price_usd` publishes the
      latest *priced* close, and an un-enriched venue keeps its latest priced
      close in `sources` / `vwap_24h` rather than vanishing. The reply now
      states both decisions instead of asking BE to make them, records why
      absent/`NULL` was rejected for now, and still invites BE to object since
      they are the only consumer.
---

# Reply to BE — 0199 USD read-surface findings

> **Status: ready to send.** All measured figures filled from
> [`G-phase0-prod-queries.md`](G-phase0-prod-queries.md); phase 0 complete; the
> [[0135]] contract call was settled 2026-08-05 and this reply now **states** the
> decision rather than asking BE to make it.

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

**Confirmed on production, and it is not intermittent — it is continuous.**
Measured 2026-08-05 13:22 UTC:

```
prices.current_prices, native XLM (asset_id 4)
  price_usd  (what you read) : 0
  vwap_24h                   : 0.16726314490953
  with the one-line guard    : 0.16720799309045
  updated_at                 : 13:22:00   <- the view is ticking normally
```

**Cause.** `mv_current_prices` computes the headline price as
`argMax(close_usd, timestamp)` over the trailing 24h of 1-minute candles, with
**no `close_usd > 0` guard**. `close_usd` is written by a separate enrichment
pass on an hourly schedule, so it necessarily sits behind the candle tip.
`argMax` takes the *newest* candle, so it returns whatever that candle holds,
including the zero.

**This is why you see it constantly rather than occasionally.** XLM produces a
candle nearly every minute; the frontier moves at most once an hour. So except
for the moment just after a pass completes, XLM's newest candle is always behind
the frontier and the aggregate always reads a zero.

The cost of the fix is therefore bounded by the enrichment cycle. We measured it
rather than quoting you the schedule:

```
2026-08-05, one enrichment cycle observed
  pass ran ~13:17–13:24, carried the priced frontier to 13:24
  13:23  frontier 13:20   lag  2 min   (pass still running)
  13:45  frontier 13:24   lag 21 min   (frontier static, candles still arriving)
  → worst case just before the next pass ≈ 52 min
```

**So the guarded `price_usd` will be a real price up to ~50 minutes old,
averaging ~25 — against today's permanent zero.**

**We have made that call: `price_usd` becomes the latest *priced* close.** We
considered publishing absent/`NULL` instead, which is arguably more honest, and
rejected it for now — it needs a nullable column or a companion status field,
and we would rather ship you a working number this week than a schema change
next month. That option stays open and is the subject of a design note we are
writing anyway. **If a stale-but-real price is actively worse for your LP
analytics than an explicit absence, say so now and we will revisit** — you are
the only consumer, so your objection wins.

**If ~50 minutes is too stale, say so now**, because the fix is different work.
Making the value *fresher* means shortening the enrichment cadence, and that is
currently blocked by a known cost problem on our side — the pass re-scans the
whole candle table on every batch, so running it 12× more often is not a
configuration change. We would need to fix that first. It is on our list either
way; your answer decides whether it moves up.

For completeness, since it affects the *shape* of the fix and not this finding:
we do have candles that can never be priced — pairs whose quote is not
USDC/USDT/XLM and which have no oracle. **That is not what is happening to
XLM.** The tip we sampled was XLM/USDC on the SDEX orderbook, which we price
from an oracle, and every completed hour in the last 24 was fully priced. Plain
lag explains your finding entirely. We mention the unpriceable class only
because it is the reason your option A cannot work — see finding 3.

The fix is one line — `argMaxIf(close_usd, timestamp, close_usd > 0)` — and the
contract change it implies is decided, above.

**On your 0039 question — the premise doesn't hold, and this is our
documentation failing you.** 0039 is completed and archived, and the "Current
Price Updater" it named was **eliminated, not shipped**: five of the six
`current_prices` columns turned out to be SQL-derivable, so the scheduled Lambda
became the refreshable materialized view `prices.mv_current_prices`. The thing
you observe ticking every minute is that view. So "is native pricing in 0039's
scope" has no yes/no answer as posed — **the owner is our task 0135**, against
`current.sql`, and your XLM measurement is the strongest argument yet for doing
it. It is now our highest-priority quick win.

**A second consequence you have not hit yet, but will — and we only found it
because of your report.** The same unguarded pattern sits in the `per_source`
CTE, where it is "rescued" downstream by a `WHERE src_price > 0`. The effect:
**a venue whose newest candle is not yet enriched disappears entirely from the
`sources` JSON and from the `vwap_24h` weighting.** In the same sample:

```
sources: {"aquarius": …, "phoenix": …, "soroswap": …}      <- sdex absent
```

`sdex` had traded in almost every minute of the preceding 20, including a
24,079-unit print. It is missing because its newest candle was not yet enriched.
So the `vwap_24h` above is not "the 24h VWAP" — it is the VWAP over whichever
venues happened to be enriched at refresh time, with nothing in the payload
saying so.

**The bias runs the wrong way.** A venue is dropped precisely when its newest
candle is inside the enrichment lag — which is *more* likely the more actively it
trades. A quiet venue's newest candle is usually behind the frontier and
therefore already priced. **The busier the venue, the more likely it is to be
excluded from the number.**

**We are fixing this the same way** — the venue will carry its latest *priced*
close and stay in the payload, so `vwap_24h` goes back to being weighted across
all venues rather than across whichever ones happened to be enriched. Same
one-line change, same task, same release.

Until it ships: if you are using `vwap_24h`, or reading `sources` for venue
coverage, treat both as unreliable inside the enrichment lag window.

---

## Finding 2 — materialize `price_usd_series*`

**Granted in principle — this was pre-authorized and your measurement trips the
trigger.** Our own schema header says "promote to a materialized table only if
measured read latency demands it", and the design note says the same. 70.7M read
rows / 4.6 s / 2.1 GiB for a 104-week window is the demand.

**We suspected half your scan was a bug of ours. We measured it, and it is
not — so your request stands on its own merits.** Both series views join

```sql
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
```

and `prices.assets` is a `ReplacingMergeTree` ordered by *natural identity*
(`asset_code, issuer_address, contract_address`) — **not** by `asset_id`. So
`FINAL` dedups on identity, and any `asset_id` mapped to more than one natural
identity multiplies its candles. We expected that to be your "scans every
asset's daily candles twice". Measured over your 104-week window:

```
candles 11,685,065   joined rows 12,233,490   ratio 1.047
3,278 asset_ids carry duplicate identities; 2,493 of them have candles,
totalling 548,439 candle rows
```

**+4.7%, not 2×.** The duplicated identities sit almost entirely in thin
long-tail assets. **So fixing it will not measurably improve your read cost, and
we are not going to imply otherwise** — your 4.6 s is the shape of the view, not
our defect, which makes the materialised table the right answer rather than a
workaround for a bug.

**It is still a correctness problem for you, though, and that is the reason it
is ordered ahead of the materialisation.** In `price_usd_series` the fan-out
feeds a `GROUP BY` on identity, so one `asset_id`'s candles are attributed to
*every* natural identity sharing that id — meaning **a second identity publishes
a price series for volume it never traded**, across those 548,439 rows. The
weighted numbers for the real identity stay correct (uniform duplication
cancels), but the extra rows are fiction. **If you are keying LP analytics on
natural identity, you are picking up identities that never traded** — worth
checking your long tail specifically, since that is where these sit.

**Order of work:** fix the fan-out (our task 0139) → settle the population rule
from finding 3 → then materialise, identity-keyed as you asked. We are not
materialising first, because that would bake those 548,439 rows of
wrong-identity attribution into a physical table where they become a stored
fact instead of a view artefact.

**Where your 4.6 s actually goes — we measured this too.** Running the identical
`FINAL` join over your identical 104-week window, but *only counting rows*,
costs **344 ms and 33M read rows**. So neither the join nor the fan-out is your
bottleneck. The cost is what that count skipped: the weighted aggregation and
the `GROUP BY` on four **computed** identity columns, which no index can help
with and which forces the whole window to be materialised before grouping.

That is genuinely good news for your request — **it means the materialised table
attacks the real bottleneck**, because it precomputes exactly that aggregate. If
the cost had been in the join, the identity-keyed table you asked for would have
helped much less than you expect.

One caveat on comparing our number to yours: you measured 4.6 s uncached and
ours may have hit a warm cache, so treat the row counts as comparable and the
wall-clock as not.

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
wildly off unit price.

We re-ran your asset. Your 12:00 bucket reads `volume_base 42,037.752` — your
42,038, so we are looking at exactly your rows. **Your 13:00 bucket, which you
watched read 1.3085, now reads 0.16931024.** That is the part we think should
worry you most: not that it was wrong live, but that **the published value for a
closed historical bucket changed underneath you**, with nothing to signal it.

**(ii) Your 14:13 reading — the bucket reverting to unpriced.** Partial
enrichment does not explain a bucket that *had* a priced row and then had none —
that is data moving backwards. Different cause: every rollup tier carries
`close_usd` forward with the same unguarded `argMax` as finding 1. When the
latest sub-bucket is not yet enriched, the coarse row inherits **0**, discarding
the priced sub-buckets underneath it. **A partly-enriched hour does not roll up
as partly priced — it rolls up as unpriced.**

We caught this live while investigating: an hour whose four 15-minute
sub-buckets were **all priced**, rolling up to a 1-hour row reading `close_usd =
0`. So the mechanism is confirmed on production, not just in a test.

**But we also measured how far it reaches, and the answer should reassure you
about your history.** Across seven days of hourly rows and thirty days of daily
rows, *every* wrongly-zeroed row was in the **bucket currently being formed** —
115 hourly rows, all in the current hour; 449 daily rows, all in the current
day. Once a bucket closes and enrichment catches up, the rollup re-runs and
repairs it. **This bug costs you the live edge, not your history.**

### On your two proposed options

**Option A — "exclude the bucket until all its rows are enriched" — cannot
terminate, and we want to be blunt about why.** Our enrichment resolves a USD
price only when the candle's **quote** is USDC, USDT or XLM, or has a Reflector
oracle row. Everything else keeps `close_usd = 0` indefinitely, so any bucket
containing one such row would be suppressed in perpetuity. This is the kind of
gate that looks correct in a test fixture and silently strands real assets in
production.

**We should be straight with you about one thing here, because it cuts against
us.** That floor is the reach of our current resolver, not a fact about the
market. Measured over seven days: yXLM-quoted candles are **never** priced —
114,330 of them — even though we price yXLM itself perfectly well as a base
asset and publish its USD price. Same for XRP, 42,296 rows. Across five quote
assets we sampled, **18.4% of candles are unpriceable purely because the
resolver does not pivot through an already-priced quote.**

So option A cannot terminate *against today's resolver*, which is the honest
version of the claim. Adding that second pivot hop is now on our list — it
shrinks the problem at the source instead of working around it downstream, and
it would shrink the population every other fix here has to handle. It does not
change our recommendation below, but you should know the constraint we are
citing is one we can partly remove.

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

We are picking X from the measured distribution, not from taste — and that
measurement has already changed the design. Over 48 hours, **~51% of buckets sit
strictly between 0% and 100% coverage**, so this is not the thin tail we assumed.
Worse, there is a hard mode at **exactly 50%** holding 16.9% of all buckets, and
we traced it: those are **path payments**, where your base asset is the
intermediate hop and the trade is booked against two quotes — one of which we
can price and one of which we cannot. Those buckets sit at 50% *permanently*.

**A naive gate at any X above 0.5 would therefore black out ~15% of buckets
forever — the exact defect we just told you sinks your option A.** So the gate
will measure coverage against **priceable** volume rather than total volume,
which scores those path-payment buckets at 100% where they belong.

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

---

## The thing you did not ask about, which matters more than all three

While sizing the above we measured the whole daily estate, and you should have
this number before you build on it:

```
prices.price_ohlcv_1d, 90 days, by the class of the candle's quote asset
  quote is USDC          59,229 rows     1.6% unpriced
  quote is XLM          308,436 rows     0.1% unpriced
  quote is USDT           1,943 rows    18.1% unpriced
  quote is anything else 945,752 rows  100.0% unpriced   <- 72% of the table
```

**About two-thirds of our daily candles have no USD price at all, and never
have** — stable at 65–72% every month for the last 24 months, rising slowly as
the long tail grows. This is not caused by anything in your report, it predates
every incident on our side, and no fix in the list below moves it.

The cause is that our enrichment resolves USD only when the candle's **quote**
is USDC, USDT or XLM, or has an oracle. Everything else stays at zero. That
includes cases that are frankly embarrassing: **yXLM-quoted candles are never
priced even though we price yXLM itself perfectly well** and publish its USD
value — 114,330 such candles in seven days. Same for XRP.

**Practical consequence for your LP analytics:** if a pool's assets trade mainly
against exotic quotes, you will get sparse or empty series from us regardless of
everything else we fix here, and the sparsity is worst in exactly the long tail
where LP coverage is most interesting. **Check your asset set against this
before sizing your own work** — we would rather you find out now than after we
ship the fixes below and coverage barely moves.

We are adding a second pivot hop — price a candle whose quote is any asset we
already have a USD close for — which resolves yXLM, XRP and most of that tail.
On the sample above it recovers 18.4% of rows from five quote assets alone.
**This is now the largest single improvement available to you**, larger than any
of the three findings you filed, and your report is what surfaced it.

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
