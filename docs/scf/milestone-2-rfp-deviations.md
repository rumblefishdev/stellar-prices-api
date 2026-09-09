# Milestone 2 — deviations from the RFP, and why

Every place the delivered API departs from the literal wording of the RFP or of
the Tranche 2 acceptance criteria, with the reasoning and the evidence for each.

**This document exists because Milestone 1 was accepted on the strength of the
same discipline.** M1's package carried a section headed _"What is deliberately
not claimed"_, and that honest gap list is what made the rest of it credible.
Three of its rows became funded M2 scope. This is the M2 instance.

Nothing here is a request to lower a bar. Each entry names the deviation, says
what we did instead, and supplies the measurement to judge it on.

| #   | RFP / criterion says                                            | We deliver                                                                                          | Where     |
| --- | --------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | --------- |
| 1   | `Current Price (float USD)`                                     | a decimal **string**                                                                                | §1, below |
| 2   | _"…return `X-Cache: Hit` header"_                               | a reworded criterion, graded on latency and the deployed cache configuration; no such header exists | §2        |
| 3   | USDC 1d candles verifiable against known price history          | XLM and yBTC instead; USDC excluded                                                                 | §3        |
| 4   | Tranche 3: _"`sdex.status: \"running\"`, `last_push_at` fresh"_ | the archive finished early and reports `completed`; the criterion needs rewording                   | §4        |

---

## 1. `Current Price (float USD)` — we publish a decimal string

### The deviation

The RFP's _Asset Metadata Required_ list types the field as:

> **`Current Price (float USD)`**

We publish it as a JSON **string**:

```json
"price_usd": "1.00002374814135"
```

The same applies to **every** numeric value in the API — not only this field.
`vwap_24h`, `volume_24h_usd`, the per-venue `sources` values, and every OHLCV
`open` / `high` / `low` / `close` are decimal strings. Counts and ledger
sequences remain plain JSON integers, because they are exact in a float and
nothing is lost.

### Why

**A JSON number is an IEEE-754 double in every mainstream parser.** `JSON.parse`
in JavaScript, `json.loads` in Python, `encoding/json` into `interface{}` in Go
— all produce a float64 with a 53-bit mantissa, roughly **15-16 significant
decimal digits**. Our prices are `Decimal(38, 14)`: fourteen fractional digits,
by design.

That gap is not theoretical here, and we do not have to argue it from first
principles — **it has already caused a measured defect inside this system.**

🔑 **Evidence 1 — float precision produced a malformed candle on production.**
Recorded in [ADR 0011](../../lore/2-adrs/0011_base-currency-is-a-denomination-not-a-pair-filter.md):
deriving a candle's extremes through `toFloat64` returned a `close` **below**
its own `low` by **1.343e-11** at BTC 1h — violating the
`low <= open,close <= high` rule every charting library assumes. The ADR
identifies the mechanism explicitly: float64 holds ~15-16 significant digits
while a five-figure price at `Decimal(38, 14)` carries **19**, and the observed
gap was **0.92 of one float64 ulp** at that magnitude. Rounding cannot fix it —
a 14-decimal half-tick is 5e-15, some 2,700× too small to account for the error.

That damage came from a _single internal_ float conversion. Publishing floats
would impose the same conversion on every consumer, on every field, on every
request.

🔑 **Evidence 2 — the affected values are real assets, not hypotheticals.**
Prices on this store reach **7e-8** (RON, measured on production during
[task 0123](../../lore/1-tasks/archive/0123_TEST_vwap-reconciliation-against-raw-ohlcv/README.md)),
and a non-canonical USDC was observed at `close = 5e-14` — five ticks above the
`Decimal(38, 14)` floor. **These are exactly the long-tail assets the RFP asks
us to cover.** A float round-trip silently destroys their low-order digits; the
value still parses, still looks plausible, and is quietly wrong.

