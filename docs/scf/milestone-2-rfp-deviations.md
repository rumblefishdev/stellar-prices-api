# Milestone 2 — deviations from the RFP, and why

Every place the delivered API departs from the literal wording of the RFP or of
the Tranche 2 acceptance criteria, with the reasoning and the evidence for each.

**This document exists because Milestone 1 was accepted on the strength of the
same discipline.** M1's package carried a section headed _"What is deliberately
not claimed"_, and that honest gap list is what made the rest of it credible.
Three of its rows became funded M2 scope. This is the M2 instance.

Nothing here is a request to lower a bar. Each entry names the deviation, says
what we did instead, and supplies the measurement to judge it on.

| #   | RFP / criterion says                                   | We deliver                              | Where     |
| --- | ------------------------------------------------------ | --------------------------------------- | --------- |
| 1   | `Current Price (float USD)`                            | a decimal **string**                    | §1, below |
| 2   | _"…return `X-Cache: Hit` header"_                      | latency evidence; no such header exists | §2        |
| 3   | USDC 1d candles verifiable against known price history | XLM and yBTC instead; USDC excluded     | §3        |

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

## 2. `X-Cache: Hit` — the header does not exist and is not being added

**Tranche 2 acceptance criterion 3** reads: _"Cache confirmed: consecutive
identical requests within TTL window return `X-Cache: Hit` header."_

The cache is real, verified, and behaving correctly. **API Gateway emits no
`X-Cache` header on any route** — it is not a setting left off; the feature does
not exist.

A header written by our own handler would be **actively wrong**, not merely
absent: API Gateway replays a cached response byte for byte and the Lambda runs
only on a miss, so the header would freeze at miss time and report `Miss` on
every genuine hit. And CloudFront — the only architecture that emits a truthful
one — writes `X-Cache: Hit from cloudfront`, not `X-Cache: Hit`, so it would not
satisfy the wording either after 3-5 days of edge work.

**Full reasoning, measurements and the proposed rewording:**
[`prices-api-cache-verification.md`](../prices-api-cache-verification.md).
**Architectural decision:**
[ADR 0012](../../lore/2-adrs/0012_api-gateway-stage-cache-no-cloudfront-no-x-cache-header.md),
accepted and team-ratified.

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

## How to read this document

Each deviation above is either **defended** (we believe the delivered behaviour
is correct and better) or **disclosed** (we cannot meet the wording and say so).
None is a silent departure.

- §1 is **defended** — the string form is strictly more capable than the float
  the RFP names, and it is tested.
- §2 is **disclosed** — the criterion cannot pass as literally worded, and a
  rewording is proposed with evidence rather than assumed.
- §3 is **disclosed** — the named asset is excluded, with the alternative
  delivered at greater depth than asked.
