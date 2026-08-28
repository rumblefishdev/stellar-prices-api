---
id: "0011"
title: "`base_currency` on the read surfaces is a DENOMINATION, not a quote-leg pair filter"
status: accepted
deciders: [okarcz, stkrolikiewicz]
related_tasks: ["0170", "0178", "0120", "0127", "0128", "0211", "0165", "0114", "0201", "0116", "0212"]
related_adrs: ["0003", "0004", "0008"]
tags: [api, read-surface, contract, usd, denomination, provenance, milestone-M2]
links:
  - "../../../packages/prices-api/src/assets/handlers.rs"
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
history:
  - date: 2026-08-25
    status: proposed
    who: okarcz
    note: >
      Raised from [[0170]] after measuring the blast radius at 20,481 assets and
      establishing that the USD-per-USDC rate is measured rather than pegged.
      Reviewed with the [[0120]] owner, who owns the conformance suite this
      changes; denomination path agreed in principle, ADR requested to settle it
      for both read surfaces at once. Carries the reconciliation against [[0201]]
      and the [[0212]] peg check inside the document rather than after it, at the
      reviewer's request.
  - date: 2026-08-25
    status: accepted
    who: stkrolikiewicz
    note: >
      ACCEPTED. Approved by the [[0120]] owner on PR #246 — they own the
      conformance suite whose assertions this inverts, so their sign-off is the
      one that matters. Both read surfaces ([[0170]] and [[0178]]) now adopt this
      rather than deciding separately.
      ⚠️ §4's open item is NOT resolved by this acceptance: reusing 0165's
      `traded`/`peg`/`oracle` values is agreed, but whether *derived O/H/L* rides
      as a separate flag or as a fourth `method` value is still open. It changes
      the response shape, so it must be settled before the implementation lands.
      What is accepted here is the denomination CONTRACT, not the serialisation
      of provenance.
  - date: 2026-08-26
    status: accepted
    who: okarcz
    note: >
      §4's open item RESOLVED — the one thing the acceptance flagged as blocking
      implementation. Derived O/H/L carries a **separate flag**; `method` keeps
      0165's traded/peg/oracle values untouched. Reasoning: the two are different
      axes and a bucket can be `traded` AND derived, so one enum cannot express
      both without making `method=derived` ambiguous about the rate's provenance.
      The field is additive, so a consumer reading only `method` is unaffected —
      but it is still a response-shape decision that [[0120]]'s suite asserts
      against, and [[0178]] inherits it. No other part of the ADR changes; the
      denomination contract accepted on 2026-08-25 stands as written.
  - date: 2026-08-26
    status: accepted
    who: okarcz
    note: >
      §2b added during [[0170]]'s implementation: the per-row conversion must
      precede the cross-leg merge. §1 implies it and nothing stated it, and the
      reversed order fails silently — `max(high)` across mixed denominations
      yields a plausible number in the wrong unit. Pinned by an integration test
      whose two possible answers differ (3.0 vs 12.0), so it discriminates.
      Also recorded from the implementation: §4's `method` is **derived at read
      time** from the quote leg and rate signature, because the candle tables
      carry `close_usd` with no companion provenance column — there is nothing to
      propagate. The pivot maps to 0165's `traded` rather than coining a fourth
      word, which leaves an XLM-quoted candle labelled `traded` resting on the
      USDC peg one hop back — [[0228]], not this ADR.
---

# ADR 0011: `base_currency` is a denomination, not a pair filter

**Related:**
- [Task 0170: `/ohlcv` returns an empty series in every mode](../1-tasks/active/0170_BUG_ohlcv-endpoint-cannot-return-usdc-self-pair.md)
- [Task 0178: `/price` cannot return USDC — same defect, harder fix](../1-tasks/backlog/0178_BUG_current-prices-cannot-publish-the-quote-asset.md)
- [Task 0211: OHLCV window boundary semantics are undocumented](../1-tasks/backlog/0211_DOCS_document-ohlcv-window-boundary-semantics.md)

---

## Context

`GET /assets/{id}/ohlcv?base_currency=USD` currently resolves `USD` to the
canonical USDC identity and filters candles on **both legs**
(`queries_ch.rs:560`):

```rust
let mut conds = vec!["asset_id = ?".to_string(), "quote_asset_id = ?".to_string()];
```

`base_currency` is therefore a **pair filter**: it selects which rows to fetch.
Two defects follow, and they are different in kind.

### Defect 1 — the parameter is unsatisfiable for most of the store