🔑 **Evidence 3 — the string form is verified end to end.** Task 0120's
conformance suite asserts across the API that numeric strings parse and that
`Decimal(38, 14)` precision survives the JSON round-trip. Its recorded result:
_"`Decimal(38,14)` strings parse everywhere."_ The deviation is not merely
argued; the alternative it selects is tested.

### What a consumer has to do differently

Parse the string with a decimal type — `BigDecimal`, Python's `decimal.Decimal`,
`decimal.js`, Go's `shopspring/decimal`. This is the ordinary convention for
financial APIs and costs a consumer one wrapper at the parse site.

⚠️ **A consumer who wants a float can still have one** — `parseFloat` on our
string yields exactly the number the RFP's literal reading would have delivered.
**The reverse is not true**: had we published a float, no consumer could recover
the digits the serialisation had already destroyed. The deviation is strictly
more capable than the literal wording, not less.

### Where it is documented for the API's users

The published OpenAPI document states it at the API level — _"Prices, volumes
and rates are decimal strings rather than JSON numbers, so no precision is lost
in transport; counts and ledger sequences are plain integers"_ — and on the
`price_usd` field itself, with the reason.

---

## 2. `X-Cache: Hit` — the header does not exist, and the criterion is graded against a reworded observable

### The deviation

**Tranche 2 acceptance criterion 3** reads:

> _"Cache confirmed: consecutive identical requests within TTL window return
> `X-Cache: Hit` header."_

The criterion bundles two claims — **that the cache works**, and **that a
particular header proves it**. The first is confirmed on every cached route. The
second is not achievable on this architecture, and we are not going to pretend
otherwise.

**We grade the criterion against this wording instead:**

> _"Cache confirmed: consecutive identical requests within the TTL window are
> served from the API Gateway stage cache, and a request after the window is not.
> Demonstrated by response latency, which separates cleanly, and by the deployed
> per-method cache configuration."_

Two things change, and we would rather name both than have them noticed:

1. **The observable** moves from a response header to measured latency plus the
   per-method cache configuration read back off the deployed stage.
2. 🔑 **The claim weakens.** A header would be _the cache asserting itself_.
   Latency is _behaviour consistent with a cache_. That is a weaker form of
   proof. It is stated here in those words rather than blurred at submission
   time.

### Why

**API Gateway emits no `X-Cache` header on any route.** This is not a setting we
left switched off — the stage cache has no hit/miss header feature at all. A live
`200` carries `x-amzn-requestid`, `x-amz-apigw-id`, `x-amzn-trace-id`,
`cache-control`, `vary` and `access-control-allow-origin`, and nothing else. The
criterion's wording is CloudFront's; this API is a regional REST API with no
CloudFront distribution in front of it. Verified independently three times, on
2026-08-20 and twice on 2026-09-03.

Three ways to produce the header were examined. All three are closed with
reasons in
[ADR 0012](../../lore/2-adrs/0012_api-gateway-stage-cache-no-cloudfront-no-x-cache-header.md),
accepted and team-ratified, not merely unchosen.

🔑 **Evidence 1 — a header written by our own handler would be actively wrong,
not merely absent.** API Gateway replays a cached response **byte for byte,
headers included**, and the Lambda runs only on a miss. A header the handler
writes therefore freezes at miss time and is replayed verbatim on every
subsequent hit — reporting **`Miss` on exactly the requests that were hits**, the
inverse of what the criterion asks for. This was measured, not assumed. Across a
TTL boundary on `/price`, the response body hash held at `4694801b` through every
hit and changed to `039f54a6` only when the entry expired:

| request               | body hash              |
| --------------------- | ---------------------- |
| first ask (miss)      | `4694801b`             |
| +2 s (hit)            | `4694801b` — identical |
| +4 s (hit)            | `4694801b` — identical |
| +13 s (miss, expired) | `039f54a6` — changed   |

A header that lies is worse than a header that is absent, because a reviewer
could reasonably act on it. The same reasoning rules out integration-response
header mappings: they run on the integration, and a hit never reaches it.