An asset that never traded against canonical USDC has no row to select, so the
endpoint returns `200 OK` with `data: []` — indistinguishable from a tracked
asset that never traded at all.

Measured on prod 2026-08-25, `price_ohlcv_1d`, 30 days:

| metric | value |
|---|---|
| assets XLM-quoted with **no** USDC leg | **20,481** |
| assets fully covered by `close_usd` | 20,449 |
| assets with no `close_usd` at all | 26 |

This is not a USDC edge case. It is the default response for twenty thousand
assets, including 5 of the 20 majors [[0120]]'s conformance suite samples.

### Defect 2 — the answer we DO give is mislabelled

USDC is not a dollar. Measured from our own data, the implied USD-per-USDC rate
(`close_usd / close` on a USDC-quoted candle) moves:

| day | rows | min rate | max rate | exactly 1.0 |
|---|---|---|---|---|
| 2026-08-25 | 663 | 1.000000 | 1.000264 | 9 |
| 2026-08-24 | 716 | 0.997608 | 1.000263 | 39 |
| 2026-08-22 | 817 | 0.996711 | 1.000187 | 18 |
| 2026-08-21 | 807 | 0.999982 | 1.000434 | 26 |

The rate **wobbles** and `exactly 1.0` is a minority of rows, so the rate is
measured, not hardcoded. Which means: every asset that *does* get an answer today
receives a **USDC-denominated price under a field labelled `USD`**, wrong by
however far the peg has drifted at that moment.

🔑 **This is what makes the decision a defect fix rather than a preference.** The
population argument (20,481) is the louder number; the peg argument is the
load-bearing one, because it applies to the assets that currently succeed.

That USDT genuinely depegged to ~$0.13 and that assuming $1 was itself the bug is
established by [[0172]] and repaired across 567,760 candles by [[0182]]. A
stablecoin's peg is a measurement, not a constant.

---

## Decision

### 1. `base_currency` denominates; it does not filter

`base_currency=USD` means **"express this asset's candles in USD"**, whatever
quote leg they were traded against. `base_currency=XLM` means the same in XLM.
The parameter no longer selects `quote_asset_id`.

This applies uniformly to **every** asset. It must not be applied per-asset —
a field whose meaning depends on data availability is the `close_usd = 0` defect
class in a new place ([[close-usd-zero-as-missing-defect-class]]).

### 2. The USD value derives from the candle's own `close_usd`

`price_ohlcv_1m` and every rolled copy already carry `close_usd`
(`init.sql:115`). The current SELECT never reads it. The per-bucket rate is:

```
rate      = close_usd / close        -- implied USD per quote unit, per bucket
open_usd  = open  × rate
high_usd  = high  × rate
low_usd   = low   × rate
close_usd = close_usd                -- as stored, not derived
vwap_usd  = vwap  × rate
```

**No `ASOF` join against `prices.usd_rate` is required for this path**, and no
dependency on [[0167]]'s coverage window. The rate table is still needed for the
degenerate cases in §6.

Column behaviour under re-denomination is **not uniform** and must not be mapped
blindly:

| column | under denomination |
|---|---|
| `open` / `high` / `low` / `close` | × rate; ordering is preserved, so `high` stays the maximum |
| `volume_base` | unchanged — base units do not move |
| `volume_quote_usd` | **already USD**, whatever the quote leg |
| `vwap` | × rate |
| `trade_count` | unchanged |

### 2b. The conversion happens BEFORE the cross-leg merge

Added 2026-08-26 during implementation. §1 implies it; nothing stated it, and
reversing it fails **silently**.

Once `base_currency` stops filtering the quote leg, one bucket can hold candles
from several legs at once — AUD against XLM and against USDC on the same day. The
merge takes `max(high)` / `min(low)` across those rows. Convert *after* merging
and that `max` compares an XLM-denominated high with a USDC-denominated one:
different units, no error, and a plausible-looking number falling out.

🔑 **Scale every row to the denomination in the inner SELECT, then aggregate.**
§1 forces it — a denomination whose meaning varies with the data available is the
`close_usd = 0` defect class in a new place — but the ordering is written down
here because the failure mode produces no signal.

Pinned by `ohlcv_converts_each_leg_before_merging_across_them`: one bucket, two
legs, rates 0.25 and 1.0. Converted first the high is `max(3.0, 1.3) = 3.0`;
converted after the merge it would be `max(12.0, 1.3) = 12.0`. The two answers
differ, so the test discriminates rather than merely passing.