🔑 **Evidence 2 — CloudFront would not satisfy the wording either.** It is the
only architecture that emits a truthful hit/miss header, and **it writes
`X-Cache: Hit from cloudfront`, not `X-Cache: Hit`.** A reviewer reading the
criterion literally would still see a header that does not say `Hit`, and this
same section would still have to be written. That is what decides it: the
expensive option does not buy the thing it costs a week to buy.

What it would have cost: roughly six cache behaviours to reproduce the current
per-route, per-parameter cache key; an origin request policy keeping `x-api-key`
**out** of that key, or every API key gets a private cache and the hit rate
collapses; a DNS migration for the production domain; a decision on the existing
0.5 GB stage cache; and re-verification of the CORS and gateway-response work
behind a new edge. **Three to five days, plus edge deploy risk.** ⚠️ Cost is
explicitly _not_ the argument — the two are within a couple of dollars a month of
each other, and the certificate that would have been needed already exists.

### Why an amendment rather than a request

**Declaring an in-tranche refinement, with the reasoning attached, is the
practice this project has been graded on before.** Milestone 1 was accepted with
a section headed _"In-tranche scope refinements"_ that disclosed a change of
primary datastore, and its evidence document carried a _"What is deliberately not
claimed"_ list. Within Tranche 2 itself, **AC 2 already carries an in-place
amendment note** — the 100 req/s target is unchanged but no key in the account
can sustain it since task 0157 — and **AC 6 defers its spot-check dates to the
reviewer**. The criteria list in `prices-api-general-overview.md` §12 now carries
the same kind of note against AC 3.

This is not a request to lower a bar. The cache itself is held to the original
standard and passes; what changes is the instrument used to read it, because the
instrument the criterion names does not exist on this platform.

### The evidence, and where it is weakest

Latency measured as server time only (`time_starttransfer - time_appconnect`), so
the TLS handshake a fresh `curl` pays is excluded. `/price`, declared 10 s TTL,
same URL throughout:

| request                    | server time  |
| -------------------------- | ------------ |
| first ask (MISS)           | 144.7 ms     |
| +2 s (HIT)                 | 53.3 ms      |
| +4 s (HIT)                 | 45.1 ms      |
| +6 s (HIT)                 | 47.7 ms      |
| **+13 s (MISS — expired)** | **139.7 ms** |
| +15 s (HIT)                | 49.5 ms      |

**Hits 45-53 ms, misses 78-145 ms, no overlap.** Expiry is demonstrated on both
TTL tiers and both re-fill immediately. All six key-gated cached routes show the
pattern, and the figures reproduce task 0121's k6 numbers independently, from a
different tool on a different day.

⚠️ **The evidence is not uniformly strong, and a header would not have had this
property.** The gap between a hit and a miss tracks how expensive the underlying
query is, so the method discriminates unevenly:

| route              | miss/hit ratio | daylight    |
| ------------------ | -------------- | ----------- |
| `/ohlcv`           | 2.7×           | 120 ms      |
| `/price`           | ~2.9×          | ~95 ms      |
| `/backfill/status` | **1.4×**       | **19.5 ms** |

19.5 ms is real and repeatable, but close enough to ordinary variance that a
single pair of requests would not settle it on that route. It is carried by the
deployed configuration, not by the latency alone. `/api-docs-json` is a further
gap: confirmed **configured** at a 3600 s TTL but deliberately **not** claimed as
demonstrated, because its entry only clears on a deploy flush, so no miss can be
induced to compare against.

### If the literal header is required

Say so and we will treat it as separate scope. It is the CloudFront project
costed above — three to five days, a DNS migration, and re-verification behind a
new edge — and it would deliver `X-Cache: Hit from cloudfront`, which still does
not match the criterion's literal string.