### 3. O/H/L are DERIVED, and must say so

Scaling by one rate per bucket assumes the rate is constant within the bucket.
The true USD high may fall at a different instant than the quote-denominated
high. This is defensible at `1d` and weaker at `1m`.

`close` is exact (it is `close_usd` as stored). `open`, `high`, `low` and `vwap`
are derived. The response must distinguish them; [[0128]]'s evidence must state
it rather than presenting derived extremes as measured ones.

#### ✅ AMENDED 2026-08-28 — the derived extremes are CLAMPED to bracket the exact close

Task [[0229]]. Keeping `close` exact while deriving the extremes puts the two on
different numeric scales, and they can cross: measured on prod at BTC 1h,
`close` came back **below** `low` by 1.343e-11 — a malformed candle by the
`low <= open,close <= high` rule every charting library assumes.

🔑 **The mechanism is float precision, not decimal rounding**, and that rules out
the obvious alternative. Derivation runs through `toFloat64`, whose 53-bit
mantissa holds ~15-16 significant digits, while a five-figure price at
`Decimal(38, 14)` carries 19. The observed gap is **0.92 of one float64 ulp** at
that magnitude; a 14-decimal half-tick is 5e-15, ~2,700× too small to account for
it. So "round `close` to the same 14-decimal scale" **cannot** fix this — `close`
is already at 14 decimals, and that change is a no-op.

**Decision: clamp, don't re-round.** `low = min(low_derived, close)` and
`high = max(high_derived, close)`. This keeps the exactness §3 deliberately
preserves, and moves an extreme by at most the ulp the derivation had already
introduced — it cannot invent a value beyond the rounding error already present.

**`vwap` is clamped into the published `[low, high]` on the same grounds**, and on
**every** denomination — including the as-stored `base_currency=XLM` path, whose
O/H/L cannot cross but whose merged vwap can. It carries a second float
round-trip (`sum(vwap × volume) / sum(volume)`), so it escapes the band far more
readily than the extremes do: ~8.8% of single-trade candles at BTC scale in USD
mode, and 12,017-of-200,000 in each direction on the as-stored path with two
sources. A volume-weighted mean of prices within a bucket must lie within that
bucket's range, so this restates what `vwap` is rather than adjusting it.

⚠️ **Consequence a consumer can observe:** on a candle that closed at its low or
its high, `low` or `high` may now be *exactly* equal to `close` where it
previously differed in the last picoseconds of precision; and `vwap` may sit
exactly on `low` or `high` for the same reason. That is the intended outcome. `open` is unaffected and needs no clamp — it is scaled by the same rate
as the extremes, and scaling by one positive factor is monotonic, so
`low <= open <= high` holds within a row and survives `min`/`max` across rows.
Only the exact/derived boundary was ever at risk.

### 4. Provenance reuses [[0165]]'s vocabulary

`method` is propagated from the underlying `close_usd`, using the values 0165
already shipped on the series views — `traded` / `peg` / `oracle`. **No fourth
word is coined for the same concept on a third endpoint.**

✅ **SETTLED 2026-08-26 — a separate flag, not a fourth `method` value.**
The proposal below was accepted as written.

"derived O/H/L" (§3) is a *different axis* from how `close_usd` was arrived at.
`method` answers **where the USD value came from**; derivation answers **which
fields were reconstructed rather than measured**. Collapsing them into one enum
makes `method=derived` silently ambiguous about the rate's own provenance — a
bucket can be `traded` *and* derived, or `peg` *and* derived, and a single field
cannot carry both.

So: `method` keeps 0165's `traded` / `peg` / `oracle` unchanged, and the
derivation rides as its own boolean-shaped flag alongside it.

⚠️ **Consequence to carry into implementation:** this settles the response
*shape*, which [[0120]]'s conformance suite asserts against. The field is
**additive** — `method`'s existing values and meaning do not change — so a
consumer reading only `method` is unaffected. [[0178]] inherits the same
two-axis vocabulary rather than deciding again.

### 5. An unpriced bucket is returned, never silently dropped

~1.8% of the most recent buckets carry `close_usd = 0` because enrichment has not
caught up. Such a bucket is **returned with its USD fields absent**, not omitted.

This follows the contract `views.sql` already states for `close_usd`: *"a
value-or-absent: a miss is a missing row … never an error and never a dropped
row."* Dropping would put a hole at the right-hand edge of every chart and make
"not yet priced" indistinguishable from "did not trade".

### 6. Degenerate cases — the asset IS the denomination

- **`base_currency=USD` on a peg asset** (USDC/USDT at canonical issuer):
  synthesise from the measured rate in `prices.usd_rate`. ⚠️ **Not a hardcoded
  `1.0`** — our own enrichment already prices a `TF/USDC` candle at
  `close × 0.9993`, so a flat $1 would contradict our own data.
- **`base_currency=XLM` on USDC**: the pair is stored base=XLM/quote=USDC and the
  inverse is never written. Derive rather than invert. ⚠️ If inversion is chosen,
  OHLC inverts with **high↔low swapping**, `volume_base` / `volume_quote_usd` do
  **not** invert by reciprocal, and `vwap` must be recomputed, not flipped.

### 7. Two guards, on two different populations

They are not the same defect and one guard does not cover both.

**(a) Precision precondition.** Refuse to derive a rate when `close` or
`close_usd` sits within a few ticks of the `Decimal(38, 14)` floor. Observed: a
non-canonical USDC (`GC4F4IX6DV`) with `close = 5e-14`, `close_usd = 4e-14` — 5
ticks over 4 ticks, yielding an "implied rate" of exactly 0.8 that is pure
quantisation.

🔴 **A band check on the derived rate is insufficient.** It catches that row only
because 0.8 looks wrong; a row quantising to ~1.02 would pass the band and be
equally meaningless. The precondition belongs on the **inputs**, not the output.

**(b) No-reference rows are out of scope entirely.** See §Non-goals. Their
problem is not their value, so no value-based guard can catch them.

### 8. Window boundary semantics — settles [[0211]]

Both ends are **inclusive**, over bucket **start** timestamps. A window
`[start, end]` selects every bucket whose start timestamp falls in that closed
range; a request with `start == end` is a legitimate one-bucket window. `end`
binds only when the client supplies one — a derived `end = now` from the API
process's clock could, under skew, cut a bucket ClickHouse already holds.

Measured behaviour today (`queries_ch.rs:561-566`), documented here rather than
changed.

### 9. Applies to `/price` as well as `/ohlcv`

[[0178]] is the same defect on `current_prices`. It adopts this ADR rather than
deciding separately, so the two surfaces cannot diverge.

---

## Coverage — what may and may not be claimed

Raised in review: [[0201]] reports **12,981,344** `_1d` rows at the no-reference
floor, which sits badly beside a 99.9% coverage claim. Both are correct; they
count different legs. `price_ohlcv_1d` FINAL, **all history**:

| leg | era | rows | with `close_usd` | pct |
|---|---|---|---|---|
| USDC | live | 542,484 | 542,285 | 99.96% |
| USDC | pre-soroban | 235,083 | 235,065 | 99.99% |
| **XLM** | **live** | **4,842,528** | **4,837,891** | **99.90%** |
| **XLM** | **pre-soroban** | **6,320,362** | **6,199,191** | **98.08%** |
| other | live | 9,491,899 | 17,027 | **0.18%** |
| other | pre-soroban | 3,655,518 | 27,847 | **0.76%** |

Dark rows total **13,228,568**, of which **13,102,543 are exotic-quoted** —
within 1% of 0201's figure, measured a week later.

🔑 **0201's no-reference floor IS the exotic-quoted population.**

**Claimable:** for the population this ADR serves — XLM-quoted assets with no
USDC leg — coverage is **99.90% live and 98.08% pre-Soroban**.

⚠️ **Not claimable:** a flat "99.9%". The ~2% pre-Soroban shortfall lands
squarely on `timeframe=all`, which is [[0127]] AC 3, and must be stated there.

### The peg dependency, checked rather than assumed

[[0212]] reports `price_ohlcv_1m` carrying a hardcoded USDT $1 peg. If true on
this column, the denomination inherits a plausible-looking wrong number. Checked
on both tiers, USDT as the quote leg:

| tier | window | rows | min rate | max rate | exactly 1.0 |
|---|---|---|---|---|---|
| `_1d` | 12 months | ~500-650/mo | 0.127 | 0.349 | **0 every month** |
| `_1m` | 5 days | 2,449 | 0.151 | 0.174 | **0** |

The implied USD-per-USDT sits near **0.13** — the real depegged value — not $1.
So this ADR has **no dependency on 0212**.

⚠️ **Scope of that clearance:** measured where USDT/USDC is the **quote** leg,
which is the population this ADR converts through. It is **not** a verdict on
0212, which may concern USDT-as-base or a span not covered. Do not close 0212 on
this evidence.

---