**Full reasoning, every measurement, and the reproduction method:**
[`prices-api-cache-verification.md`](../prices-api-cache-verification.md).
**Architectural decision, with all three rejected alternatives and a
when-to-revisit list:**
[ADR 0012](../../lore/2-adrs/0012_api-gateway-stage-cache-no-cloudfront-no-x-cache-header.md).

## 3. USDC is excluded from the backfill spot-check

**Tranche 2 acceptance criterion 6** names USDC. We spot-check **XLM and yBTC**
instead, over 28 dates rather than the five asked for, against an independent
off-Stellar exchange — median deviation **0.06%** and **0.48%**, 27 of 28 within
5% on each.

USDC is excluded because its series is entirely peg-derived: all 2,042 points
carry `trade_count: 0` and 1,865 are exactly `1`. On 2023-03-11, when USDC broke
its peg to roughly $0.87, we return `1`. It is right _by construction_ rather
than by measurement — so it cannot fail the check, which is why it cannot pass
it either.

**Full reasoning and the comparison tables:**
[`prices-api-backfill-depth-verification.md`](../prices-api-backfill-depth-verification.md) §6.

---

## 4. Tranche 3 asks a reviewer to confirm the backfill is still running

### The deviation

**Tranche 3 acceptance criterion 1** (design document §9) reads:

> `GET /backfill/status` shows `sdex.status: "running"`, `sdex.last_push_at`
> within the Tranche 3 push-cadence window, and `sdex.earliest_data_available`
> ≤ 2018-01-01

The same wording appears in the Tranche 3 reviewer-confirmation list. Two of its
three clauses can no longer be satisfied:

| clause                                             | state                                        |
| -------------------------------------------------- | -------------------------------------------- |
| `sdex.status: "running"`                           | **`completed`** since 2026-07-27             |
| `sdex.last_push_at` within the push-cadence window | nothing pushes any more; the value only ages |
| `sdex.earliest_data_available` ≤ 2018-01-01        | **met** — 2015-11-18, six years beyond       |

### Why

The criteria were written expecting the archive to still be ingesting through
Tranche 3. It finished during Tranche 2 instead. `status` and `last_push_at`
are progress signals for a _running_ backfill, and a finished one has neither by
definition — a completed archive that keeps pushing would be the defect.

**This is a deviation caused by delivering early, not by falling short.** The
one clause that measures the _data_ rather than the _process_ is met by six
years.

### What should replace it

The criterion's intent — "the archive is deep and the pipeline is alive" —
survives; only its instruments have to change. The depth clause stands as
written. The liveness half is better served by the signals that are actually
live post-backfill: the rollup-freshness alarms, the ledger-processor lag alarm,
and `realtime_tip_ledger` tracking the chain.

This is flagged now, in the Milestone 2 package, rather than discovered by a
reviewer at Tranche 3 — which is what §8 of the evidence document is for.

### Status

**Disclosed.** No Tranche 2 criterion depends on it. Milestone 2's own AC 5 is
graded on `earliest_data_available`, which is met and reconciled four ways
(evidence §5, AC 5).

---

## How to read this document

Each deviation above is either **defended** (we believe the delivered behaviour
is correct and better) or **disclosed** (we cannot meet the wording and say so).
None is a silent departure.

- §1 is **defended** — the string form is strictly more capable than the float
  the RFP names, and it is tested.
- §2 is **disclosed** — the criterion cannot pass as literally worded on this
  platform, so it is **amended in place and graded against a reworded
  observable**, with the evidence supplied and the weakened claim named. The
  amendment is declared here rather than negotiated in advance, which is the
  same discipline M1 was accepted on.
- §3 is **disclosed** — the named asset is excluded, with the alternative
  delivered at greater depth than asked.
- §4 is **disclosed**, and is the only entry here caused by delivering _ahead_
  of the plan rather than short of it. It affects Tranche 3, not this
  submission, and is raised now so the wording can be fixed before a reviewer
  is asked to confirm something that cannot be true.