## Non-goals

- **The 13.1 M no-reference rows.** Rows whose quote leg has no USD reference at
  all carry no `close_usd` and cannot be denominated by any means this ADR
  describes. They are [[0114]]'s floor, restated on the read path. `/ohlcv` and
  `/price` return nothing in USD for them, and [[0128]] must state the number
  rather than round it away.
- **[[0116]]'s absurd-large `close_usd`.** A separate defect at the opposite end
  from §7(a)'s quantisation floor. A bad `close_usd` becomes a bad multiplier for
  a whole candle under this ADR, so 0116 grows in severity — but it is not fixed
  here.
- **Changing what is stored.** This is a read-surface contract. No writer, MV or
  table changes.

---

## Alternatives Considered

### Alternative 1: keep the pair filter, return an error instead of an empty 200

**Description:** `base_currency=USD` keeps meaning "quoted in USDC"; assets with
no USDC leg get `503 quote_unavailable` instead of an empty `200`, matching the
precedent already in `handlers.rs:427-437`.

**Pros:**
- Honest about what the parameter does.
- Smallest change; no derivation, no guards, no provenance.
- Preserves "`/ohlcv` returns OHLC in the quote asset"
  ([[ohlcv-returns-quote-asset-not-usd]]) unchanged.

**Cons:**
- ⛔ **Does not fix defect 2.** It keeps labelling USDC prices as `USD`, which is
  wrong whenever the peg moves — for every asset that currently succeeds.
- The default request errors for 20,481 assets.
- [[0127]] AC 3 and AC 4 fail by design.

**Decision: REJECTED** — it addresses the empty response while preserving the
mislabelling, which is the more serious of the two defects.

### Alternative 2: fall back to the inverse pair when the direct one is missing

**Description:** when `A/USDC` has no rows, serve `USDC/A` inverted.

**Pros:**
- No new column reads; purely a query change.

**Cons:**
- ⛔ Makes the response's meaning depend on data availability — the exact defect
  class this project has been repeatedly bitten by.
- Only helps where the inverse pair exists; most of the 20,481 have neither
  orientation against USDC.
- The inversion arithmetic is subtle and easy to get wrong (high↔low, volumes,
  vwap).

**Decision: REJECTED** — already warned against in 0170's own task file.

### Alternative 3: `ASOF` join against `prices.usd_rate` per bucket

**Description:** ignore `close_usd`; join every bucket to the measured rate table
and convert.

**Pros:**
- One rate source for every surface.
- Independent of enrichment's reach.

**Cons:**
- Redundant — `close_usd` is already on the row at 99.90%/98.08%, and it is what
  enrichment derived *from* the rate table.
- Adds a dependency on [[0167]]'s coverage window, which starts 2026-03-11 —
  useless for `timeframe=all` back to 2022.
- More expensive per query, on the hot read path.

**Decision: REJECTED for the main path**, retained for §6's degenerate cases
where there is no candle to derive from.

---

## Consequences

### Positive

- 20,481 assets stop returning an empty `200` on the default request.
- The `USD` label becomes true, and stays true when the peg moves.
- [[0127]] AC 3 and AC 4 become reachable; [[0120]]'s five flagged majors flip
  from empty to populated.
- `/ohlcv` and `/price` are decided together, so the two surfaces cannot diverge.
- [[0211]] settles for free.

### Negative

- **A bad `close_usd` becomes a bad whole candle.** Today it corrupts one column
  `/ohlcv` does not even return; under this ADR it multiplies O/H/L/vwap. The fix
  amplifies [[0116]] rather than causing it, but the visible blast radius of one
  bad row grows from a field to a candle. §7(a) guards the quantisation end only.
- **O/H/L are no longer measured values.** A real change in what the endpoint
  means, requiring the §3 disclosure and an update to
  [[ohlcv-returns-quote-asset-not-usd]].
- **[[0120]]'s suite assertions invert** for the affected assets — coordinated,
  not incidental.
- **The exotic-quoted population becomes visibly unserved.** It is unserved today
  too, but an empty `200` hid it; naming it is honest and slightly worse-looking.

---

## References

- `packages/prices-api/src/assets/queries_ch.rs:557-608` — the query this changes
- `packages/prices-api/src/assets/handlers.rs:408-456` — leg resolution
- `packages/prices-clickhouse/schema/init.sql:103-123` — `close_usd` on the candle
- `packages/prices-clickhouse/schema/views.sql:235-263` — the value-or-absent
  contract and 0165's `method` vocabulary
